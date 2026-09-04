// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The network datapath, end to end: a second driver outside the frame, a client
//! with a registered buffer, a real device, a real host network backend, and one
//! frame that goes out and comes back into memory the driver never touched.
//!
//! # What this file is, and the one thing it is evidence of
//!
//! It is the **supervisor's half** of E1-B03. `user/virtio-net` is the driver:
//! the transport handshake, two virtqueues, the registration table and the
//! service loop, in a crate that forbids `unsafe` and holds no mapping of any
//! client's memory. What is here is everything a supervisor does around one —
//! find the device, program its domain, route it four register windows and one
//! untyped region, stand a client up on the other end of a ring, and judge what
//! happened.
//!
//! It is not a second implementation of the driver, and it is not a second
//! implementation of `kernel/src/blk.rs` either — though it looks like one, and
//! that resemblance is the finding this task exists to produce.
//! `docs/rfc/0051-a-second-driver-is-what-says-the-shape-is-a-shape.md` counts
//! the resemblance exactly: what a second driver could reuse unchanged, what it
//! had to write again, and what it found that one driver could not.
//!
//! **This file is the largest single entry on the *written again* list.**
//! [`Registers`], [`Supervising`], [`Reported`], [`declared`] and `order_for`
//! are `kernel/src/blk.rs`'s, adapted only in which manifest name they look for
//! and which counters they read. That duplication is deliberate and it is
//! bounded — the same trade `kernel/src/arch/x86_64/virtio.rs` made about
//! `dma.rs`'s duplicated capability walk, and for a sharper reason: `blk.rs` is
//! the evidence a closed task's exit rests on, and refactoring it to share a
//! supervisor with a later task would change closed evidence for the
//! convenience of the later task. *What would merge them is a third driver*,
//! which is E1-B04, at which point the shared half moves out of both and neither
//! of them is closed evidence any more.
//!
//! # Three halves, and the middle one is the control
//!
//! `net=inside` registers the client's page, posts one receive buffer, forms an
//! ARP request for the gateway by hand, transmits it, and requires an ARP
//! **reply addressed to the MAC this boot invented** to land in the registered
//! buffer. That last clause is what makes it a demonstration rather than a
//! coincidence: the host's user-mode network backend answers ARP for its own
//! address, and it cannot have produced a reply carrying a target hardware
//! address it was never told.
//!
//! `net=silent` is the identical client with the transmit removed. The receive
//! buffer is posted and nothing is sent, and **nothing may land**. Without it
//! `inside` establishes only that a frame arrived, not that this driver's
//! transmit caused it — and a link with an unsolicited broadcast on it would
//! make the first half pass on its own. It is also the only half that exercises
//! the teardown obligation the receive direction creates: a buffer posted and
//! never filled has to be given back as a cancellation, and
//! `f_virtio_net::driver::Driver::cancel` argues why RFC 0024 leaves a client no
//! other exit.
//!
//! `net=escape` transmits exactly as `inside` does and has the driver add
//! [`BEYOND`] to the address the registration answered before it becomes a
//! **receive** descriptor. Nothing is taken away from the driver; it points the
//! device at memory it never held, and the device's next act would be to *write*
//! there. The remapping unit must fault it, nothing may land, and the client's
//! buffer must still hold its poison.
//!
//! The block driver's `escape` and this one are the same provocation in
//! different directions and the difference is worth the second boot: there, an
//! unrefused escape is a device *reading* memory it was not granted, bounded by
//! a request that was outstanding. Here it is a device *writing* into memory it
//! was not granted, at a moment nothing in this system chose, for as long as the
//! buffer stays posted.
//!
//! # What the demonstration does not show
//!
//! It uses one host backend, one frame, one protocol and no stack. It says
//! nothing about throughput, nothing about many buffers in flight, nothing about
//! a frame larger than the one it sends, and nothing about what happens when the
//! link is busy — `f_virtio_net::driver::Counters::spun` is published so that
//! *how long this waited* is a number rather than an impression, and it is the
//! argument for E1-B09 rather than a measurement of anything.
//!
//! It also cannot show that a transmit was **delivered**. virtio-net's transmit
//! queue publishes a used entry with no status anywhere, so a frame the link
//! dropped and a frame delivered intact are the same completion — `sim/src/net.rs`
//! models exactly that silence. What stands in for delivery here is the reply:
//! the only evidence this boot has that the frame left the machine is that
//! something outside it answered.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, rights};
use f_abi::control;
use f_abi::manifest::{self, ContentId, Record, route};
use f_abi::{ABI_VERSION, Negotiated, class, error, feature};
use f_ring::device::Window;
use f_ring::registry::{Domains, registration};
use f_ring::{BufferSet, Collector, Consumer, Fixed, Mapping, Poster, Producer};
use f_virtio_net::driver;
use f_virtio_net::routing;

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging::{self, AddressSpace, Features};
use crate::arch::x86_64::pci::{self, Bdf, Survey};
use crate::arch::x86_64::virtio;
use crate::arch::x86_64::vtd::{Fault, Unit};
use crate::cap::Table;
use crate::component;
use crate::iommu;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};

/// The one address a driver component holds as a constant, agreed — for the
/// second time, in a second crate.
///
/// `f_virtio_net::routing::AT` is written down in the component and
/// `kernel::process::BLK_BOARD` in the frame, and they are linked separately.
/// The kernel is the one artefact that links both definitions, so the agreement
/// is a check rather than a comment.
///
/// **That there are now two of these assertions is the point of the file they
/// are in.** The frame builds one driver shape, so every driver scheduled into
/// it finds its board at the same address, and each driver crate holds that
/// address as its own constant with no way to see the others. Two crates, two
/// assertions, and a third driver adds a third. RFC 0051 says where the constant
/// should live instead and why moving it is not this task's to do.
const _: () = assert!(
    crate::process::BLK_BOARD == routing::AT,
    "the frame and the network driver disagree about where the routing page is"
);

/// The two drivers' routing pages are told apart by their magic and by nothing
/// else.
///
/// They are mapped at the same address, in the same shape, by the same loader.
/// If the two magics were equal, a build that routed one driver's supervisor at
/// the other driver's image would find a page whose magic matched and whose
/// fields meant something else — a driver reading another driver's board as its
/// own, with no refusal anywhere. The assertion is here rather than in either
/// component because this is the only place both constants exist at once.
const _: () = assert!(
    routing::MAGIC != f_virtio_blk::routing::MAGIC,
    "the two drivers' routing pages are mapped at one address and must not answer to one magic"
);

/// Entries on the client's data ring.
///
/// Sixteen, which is what fits one frame beside its completion ring and its
/// index ring — the same number and the same reason as a control ring's. The
/// manifest declares two hundred and fifty-six, which is what a real client gets
/// when a component pays for its own channel; this demonstration submits four
/// entries in total.
const ENTRIES: u32 = 16;

/// Buffers the client's registered set holds.
///
/// Two: one to receive into and one to transmit from. They have to be different
/// buffers and the reason is sharper here than on the block driver — the device
/// is *writing* one of them while the client is filling the other, so a single
/// buffer would be a client and a network card racing over the same bytes with
/// nothing between them.
const BUFFERS: u32 = 2;

/// Which buffer of the set the device writes into. Unit: buffers, zero-based.
///
/// Zero, so that the address a registration answers for it is the base of the
/// client's page — which makes [`Report::expected_fault`] the registration's own
/// answer plus [`BEYOND`], with no per-buffer arithmetic between the boot log
/// and the unit's fault record.
const SINK: usize = 0;

/// Which buffer the client transmits from. Unit: buffers, zero-based.
const SOURCE: usize = 1;

/// The byte the sink is filled with before anything is posted.
///
/// Not a byte an ARP reply contains, so a sink still full of it is a sink
/// nothing wrote. The same trick `dma.rs` uses and for the same reason: *the
/// transfer was refused* and *the transfer happened and wrote nothing* are
/// different claims, and only one of them is an exit criterion.
const POISON: u8 = 0xA5;

/// The interface address this boot invents.
///
/// Invented rather than read out of the device, because `VIRTIO_NET_F_MAC` is
/// not negotiated — `f_virtio_net::transport` says why — and because an invented
/// address is *better evidence*. The host's network backend cannot produce a
/// reply carrying a target hardware address it was never told, so a reply
/// carrying this one is a reply to the request this boot sent.
///
/// The first three bytes are the prefix QEMU uses for its own guests, which
/// makes a frame captured on the host recognisable as this machine's.
const OUR_MAC: [u8; 6] = [0x52, 0x54, 0x00, 0xF0, 0x0D, 0x01];

/// The protocol address this boot claims.
///
/// The address the host's user-mode network backend assigns a guest, which is
/// what makes the backend willing to answer at all. It is not configured
/// anywhere on this machine and nothing here has an IP stack: it is four bytes
/// in a frame this file forms by hand.
const OUR_IP: [u8; 4] = [10, 0, 2, 15];

/// The protocol address the request asks about: the backend's own gateway.
const GATEWAY_IP: [u8; 4] = [10, 0, 2, 2];

/// Ethernet's type for address resolution.
const ETHERTYPE_ARP: u16 = 0x0806;

/// An address-resolution request.
const ARP_REQUEST: u16 = 1;

/// An address-resolution reply.
const ARP_REPLY: u16 = 2;

/// Bytes in the frame this boot forms: an Ethernet header and an ARP body over
/// Ethernet and IPv4. Unit: bytes.
const FRAME_BYTES: u32 = 42;

/// Where each field of that frame is. Unit: bytes from the frame's first.
mod at {
    /// The destination hardware address.
    pub const DESTINATION: usize = 0;
    /// The source hardware address.
    pub const SOURCE: usize = 6;
    /// What the payload is.
    pub const ETHERTYPE: usize = 12;
    /// Which kind of hardware address the body names.
    pub const HARDWARE: usize = 14;
    /// Which kind of protocol address.
    pub const PROTOCOL: usize = 16;
    /// How long a hardware address is.
    pub const HARDWARE_LEN: usize = 18;
    /// How long a protocol address is.
    pub const PROTOCOL_LEN: usize = 19;
    /// Request or reply.
    pub const OPERATION: usize = 20;
    /// The sender's hardware address.
    pub const SENDER_MAC: usize = 22;
    /// The sender's protocol address.
    pub const SENDER_IP: usize = 28;
    /// The target's hardware address.
    pub const TARGET_MAC: usize = 32;
    /// The target's protocol address.
    pub const TARGET_IP: usize = 38;
}

/// How many turns the driver's loop may spend with nothing arriving.
///
/// Told to the component rather than chosen by it, because how long to wait for
/// a network is a property of the machine and its backend, which the frame knows
/// and a component cannot. It is a **backstop and not the mechanism**: on every
/// half of this demonstration the frame's own client gives up first and posts a
/// stop, so a run that reached this number is a run where the frame stopped
/// serving — which is a different failure and wants a different answer.
///
/// A count and not a duration, because RFC 0004 offers a component no clock and
/// because a count is the same number on every host, which is what keeps
/// `cargo xtask trace`'s fixture a fixture.
///
/// A hundred million, and the size is the argument. It has to be **larger than
/// anything the frame's own bounds allow**, or it stops being a backstop and
/// becomes the mechanism: the `silent` half was measured spinning one million
/// nine hundred thousand turns inside [`RECEIVE_MICROS`], so a bound anywhere
/// near that would fire before the frame's stop and the control would be
/// measuring this constant rather than an empty link. What catches a driver that
/// is genuinely stuck is [`EXIT_MICROS`] and the harness's boot timeout, both of
/// which are the frame's and neither of which a component can outlast.
///
/// Unit: turns.
const RECEIVE_SPINS: u64 = 100_000_000;

/// The deadline the client's transmit carries.
/// Unit: nanoseconds, monotonic, in the channel's epoch.
///
/// A millisecond, which is outside [`FLOOR_NS`] by two orders of magnitude, so
/// this request is *not* floored. That matters because `cflags::SHORTFALL` is
/// **one bit**: a completion says the request got less than it asked for and not
/// which of the three ways, so the frame cannot tell a class demotion from a
/// floored deadline. This constant is what leaves only one of them possible, and
/// it is an argument rather than a check — the check would need a field on the
/// completion, which is an ABI change under RFC 0011. The same constant and the
/// same argument as `kernel/src/blk.rs`'s, because it is the same one bit.
const HARD_DEADLINE_NS: u64 = 1_000_000;

/// The floor the driver is told it needs. Unit: nanoseconds.
///
/// Ten microseconds, the same figure the block datapath routes and for the same
/// reason: what it bounds on this boot is nothing, because a component has no
/// clock and the arrival it floors from is zero. It is routed anyway, because a
/// frame that left the field at zero would be telling the driver it needs no
/// time at all — a different claim from *this driver cannot measure the
/// difference*.
const FLOOR_NS: u64 = 10_000;

/// The manifest this datapath routes for, by name.
///
/// A name and not an index, because the loader's module order is a contract
/// about `user/init` and about nothing else. The bytes are what `manifest.toml`'s
/// `name` compiles to.
const DRIVER: &[u8] = b"virtio-net";

/// The need in that manifest that names the register pages.
const NEED_MMIO: &[u8] = b"mmio";

/// The need that names the untyped region the driver splits into its queues.
const NEED_QUEUES: &[u8] = b"queues";

/// The rights a component holds over memory it means to hand to a device.
///
/// `GRANT` is the load-bearing one and [`iommu::Grant::map`] argues why: putting
/// a page in a device's domain is a transfer to something the capability system
/// does not mediate. `WRITE` because a receive is the device writing — which on
/// this driver is not an incidental half of the story but the whole clause the
/// task turns on.
const GRANTABLE: u8 = rights::READ | rights::WRITE | rights::GRANT;

/// How far past a registration's answer the `escape` half points the device.
///
/// One frame, so the address lands in the page *after* the one the client
/// registered — outside the driver's domain by a whole page rather than by a
/// byte, because an address that straddles the end of a grant is a second
/// question and this run is asking the first. Unit: bytes.
pub const BEYOND: u64 = FRAME_SIZE;

/// How long the frame waits for one completion from a driver it is a client of.
/// Unit: microseconds.
const ANSWER_MICROS: u64 = 5_000_000;

/// How long it waits for a frame that may never come. Unit: microseconds.
///
/// The same five seconds, and it is used in two opposite ways: on `inside` a
/// bound that passes is a failure, and on `silent` and `escape` a bound that
/// passes is the result. One number for both, because a control that waited less
/// than the experiment would be a control that proves only that it was more
/// impatient.
const RECEIVE_MICROS: u64 = 5_000_000;

/// How long it waits for the core afterwards. Unit: microseconds.
const EXIT_MICROS: u64 = 5_000_000;

/// Why the demonstration could not be run.
///
/// None of these is the result being looked for. A datapath that could not be
/// set up is not a datapath that was exercised, and the boot path says so rather
/// than reporting a pass.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// The device could not be found or routed. The finder's own reason.
    Device(virtio::Trouble),
    /// The remapping unit refused something. Its own reason, unchanged.
    Unit(crate::arch::x86_64::vtd::Refuse),
    /// The allocator had no block for the driver's region, the client's page or
    /// the channel.
    NoFrames,
    /// A capability table this builds could not hold the handles the grant is
    /// made of. A bug here rather than a machine property.
    Authority,
    /// The frame refused a translation for memory the holder does hold a
    /// grantable capability for.
    Refused,
    /// The frame *gave* a translation for a capability carrying no right to hand
    /// memory to a device.
    ///
    /// The one variant that is a failure of the thing being tested rather than
    /// of the test.
    NotRefused,
    /// A channel could not be laid out, or one of its four ends could not bind.
    Channel(i32),
    /// The client's region does not divide into the buffers the registration
    /// declared, which is this file's arithmetic and not a peer's.
    Geometry,
    /// A registration was refused. Carries the packed refusal.
    Registration(i32),
    /// A transmit or a receive was refused. Carries the packed refusal.
    Transfer(i32),
    /// The frame's own count of what it took and what it gave back disagreed.
    Leaked,
    /// No component file among the boot modules declares this driver.
    NoManifest,
    /// The manifest and the machine disagree: a need this datapath has to route
    /// is missing from the record, declares less than the driver's own layout
    /// needs, or declares fewer register pages than the device describes.
    Manifest,
    /// The driver could not be built as a process, carrying which step.
    Process(crate::process::Error),
    /// The core the driver was given never took it, or never gave it back.
    Scheduled(usize),
    /// The driver did not answer a completion inside the frame's own bound.
    /// Carries that bound. Unit: microseconds.
    NoAnswer(u64),
    /// The driver's core was still holding its job when the frame's bound
    /// passed. Carries that bound. Unit: microseconds.
    Overdue(u64),
    /// A frame was required and none arrived inside the frame's own bound.
    /// Carries that bound. Unit: microseconds.
    ///
    /// **A bound and not a finding**, which is the whole reason it is a variant
    /// rather than a `landed` left false. [`RECEIVE_MICROS`] is read two
    /// opposite ways on this demonstration — on `silent` and `escape` a bound
    /// that passes is the *result*, and on `inside` it is a failure — and
    /// review found the diagnosis had only been split one way: a slow runner and
    /// a broken receive path both printed *nothing came back on the receive
    /// queue*, under the same heading as every failure the frame actually
    /// observed. This is what puts a receive that ran out of time under the same
    /// paragraph as every other wall-clock bound in this file, which says
    /// plainly that a red here is a wedged component or a machine slower than
    /// the number.
    NoFrame(u64),
}

impl Trouble {
    /// A sentence for the boot log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Device(why) => why.message(),
            Self::Unit(why) => why.message(),
            Self::NoFrames => "no frames for the driver's region, the client's page or the channel",
            Self::Authority => "the demonstration could not mint the capabilities it is made of",
            Self::Refused => "the frame refused a translation for memory its holder may grant",
            Self::NotRefused => {
                "the frame gave a device translation for a capability carrying no right to grant"
            }
            Self::Channel(_) => "the client's data ring could not be laid out or bound",
            Self::Geometry => "the client's region does not divide into the buffers declared",
            Self::Registration(_) => "the driver refused to register the client's buffer set",
            Self::Transfer(_) => "the driver refused an operation that was supposed to be served",
            Self::Leaked => "the demonstration's frames did not all come back",
            Self::NoManifest => "no boot module declares the virtio-net component",
            Self::Manifest => {
                "the driver's manifest and this machine disagree about what has to be routed"
            }
            Self::Process(_) => "the driver could not be built as a process",
            Self::Scheduled(_) => {
                "the core the driver was given never took it or never gave it back"
            }
            Self::NoAnswer(_) => "the driver did not answer a completion inside the bound",
            Self::Overdue(_) => "the driver's core did not report finished inside the bound",
            Self::NoFrame(_) => "no frame reached the client's buffer inside the bound",
        }
    }

    /// The wall-clock bound this refusal is, when it is one.
    ///
    /// Three of these variants are not findings: they are spins that ran out of
    /// a number derived from `tsc_khz`, so they fire for a component that is
    /// wedged and for a runner slower than the number alike, and nothing here
    /// can tell those apart. Printing them under the same sentence as the rest
    /// is how a slow CI machine comes to be read as a datapath defect.
    /// Unit: microseconds.
    #[must_use]
    pub const fn bound(self) -> Option<u64> {
        match self {
            Self::NoAnswer(micros) | Self::Overdue(micros) | Self::NoFrame(micros) => Some(micros),
            _ => None,
        }
    }
}

/// What the driver's manifest says it must be given.
///
/// Read out of the record `cargo xtask component` compiled, on every run, rather
/// than repeated as constants here. That is the whole point of the detour:
/// `user/virtio-net/manifest.toml` was written before the driver *and before this
/// file*, and a datapath that routed numbers of its own choosing would leave the
/// manifest as decoration.
#[derive(Clone, Copy, Debug)]
pub struct Declared {
    /// The content hash a spawn would name: one hash over the record and the
    /// image together. Unit: none — an identity.
    pub id: ContentId,
    /// Register pages the manifest routes. Unit: pages.
    pub frames: u32,
    /// Untyped bytes it routes for the queues. Unit: bytes.
    pub bytes: u64,
    /// The reservation class the manifest declares, as `f_abi::class` reads it —
    /// the ceiling this component is admitted for. Read out of the record like
    /// everything else here and never written down in the frame, for the reason
    /// RFC 0025 bound 2 gives about ceilings. Unit: none — a class ordinal.
    pub admitted: u16,
    /// The component's own image, out of the same component file.
    pub image: &'static [u8],
}

/// Find the driver's component file and read what it declares.
///
/// # Errors
///
/// [`Trouble::NoManifest`] when no module carries it, [`Trouble::Manifest`] when
/// the record is not one this build can read or does not declare both needs this
/// datapath routes.
///
/// # Safety
///
/// The direct map must be live and `frames` must already have been rebound onto
/// it, which is `component::modules`' obligation.
pub unsafe fn declared(boot: &BootInfo) -> Result<Declared, Trouble> {
    // SAFETY: the caller's guarantee, passed down.
    let (modules, count) = unsafe { component::modules(boot) };
    for module in modules.iter().take(count) {
        let Ok(record) = Record::read(module) else { continue };
        if record.label() != DRIVER {
            continue;
        }
        let mut frames = None;
        let mut bytes = None;
        for need in record.needs() {
            // An ask is not routed at spawn, so a need routed through the
            // powerbox is not one this datapath supplies.
            if need.route == route::POWERBOX {
                continue;
            }
            match need.label() {
                NEED_MMIO => frames = Some(need.frames),
                NEED_QUEUES => bytes = Some(need.bytes),
                _ => {}
            }
        }
        let (Some(frames), Some(bytes)) = (frames, bytes) else { return Err(Trouble::Manifest) };
        let Ok(image) = record.image(module) else { return Err(Trouble::Manifest) };
        // R04 at a byte the frame did not write: a class this build cannot name
        // is a record from a schema this build cannot read.
        let Some(admitted) = manifest::class::admitted(record.class) else {
            return Err(Trouble::Manifest);
        };
        return Ok(Declared { id: ContentId::of(module), frames, bytes, image, admitted });
    }
    Err(Trouble::NoManifest)
}

/// Which of the three experiments a run is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    /// A receive buffer is posted, an ARP request is transmitted, and the reply
    /// must land in the registered buffer. The positive control, without which
    /// neither result below proves anything.
    Inside,
    /// The identical client with the transmit removed. Nothing may land, and the
    /// posted buffer must come back as a cancellation.
    Silent,
    /// The transmit happens and the driver points the device past what the
    /// registration answered before the address becomes a receive descriptor.
    /// The unit must fault it and nothing may land.
    Escape,
}

impl Half {
    /// The word the boot log and the harness's parameter share.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inside => "inside",
            Self::Silent => "silent",
            Self::Escape => "escape",
        }
    }

    /// Does this half put a frame on the link?
    #[must_use]
    pub const fn transmits(self) -> bool {
        matches!(self, Self::Inside | Self::Escape)
    }

    /// Must a frame come back?
    ///
    /// True on exactly one half, and that is the design. The other two are the
    /// two different ways a frame can fail to arrive — nothing was sent, and
    /// something was sent and the unit refused where it was to be written — and
    /// a suite that could not tell those apart would be claiming one of them
    /// while showing the other.
    #[must_use]
    pub const fn expects_frame(self) -> bool {
        matches!(self, Self::Inside)
    }

    /// How far past the registration's answer this half points the device.
    /// Unit: bytes.
    #[must_use]
    pub const fn beyond(self) -> u64 {
        match self {
            Self::Escape => BEYOND,
            _ => 0,
        }
    }

    /// Which of the component's lives this half asks for.
    ///
    /// Two and not three: `inside` and `silent` differ in what the *client*
    /// does and not in what the driver does at all, which is what makes the
    /// second a control for the first. Unit: none — a selector ordinal.
    #[must_use]
    pub const fn selector(self) -> u32 {
        match self {
            Self::Escape => routing::life::ESCAPE,
            _ => routing::life::SERVE,
        }
    }
}

/// What one run of the datapath did.
#[derive(Clone, Copy, Debug)]
pub struct Report {
    /// Which experiment this was.
    pub half: Half,
    /// What the driver's own manifest declared, and what this run routed
    /// against.
    pub declared: Declared,
    /// Which function was driven.
    pub bdf: Bdf,
    /// How many pages of the device window the four register structures took.
    /// Unit: pages.
    pub windows: u32,
    /// Where the device addresses the client's registered set.
    /// Unit: bytes, in the device's address space.
    pub registered_at: u64,
    /// Whether a registration of a capability carrying no `GRANT` was refused.
    pub refused_without_grant: bool,
    /// Whether the transmit completed without a refusal.
    ///
    /// **Not evidence of delivery.** virtio-net's transmit queue answers with no
    /// status, so this says the device took the frame and nothing more. What
    /// stands in for delivery on this boot is [`Report::landed`].
    pub transmitted: bool,
    /// Whether a receive completion arrived carrying a frame.
    pub landed: bool,
    /// How many bytes of frame that completion reported. Unit: bytes.
    pub frame_bytes: u32,
    /// Whether what landed is an ARP reply from the gateway addressed to
    /// [`OUR_MAC`].
    ///
    /// Read out of the client's own buffer rather than inferred from a
    /// completion, because a completion is evidence the device finished and
    /// never evidence that bytes moved — and because the target hardware address
    /// is the field the backend could not have produced without the request this
    /// boot sent.
    pub matched: bool,
    /// How many bytes of the sink still hold [`POISON`], over the length of the
    /// frame this boot formed. Unit: bytes.
    ///
    /// Published rather than reduced to a boolean because *nothing landed* and
    /// *some of it landed* are different failures and only one of them is the
    /// expected result of the two refused halves.
    pub untouched: u32,
    /// What the driver counted, read out of the board rather than out of a
    /// structure in this address space. Unit: see [`driver::Counters`].
    pub counters: driver::Counters,
    /// Which core the driver held. Unit: none — a core index.
    pub cpu: usize,
    /// Whether it ended by `EXIT` rather than by a fault.
    ///
    /// The frame's own reading, taken from `process::reap`: a driver that
    /// faulted mid-run could write nothing afterwards, and one that scribbled
    /// its own board could write anything.
    pub exited: bool,
    /// How many entries it took off its data ring. Unit: entries.
    pub drained: u64,
    /// How many operations the driver submitted on its control ring and this
    /// frame answered.
    ///
    /// Counted by the frame and not reported by the component, which is what
    /// makes it evidence: it is the one number here a component could not
    /// produce if the route it names had never been used. Unit: operations.
    pub asked: u32,
    /// Why its loop ended, as one of `f_virtio_net::routing::stopped`. Zero for
    /// a component that never wrote a report at all. Unit: none — an ordinal.
    pub stopped: u64,
    /// The first fault the remapping unit recorded, if it recorded one.
    pub fault: Option<Fault>,
    /// How many it recorded. Unit: transactions.
    pub faults: u32,
}

impl Report {
    /// Whether the run produced what the half it was asked for requires.
    ///
    /// The verdict is the kernel's rather than the harness's, exactly as `user`,
    /// `cap`, `iommu` and `blk` already are: this knows which half it was asked
    /// for, what the unit recorded and what is in the client's buffer
    /// afterwards, and a harness reading an exit code could not tell a refused
    /// transfer from a link with nothing on it.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold. Every one of them fails the boot: a
    /// protection that did not fire is not a smaller result than a fault, it is
    /// the opposite result.
    pub const fn verdict(&self) -> Result<(), &'static str> {
        // The component ran and ended the way a component ends, checked before
        // anything it said about itself. A driver that faulted before its loop
        // started would write nothing into its board, and every counter below
        // would read zero — including `copies`, which is the number this
        // subsystem publishes as a property.
        if !self.exited {
            return Err("the driver did not end by EXIT");
        }
        if self.stopped != routing::stopped::TOLD {
            return Err("the driver's loop did not end on the frame's stop");
        }
        if !self.refused_without_grant {
            return Err("a registration with no right to grant was not refused");
        }
        if self.counters.copies != 0 {
            return Err("the driver copied bytes on the data path");
        }
        // The zero above means something only because this is not zero: both
        // tallies pass through one function, so a build where that function
        // stopped counting would publish two zeroes and pass the line above.
        if self.counters.provoked == 0 {
            return Err("the driver's own copy self-check moved nothing");
        }
        // The route RFC 0047 is about, counted on the frame's side. A driver
        // that had stopped asking would be a driver whose translations came from
        // somewhere else.
        if self.asked == 0 {
            return Err("the driver asked the frame for no translation");
        }
        // Every half posts exactly one receive buffer, and it is the same one.
        //
        // What that leaves untested is the multi-slot machinery: the head the
        // device gives back is the only thing that says which of four posted
        // buffers filled, and no boot in this tree reaches slot one. It is
        // covered by `f_virtio_net::driver`'s unit tests against a
        // memory-backed device, which go red under both of the formulas that
        // break it, and the taxonomy row for this verb says so in place of a
        // boot.
        if self.counters.posted != 1 {
            return Err("the driver did not post exactly one receive buffer");
        }
        // Zero on every half of this demonstration, and it is a *property*
        // rather than a tally of something that happens: a transfer that failed
        // after its buffer was already with the device has no refusal available
        // to it, because a refusal hands the client back a buffer a network card
        // still holds a write descriptor into. The driver's answer is to reset
        // the device and end, which would also make `stopped` above disagree —
        // so this is the number that says *which* of the two happened, and a
        // boot where it moved is a boot where the receive path took the exit
        // that exists for a failure and not the one that exists for a client.
        if self.counters.halted != 0 {
            return Err("a transfer failed after its buffer was already with the device");
        }
        // R08, on this driver as on the other. The client's transmit is the one
        // entry here that asks for the hard class, and this driver's manifest
        // declares the soft one — so its completion must say it was served below
        // what it asked. A zero would mean the demotion happened and nobody was
        // told, which is the one outcome R08 refuses.
        //
        // Only on the halves that transmit, because only they submit that entry:
        // every other entry in this demonstration carries `Sqe::ZERO`'s class,
        // which is batch, and a batch request at a soft-class service is not
        // demoted at all. Requiring a shortfall from the control half would be
        // requiring a demotion nothing asked for.
        if self.half.transmits() && self.counters.shortfall == 0 {
            return Err("no completion reported the demotion the manifest requires");
        }
        if !self.half.transmits() && self.counters.shortfall != 0 {
            return Err("a completion reported a demotion on a half that asked for nothing");
        }

        match self.half {
            Half::Inside => {
                if !self.transmitted {
                    return Err("the transmit did not complete");
                }
                // Second, and it is the client that reports this first: an
                // empty sink on this half comes back as `Trouble::NoFrame`,
                // which is printed as the wall-clock bound it is. Kept here
                // because a `Report` is the kernel's verdict on its own and a
                // verdict that trusted its caller to have checked would be a
                // verdict with a hole in it.
                if !self.landed {
                    return Err("nothing came back on the receive queue");
                }
                if !self.matched {
                    return Err("what came back is not a reply to the frame this boot sent");
                }
                if self.counters.received != 1 {
                    return Err("the driver did not report exactly one frame received");
                }
                if self.counters.escaped != 0 {
                    return Err("the driver pointed the device past a registration's answer");
                }
                if self.faults != 0 {
                    return Err("the remapping unit faulted on the half that must not fault");
                }
                // The buffer came back through its completion rather than
                // through a cancellation, which is the difference between a
                // receive that was answered and one that was abandoned.
                if self.counters.cancelled != 0 {
                    return Err("a receive that was answered was also cancelled");
                }
                Ok(())
            }
            Half::Silent => {
                if self.transmitted {
                    return Err("the control half put a frame on the link");
                }
                if self.counters.sent != 0 {
                    return Err("the control half transmitted");
                }
                if self.landed || self.counters.received != 0 {
                    return Err("something arrived on a link this boot put nothing on");
                }
                if self.untouched != FRAME_BYTES {
                    return Err("the receive buffer was written on the half that sent nothing");
                }
                if self.faults != 0 {
                    return Err("the remapping unit faulted with nothing on the link");
                }
                // The obligation the receive direction creates, observed rather
                // than assumed: a posted buffer no frame ever filled has to come
                // back as a cancellation, because RFC 0024 leaves its holder no
                // other exit.
                if self.counters.cancelled != 1 {
                    return Err("the posted receive buffer was not given back as a cancellation");
                }
                Ok(())
            }
            Half::Escape => {
                if !self.transmitted {
                    return Err("the escape half did not put a frame on the link");
                }
                // The provocation ran. An isolation proof whose provocation
                // never ran is the same green as a protection that held.
                if self.counters.escaped == 0 {
                    return Err("the driver never pointed the device past what it was answered");
                }
                // **The client's own memory, and this is the check.** Nothing
                // about a completion is asserted here, and the first version of
                // this arm asserted the wrong thing: it required the receive
                // queue to publish nothing, and this emulator publishes a used
                // entry with a length for a transfer the remapping unit refused.
                // `kernel/src/arch/x86_64/dma.rs` recorded exactly that about the
                // block device — *a completion is evidence the device finished
                // and never evidence that bytes moved* — and a second driver
                // found it again from the other direction. A driver that
                // believed the used ring here would hand its client a length and
                // the client would read poison.
                if self.untouched != FRAME_BYTES {
                    return Err("the receive buffer was written through a refused translation");
                }
                if self.matched {
                    return Err("a reply reached the client through a descriptor the unit refused");
                }
                // And the unit's own fault-recording registers, which are the one
                // piece of evidence on this boot that neither the component nor
                // the device wrote. Checked *at the address the driver invented*
                // rather than merely counted: a fault somewhere else would mean
                // something other than this provocation was refused, and the
                // count alone cannot tell those apart.
                match self.fault {
                    None => {
                        return Err("the remapping unit recorded no fault for the refused write");
                    }
                    Some(fault) => {
                        if fault.address != self.expected_fault() {
                            return Err(
                                "the unit faulted somewhere other than the address the driver \
                                 invented",
                            );
                        }
                        if fault.read {
                            return Err(
                                "the unit faulted on a read, so the descriptor under test was \
                                 not the one the device writes",
                            );
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// Where this half's refused transaction must have faulted.
    /// Unit: bytes, in the device's address space.
    #[must_use]
    pub const fn expected_fault(&self) -> u64 {
        self.registered_at.wrapping_add(self.half.beyond())
    }
}

/// What the frame stands a scheduled driver up with.
///
/// The same bundle `blk::Scheduling` is, written again for the reason the module
/// comment gives about every other type in this file.
#[derive(Clone, Copy)]
pub struct Scheduling {
    /// The core the driver is given. Unit: none — a core index.
    pub cpu: usize,
    /// The rate that core arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks it asks for. Unit: timer ticks.
    pub target: u64,
    /// This machine's timestamp-counter rate, for bounding a wait.
    /// Unit: kilohertz.
    pub tsc_khz: u64,
    /// The physical address of the frame the state tree is published in.
    /// Unit: bytes, physical.
    pub tree: u64,
}

/// The address space a driver is built in, and what it is stood up with.
struct Setup<'a> {
    space: &'a AddressSpace,
    features: Features,
    scheduling: Scheduling,
}

/// The register window as a *component* sees it: one base and four offsets.
///
/// A component may not be told four unrelated addresses. A modern virtio
/// transport publishes its four structures inside one base-address register, and
/// what the manifest declares is *four register frames* — one window, whole,
/// which the driver narrows with `Window::slice`. Narrowing only ever goes
/// inwards, so a driver that got an offset wrong reads its own registers wrongly
/// and cannot read anybody else's.
#[derive(Clone, Copy)]
struct Registers {
    /// The first page of the span, physical. Unit: bytes, physical.
    base: u64,
    /// How many pages it covers. Unit: pages.
    pages: u32,
    /// Each structure's offset into the span and its length, in the order
    /// common, notify, ISR, device configuration. Unit: bytes.
    each: [(u32, u32); 4],
}

impl Registers {
    /// Work out the span from what the device published.
    ///
    /// # Errors
    ///
    /// [`Trouble::Manifest`] for a span wider than the manifest declares or than
    /// the driver's address space reserves — which is the direction
    /// `user/virtio-net/manifest.toml` insists on: *a device whose BAR is larger
    /// is a different device and a different manifest, not a bigger number.*
    fn of(found: &virtio::Found, declared: &Declared) -> Result<Self, Trouble> {
        let structures = [found.common, found.notify, found.isr, found.device];
        let mut low = u64::MAX;
        let mut high = 0;
        for structure in structures {
            let physical = Self::physical(&structure)?;
            let end = physical.checked_add(u64::from(structure.len)).ok_or(Trouble::Manifest)?;
            low = low.min(physical & !(FRAME_SIZE - 1));
            high = high.max(end.div_ceil(FRAME_SIZE).saturating_mul(FRAME_SIZE));
        }
        let span = high.checked_sub(low).ok_or(Trouble::Manifest)?;
        let pages = u32::try_from(span / FRAME_SIZE).map_err(|_| Trouble::Manifest)?;
        if pages > declared.frames || pages as usize > crate::process::BLK_REGISTER_PAGES {
            return Err(Trouble::Manifest);
        }
        let mut each = [(0, 0); 4];
        for (slot, structure) in each.iter_mut().zip(structures) {
            let offset = u32::try_from(Self::physical(&structure)?.wrapping_sub(low))
                .map_err(|_| Trouble::Manifest)?;
            *slot = (offset, structure.len);
        }
        Ok(Self { base: low, pages, each })
    }

    /// Where a structure is in physical memory.
    fn physical(structure: &virtio::Structure) -> Result<u64, Trouble> {
        structure.at.checked_sub(paging::DEVICE_OFFSET).ok_or(Trouble::Manifest)
    }
}

/// The frame's half of a scheduled driver's run: the client's ring, the driver's
/// control ring, and the authority behind both.
///
/// What is here is exactly what a *supervisor* holds and a driver does not: the
/// remapping unit, the domain the device is attached to, the allocator, and the
/// client's capability table.
struct Supervising<'a, 'm> {
    /// What the driver asked for.
    asks: &'a Consumer<'m>,
    /// Where its answers go, and where the frame's notices go.
    answers: &'a Poster<'m>,
    /// The client's end of the data ring, on the frame's side.
    reaper: &'a Collector<'m>,
    unit: &'a mut Unit,
    domain: &'a mut crate::arch::x86_64::vtd::Domain,
    frames: &'a mut FrameAllocator,
    /// The **client's** table. Every handle a driver names in a translation
    /// request is resolved against this one, which is what makes a driver unable
    /// to grant itself anything.
    table: &'a Table,
    /// The device address the last translation answered.
    /// Unit: bytes, in the device's address space.
    answered_at: u64,
    /// How many operations this has answered on the driver's control ring.
    /// Unit: operations.
    answered: u32,
}

impl Supervising<'_, '_> {
    /// Where the last translation this served put the memory it was asked about.
    /// Unit: bytes, in the device's address space.
    const fn answered_at(&self) -> u64 {
        self.answered_at
    }

    /// Answer everything the driver has asked for, and nothing else.
    ///
    /// **This is the frame's polling point.** R05: nothing is delivered
    /// asynchronously, and what happens here is this core looking at a ring in
    /// its own loop while another core is inside a component.
    ///
    /// # Errors
    ///
    /// [`Trouble::Channel`] for a ring that stopped validating.
    fn serve(&mut self) -> Result<u32, Trouble> {
        let mut answered = 0;
        loop {
            let Some(entry) = self.asks.pop().map_err(|_| Trouble::Channel(0))? else {
                return Ok(answered);
            };
            // Room before the entry is acted on, because an operation performed
            // and then not answered is a driver waiting forever for a reply that
            // was dropped on the floor.
            if self.answers.free().map_err(|_| Trouble::Channel(0))? == 0 {
                return Err(Trouble::Channel(0));
            }
            let answer = self.execute(&entry);
            self.answers.post(answer).map_err(|_| Trouble::Channel(0))?;
            answered += 1;
            self.answered = self.answered.saturating_add(1);
        }
    }

    /// One control-ring operation.
    ///
    /// R04 at the bottom: an opcode this build does not implement is refused and
    /// never ignored. The two it does implement are the ones a driver cannot
    /// perform for itself, and both go through the same [`iommu::Grant`] the
    /// block datapath uses — so the check that stands between a component's
    /// clients and each other's memory is the same check, on the same table,
    /// with the same refusal, for both drivers.
    fn execute(&mut self, entry: &f_abi::Sqe) -> f_abi::Cqe {
        let mut asking = iommu::Grant {
            unit: &mut *self.unit,
            domain: &mut *self.domain,
            frames: &mut *self.frames,
            table: self.table,
        };
        match entry.opcode {
            control::op::DEVICE_MAP => match asking.map(entry.cap, entry.len) {
                Ok(address) => {
                    self.answered_at = address;
                    f_abi::Cqe {
                        user_data: entry.user_data,
                        result: 0,
                        flags: 0,
                        timestamp: 0,
                        ext: address,
                    }
                }
                Err((packed, detail)) => f_ring::refusal(entry.user_data, packed, detail, 0),
            },
            control::op::DEVICE_UNMAP => {
                asking.unmap(entry.cap, entry.offset, entry.len);
                f_ring::completion(entry.user_data, 0, 0)
            }
            other => f_ring::refusal(
                entry.user_data,
                error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
                u64::from(other),
                0,
            ),
        }
    }

    /// Take the client's next completion if one arrives inside `micros`, serving
    /// the driver until it does.
    ///
    /// Answers `None` for a bound that passed, which on this datapath is a
    /// **result** on two halves out of three and a failure on the third — see
    /// [`RECEIVE_MICROS`]. That is the one place this file's shape differs from
    /// `kernel/src/blk.rs`'s, and the reason is the reason for the whole task:
    /// a block request is a question a device owes an answer to, and a posted
    /// receive is not.
    ///
    /// # Errors
    ///
    /// Whatever [`Supervising::serve`] refuses.
    fn within(&mut self, tsc_khz: u64, micros: u64) -> Result<Option<f_abi::Cqe>, Trouble> {
        let deadline = crate::smp::deadline_after(tsc_khz, micros);
        loop {
            self.serve()?;
            if let Some(answer) = self.reaper.take().map_err(|_| Trouble::Channel(0))? {
                return Ok(Some(answer));
            }
            if crate::smp::past(deadline) {
                return Ok(None);
            }
            core::hint::spin_loop();
        }
    }

    /// [`Supervising::within`], where a bound that passes is a failure.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoAnswer`] for a driver that did not answer inside
    /// [`ANSWER_MICROS`] — a wedge, or a machine slower than that bound, and this
    /// cannot tell those apart, which is what [`Trouble::bound`] exists to say in
    /// the boot log.
    fn awaited(&mut self, tsc_khz: u64) -> Result<f_abi::Cqe, Trouble> {
        self.within(tsc_khz, ANSWER_MICROS)?.ok_or(Trouble::NoAnswer(ANSWER_MICROS))
    }

    /// Tell the driver to stop.
    ///
    /// RFC 0008's stop, as the one notice this run posts. On this driver it is
    /// also what triggers the cancellation of every receive buffer the device is
    /// still holding, which is an obligation the block driver never has.
    ///
    /// # Errors
    ///
    /// [`Trouble::Channel`] for a control ring with no room left.
    fn stop(&self) -> Result<(), Trouble> {
        self.answers
            .post(control::entry(control::notice::STOP, 0, 0, 0))
            .map_err(|_| Trouble::Channel(0))
    }
}

/// Run the datapath once.
///
/// # Errors
///
/// [`Trouble`], every variant of which means the datapath did not run.
///
/// # Safety
///
/// Call on the boot processor with the kernel's address space in `CR3`, `frames`
/// rebound onto its direct map, `unit` enabled, and nothing else in this kernel
/// driving the device this finds.
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
    // sized from it.
    // SAFETY: the caller's guarantee that the direct map is live and covers every
    // module.
    let declared = unsafe { declared(boot) }?;

    // The same finder the block datapath uses, with two different constants —
    // which is the whole of what the frame's device discovery owed a second
    // driver. `kernel/src/arch/x86_64/virtio.rs` was written parameterised by
    // device id and named this task as the caller that would use it, and it was
    // right.
    // SAFETY: the caller's guarantee, passed down.
    let found = unsafe {
        virtio::route(
            frames,
            space,
            features,
            window,
            survey,
            virtio::VIRTIO_NET_MODERN,
            virtio::VIRTIO_NET_TRANSITIONAL,
        )
    }
    .map_err(Trouble::Device)?;

    // The device has to fit what the manifest declares, and the refusal is in
    // that direction on purpose.
    if found.pages > declared.frames {
        return Err(Trouble::Manifest);
    }

    let before = frames.free_count();
    // What the *unit* keeps, as opposed to what the demonstration spends. A
    // bus's context table is the unit's for the life of the machine.
    let kept = unit.tables().len();
    // A domain of the component's own, before anything is allocated for it: a
    // driver with no domain is a driver whose device addresses physical memory.
    // SAFETY: the caller's guarantee that frames are addressable.
    let mut domain = unsafe { unit.domain(frames) }.map_err(Trouble::Unit)?;

    if declared.bytes < u64::from(driver::GRANT_BYTES) {
        return Err(Trouble::Manifest);
    }
    let region_order = order_for(declared.bytes).ok_or(Trouble::Manifest)?;
    let granted = frames.alloc_zeroed(region_order).ok_or(Trouble::NoFrames)?;
    let owned = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;
    let wire = frames.alloc_zeroed(Order::FRAME).ok_or(Trouble::NoFrames)?;

    // SAFETY: three frames just allocated, each held by nobody else, and the
    // caller's guarantees passed down.
    let outcome = unsafe {
        run(
            frames,
            unit,
            &mut domain,
            &found,
            declared,
            granted,
            owned,
            wire,
            half,
            Setup { space, features, scheduling },
        )
    };

    // Whatever happened, the device stops being able to address memory before
    // its domain is freed. On a network device that ordering carries more than
    // it does on a block one: a device with posted receive buffers writes into
    // them when a packet arrives, and a packet arriving is not something this
    // kernel decides.
    // SAFETY: `found.config` is the function's configuration space.
    unsafe { pci::command_clear(found.config, pci::COMMAND_BUS_MASTER) };
    // SAFETY: the caller's guarantee, and `bdf` is the function `run` attached.
    let _ = unsafe { unit.detach(frames, found.bdf) };
    // SAFETY: nothing is attached and no device is walking these tables.
    unsafe { unit.release(frames, domain) };

    // SAFETY: allocated above, at the order each is freed at, and the device is
    // detached and stripped of bus mastering.
    unsafe { frames.free(granted) };
    // SAFETY: as above.
    unsafe { frames.free(owned) };
    // SAFETY: as above.
    unsafe { frames.free(wire) };

    let report = outcome?;
    // Everything the demonstration took, back where it started — with the unit's
    // own retained tables taken out. Two numbers rather than a tolerance: a check
    // with slack in it is a check that stops noticing the first frame.
    let retained = unit.tables().len().saturating_sub(kept) as u64;
    if frames.free_count().saturating_add(retained) != before {
        return Err(Trouble::Leaked);
    }
    Ok(report)
}

/// The datapath proper, with every allocation already made.
///
/// # Safety
///
/// As [`demonstrate`], and every frame must be one the caller allocated for this.
#[expect(
    clippy::too_many_arguments,
    reason = "every one is a thing the caller allocated or found; bundling them into a struct \
              would be a type that exists so that a lint passes"
)]
unsafe fn run(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    domain: &mut crate::arch::x86_64::vtd::Domain,
    found: &virtio::Found,
    declared: Declared,
    granted: Frame,
    owned: Frame,
    wire: Frame,
    half: Half,
    setup: Setup<'_>,
) -> Result<Report, Trouble> {
    // --- the two tables ------------------------------------------------------
    //
    // The driver's table holds the region its manifest declares; the client's
    // holds the page it is about to register. A registration resolves the
    // *client's* handle against the *client's* table and maps it into the
    // *driver's* domain. One table holding both would have made every check below
    // pass for a reason nobody chose.
    let mut driver_table = Table::EMPTY;
    let mut client_table = Table::EMPTY;

    let region_cap = driver_table
        .grant(CapType::Frame, GRANTABLE, granted.addr(), granted.bytes())
        .map(|handle| handle.bits())
        .map_err(|_| Trouble::Authority)?;
    let owned_cap = client_table
        .grant(CapType::Frame, GRANTABLE, owned.addr(), owned.bytes())
        .map(|handle| handle.bits())
        .map_err(|_| Trouble::Authority)?;
    // The same page a second time, without the right to hand it on. The pair is
    // the point: two handles naming the same bytes, separated by authority alone.
    let ungrantable_cap = client_table
        .grant(CapType::Frame, rights::READ | rights::WRITE, owned.addr(), owned.bytes())
        .map(|handle| handle.bits())
        .map_err(|_| Trouble::Authority)?;

    // --- the driver's own grant ---------------------------------------------
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

    // --- the device joins the domain ----------------------------------------
    //
    // Last, so that a refusal above leaves nothing attached, and before bus
    // mastering, so the device cannot issue a transaction until there is a domain
    // to translate it.
    // SAFETY: the caller's guarantee, and `bdf` is the function whose registers
    // are about to be driven.
    unsafe { unit.attach(frames, found.bdf, domain) }.map_err(Trouble::Unit)?;
    // SAFETY: `found.config` is the function's mapped configuration space.
    unsafe { pci::command_set(found.config, pci::COMMAND_BUS_MASTER) };

    // --- the component ------------------------------------------------------
    let registers = Registers::of(found, &declared)?;
    let plan = crate::process::DriverPlan {
        image: declared.image,
        selector: half.selector(),
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
    // `granted` and `wire` are frames this call's caller allocated and holds, and
    // `cpu` is a core that is up and idle.
    let (prepared, pages) =
        unsafe { crate::process::prepare_driver(frames, setup.space, setup.features, plan) }
            .map_err(Trouble::Process)?;

    // --- the two rings ------------------------------------------------------
    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Authority)?;
    let at = frames.virt(wire);
    // SAFETY: `wire` was allocated zeroed by the caller, is frame-aligned and is
    // `FRAME_SIZE` bytes with no pointer into it held anywhere else. Written and
    // then let go of: the frame is the grantor and writes the header, and the
    // *server* end over these bytes is the component's.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and every
    // accessor hands out atomics and `UnsafeCell`s rather than references.
    let client_end = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;
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

    // --- what the component is told ------------------------------------------
    let board = Window::at(pages.board, routing::BYTES).map_err(Trouble::Channel)?;
    let negotiated = Negotiated { version: ABI_VERSION, features: 0 };
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
        (routing::at::NEGOTIATED_VERSION, u64::from(negotiated.version)),
        (routing::at::NEGOTIATED_FEATURES, negotiated.features),
        (routing::at::BEYOND, half.beyond()),
        (routing::at::ADMITTED, u64::from(declared.admitted)),
        // The client is admitted for the hard class on every half. RFC 0025's
        // bound 2 — a peer claiming a class it does not hold — is the block
        // datapath's `unadmitted` half and is not re-run here: it is a property
        // of `f_abi::deadline::inherit`, which both drivers call, and a second
        // boot of the same arithmetic would be a fixture rather than evidence.
        (routing::at::CLIENT_ADMITTED, u64::from(class::HARD)),
        (routing::at::FLOOR, FLOOR_NS),
        (routing::at::RECEIVE_SPINS, RECEIVE_SPINS),
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
    // put in its shards by `prepare_driver`, and this core has interrupts enabled.
    unsafe { crate::smp::start_on(setup.scheduling.cpu) }.map_err(Trouble::Scheduled)?;

    let asks = Consumer::new(control.channel()).ok_or(Trouble::Channel(0))?;
    let answers = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;
    let reaper = Collector::new(client_end.completions()).ok_or(Trouble::Channel(0))?;
    let mut producer = Producer::new(client_end.channel()).ok_or(Trouble::Channel(0))?;
    // The client's page, taken before the allocator is borrowed for the length of
    // the run.
    // SAFETY: `owned` is a frame the caller allocated and handed to nobody else;
    // the direct map makes it readable and writable for the whole of this call,
    // and no other reference into it exists.
    let page = unsafe { core::slice::from_raw_parts_mut(frames.virt(owned), FRAME_SIZE as usize) };
    let tsc_khz = setup.scheduling.tsc_khz;
    let asking = Asking { half, owned_cap, ungrantable_cap, bytes, tsc_khz, negotiated };

    let mut supervising = Supervising {
        asks: &asks,
        answers: &answers,
        reaper: &reaper,
        unit: &mut *unit,
        domain: &mut *domain,
        frames: &mut *frames,
        table: &client_table,
        answered_at: 0,
        answered: 0,
    };
    let observed = client(&mut supervising, &mut producer, page, asking);
    // Told to stop whatever happened above, because a driver left serving a
    // client that has gone is a core this boot never gets back. It is also what
    // makes the driver give its posted receive buffers back.
    let told = supervising.stop();
    // SAFETY: `start_on` was called for this core and nothing else has joined it.
    // `serve` touches only the driver's control ring, whose two ends are
    // single-producer and single-consumer by construction.
    let joined = unsafe {
        crate::smp::join_serviced(setup.scheduling.cpu, tsc_khz, EXIT_MICROS, &mut || {
            let _ = supervising.serve();
        })
    };

    let asked = supervising.answered;

    // What the component said about itself, read out of memory the frame granted
    // it. RFC 0013's *read, never delivered*: it was never asked.
    let reported = Reported::of(&board);

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished — which is what `join_serviced` returning `Ok` means.
    let ended = unsafe { crate::process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    let exited = matches!(ended.death, crate::process::Death::Exited(_));

    // What the component said about itself, printed only when the *client* gave
    // up — and it is here rather than in `blk.rs` because this datapath can fail
    // in a way that one cannot. A block client that stops getting answers has a
    // driver that is wedged or a machine that is slow; a network client can also
    // be waiting for a packet it caused nothing to send, and all three read
    // identically from outside. This line is what separates *the component
    // faulted before it read its own routing page* from *the component is serving
    // and nothing has arrived*, and without it the only evidence either way is a
    // bound that passed.
    //
    // The death comes first because it is the frame's own reading and everything
    // after it is the component's: a driver that faulted wrote no report, so a
    // row of zeroes below a fault means *nothing ran* and the same row below an
    // `EXIT` means *it ran and did nothing*.
    if observed.is_err() {
        match ended.death {
            crate::process::Death::Killed { vector, error, address, rip } => crate::kprintln!(
                "  net stalled   the component was killed: vector {vector}, error {error:#x}, \
                 address {address:#018x}, at {rip:#018x}"
            ),
            crate::process::Death::Exited(status) => {
                crate::kprintln!(
                    "  net stalled   the component ended by EXIT with status {status}"
                );
            }
            crate::process::Death::Running => {
                crate::kprintln!("  net stalled   the component never reported an ending");
            }
        }
        crate::kprintln!(
            "  net stalled   it reported outcome {}, having drained {} entr(ies), served {}, \
             refused {}, posted {}, received {}, spun {}",
            reported.outcome,
            reported.drained,
            reported.counters.served,
            reported.counters.refused,
            reported.counters.posted,
            reported.counters.received,
            reported.counters.spun,
        );
    }

    told?;
    joined.map_err(|why| match why {
        crate::smp::NotJoined::Refused(cpu) => Trouble::Scheduled(cpu),
        // Not `Scheduled`: the core said nothing, the bound said everything.
        crate::smp::NotJoined::Overdue(_) => Trouble::Overdue(EXIT_MICROS),
    })?;
    let observed = observed?;
    let faults = unit.faults();

    Ok(Report {
        half,
        declared,
        bdf: found.bdf,
        windows: registers.pages,
        registered_at: observed.registered_at,
        refused_without_grant: observed.refused_without_grant,
        transmitted: observed.transmitted,
        landed: observed.landed,
        frame_bytes: observed.frame_bytes,
        matched: observed.matched,
        untouched: observed.untouched,
        counters: reported.counters,
        cpu: setup.scheduling.cpu,
        exited,
        drained: reported.drained,
        stopped: reported.outcome,
        asked,
        fault: faults.first,
        faults: faults.records,
    })
}

/// What the client asks for and holds, in one bundle.
#[derive(Clone, Copy)]
struct Asking {
    half: Half,
    /// The client's page, as a handle it may hand on. Unit: none — a handle.
    owned_cap: u32,
    /// The same page, as a handle it may not. Unit: none — a handle.
    ungrantable_cap: u32,
    /// How many bytes of it. Unit: bytes.
    bytes: u32,
    /// Unit: kilohertz.
    tsc_khz: u64,
    negotiated: Negotiated,
}

/// What the client observed.
#[derive(Clone, Copy)]
struct Observed {
    /// Unit: bytes, in the device's address space.
    registered_at: u64,
    refused_without_grant: bool,
    transmitted: bool,
    landed: bool,
    /// Unit: bytes.
    frame_bytes: u32,
    matched: bool,
    /// Unit: bytes.
    untouched: u32,
}

/// The token each of the client's four entries carries.
///
/// Distinct constants rather than a running number, because on this datapath the
/// completions **do not arrive in submission order**: the receive is submitted
/// first and answered last, or not at all. A client that assumed the order would
/// hand the wrong buffer back — and `InFlight::complete` is what refuses that,
/// by requiring the token to match.
mod token {
    /// The registration that must be refused for want of `GRANT`.
    pub const PROBE: u64 = 1;
    /// The registration that must succeed.
    pub const REGISTER: u64 = 2;
    /// The receive buffer, posted before anything is sent.
    pub const RECEIVE: u64 = 3;
    /// The frame put on the link.
    pub const TRANSMIT: u64 = 4;
}

/// The client's whole run: register, post, send, and look at what arrived.
///
/// Every wait in here serves the driver's control ring while it waits — so the
/// client and its server make progress against each other on two cores, through
/// one ring, with the frame answering the one question the driver cannot answer
/// for itself.
fn client(
    supervising: &mut Supervising<'_, '_>,
    producer: &mut Producer<'_>,
    page: &mut [u8],
    asking: Asking,
) -> Result<Observed, Trouble> {
    // --- the refusal, before anything is registered -------------------------
    //
    // A client that holds memory it may use and may not pass on cannot put it in
    // a driver's domain. Provoked on every run because a check nobody has watched
    // fail is indistinguishable from one that cannot fail.
    let probe = registration(token::PROBE, asking.ungrantable_cap, asking.bytes, BUFFERS);
    producer.submit(probe).map_err(|_| Trouble::Channel(0))?;
    let answer = supervising.awaited(asking.tsc_khz)?;
    let refused_without_grant =
        matches!(answer.error(), Some((error::AUTHORITY, error::authority::RIGHT_NOT_HELD)));
    if !refused_without_grant {
        return Err(Trouble::NotRefused);
    }

    // --- the registration ---------------------------------------------------
    let asked = registration(token::REGISTER, asking.owned_cap, asking.bytes, BUFFERS);
    producer.submit(asked).map_err(|_| Trouble::Channel(0))?;
    let answer = supervising.awaited(asking.tsc_khz)?;
    let naming = Fixed::from_completion(&answer)
        .map_err(|(refused, code)| Trouble::Registration(error::pack(refused, code)))?;
    // Where the *device* addresses the set. The frame knows it because the frame
    // answered the translation, and it is the frame's knowledge and never the
    // client's: nothing in the completion the *client* reaped carries an address.
    let registered_at = supervising.answered_at();

    // --- the client's buffers -----------------------------------------------
    //
    // The ownership types, over the page the registration just named. An `Idle`
    // is the only thing here that reaches bytes, a submission *moves* it, and the
    // completion is what hands it back — RFC 0024.
    //
    // On this driver that rule is doing the work it was designed for rather than
    // a milder version of it. The block driver's `InFlight` is a buffer a device
    // is reading or writing during one function call on another core; this one is
    // a buffer a **network card** may write into at any moment until a frame
    // arrives, and there is no method on `InFlight` that reaches its bytes.
    let mut set = BufferSet::bind(naming, asking.negotiated, page).map_err(Trouble::Channel)?;
    let carved = set.carve::<{ BUFFERS as usize }>().map_err(|_| Trouble::Geometry)?;
    // The pattern below is what makes [`SINK`] and [`SOURCE`] true, and an
    // assertion is what says so rather than a comment: [`Report::expected_fault`]
    // is the registration's own answer plus [`BEYOND`] *only* while the sink is
    // buffer zero, so a reader who swapped the two names here would move an
    // address the boot log prints and nothing would notice.
    const _: () = assert!(SINK == 0 && SOURCE == 1);
    let [mut sink, mut source] = carved;
    if sink.len() < driver::FRAME_MAX as usize || source.len() < FRAME_BYTES as usize {
        return Err(Trouble::Geometry);
    }

    // Poison, so that *nothing landed* and *something landed* are different
    // observations rather than one.
    for byte in sink.bytes_mut().iter_mut() {
        *byte = POISON;
    }
    form_request(source.bytes_mut())?;

    // --- the receive, posted before anything is sent ------------------------
    //
    // Before, and it is not an optimisation: a reply that arrives with no buffer
    // posted is a frame the device drops, and a run that posted afterwards would
    // be a run whose result depended on which of two machines was quicker.
    //
    // Nothing is awaited here. A receive is accepted and answered later — that is
    // the whole shape of this driver — so a client that waited on it would wait
    // for a packet it has not yet caused.
    let recv = driver::recv(token::RECEIVE, sink.len() as u32);
    let (lent_sink, _) = sink.submit(producer, recv).map_err(|_| Trouble::Channel(0))?;
    // In an `Option` for the whole of the rest of this function, and it is the
    // receive direction that forces it. A block client submits and awaits, so
    // its buffer is in flight across one statement; this one is in flight across
    // a transmit, a bounded wait, a stop and a second bounded wait, and
    // `InFlight::complete` consumes the buffer to answer *is this yours*. The
    // `Option` is the state between asking and being told.
    let mut lent_sink = Some(lent_sink);

    // --- the frame on the link ----------------------------------------------
    let mut transmitted = false;
    let source = if asking.half.transmits() {
        // The hard class, written explicitly, because `Sqe::ZERO` writes
        // `class::BATCH` and a batch entry at a soft-class service is not
        // demoted at all — so a client that left the field alone would never
        // exercise R08 here and the counter beside it would read zero for a
        // reason that has nothing to do with the driver. This is the one entry
        // in this demonstration that asks for urgency, and the manifest's
        // `class = "soft"` is what refuses it: the frame must be *told* the
        // request was served below what it asked.
        let mut send = driver::send(token::TRANSMIT, FRAME_BYTES);
        send.class = f_abi::deadline::pack(class::HARD, 0);
        send.deadline = HARD_DEADLINE_NS;
        let (lent, _) = source.submit(producer, send).map_err(|_| Trouble::Channel(0))?;
        let answer = supervising.awaited(asking.tsc_khz)?;
        transmitted = !answer.is_error();
        lent.complete(&answer).map_err(|_| Trouble::Transfer(0))?
    } else {
        source
    };

    // --- what came back, or did not -----------------------------------------
    //
    // One bound for all three halves, used in two opposite ways: on `inside` a
    // bound that passes is a failure and on the other two it is the result. A
    // control that waited less than the experiment would be a control that proves
    // only that it was more impatient.
    let mut landed = false;
    let mut frame_bytes = 0;
    let mut sink_back = None;
    // Whether this client has already told the driver to stop. Once, and the
    // once matters: `run` tells it again on the way out whatever happened here,
    // and a client that told it on every turn of the loop below would be filling
    // a control ring with notices for a component that has already ended.
    let mut stopped = false;
    if let Some(answer) = supervising.within(asking.tsc_khz, RECEIVE_MICROS)?
        && let Some(lent) = lent_sink.take()
    {
        match lent.complete(&answer) {
            Ok(idle) => {
                landed = !answer.is_error();
                if landed {
                    frame_bytes = u32::try_from(answer.result).unwrap_or(0);
                }
                sink_back = Some(idle);
            }
            // A completion this client is not waiting for. There is none in this
            // build; keeping the buffer rather than dropping it is what stops an
            // unexpected entry becoming an abort.
            Err(still) => lent_sink = Some(still),
        }
    }

    // The buffer, when it did not come back through a completion — which is the
    // ordinary case on two halves out of three, and is the shape of thing a
    // block client never has to deal with.
    //
    // Told to stop, the driver gives every posted receive back as a
    // cancellation. That exchange exists because RFC 0024 gives an in-flight
    // buffer exactly three exits — a completion carrying its token, `reclaim` on
    // evidence the peer is gone, and a drop that ends the component — and a
    // **live, healthy peer with nothing to give back** is none of the three.
    // `f_virtio_net::driver::Driver::cancel` is the service's half of it and
    // this is the client's, and it is written as a wait rather than as a
    // `reclaim` because the peer is not gone and `PeerGone` cannot be
    // constructed from *the service stopped politely*.
    while sink_back.is_none()
        && let Some(lent) = lent_sink.take()
    {
        if !stopped {
            supervising.stop()?;
            stopped = true;
        }
        let Some(answer) = supervising.within(asking.tsc_khz, RECEIVE_MICROS)? else {
            // Nothing came, and there is no legal way to take the buffer back.
            // Dropping it is what happens on the way out of this function and it
            // is the right thing: `f_ring::buffers` refuses a dropped in-flight
            // buffer loudly, and under `panic = "abort"` that ends the frame —
            // which is the outcome a client holding a buffer a device may still
            // be pointed at has earned. A quiet `Trouble` here would be this file
            // deciding that a network card writing into memory nobody owns is a
            // reportable condition.
            drop(lent);
            return Err(Trouble::Transfer(error::pack(error::PEER, error::peer::GONE)));
        };
        match lent.complete(&answer) {
            Ok(idle) => sink_back = Some(idle),
            Err(still) => lent_sink = Some(still),
        }
    }

    let Some(sink) = sink_back else { return Err(Trouble::Geometry) };

    // --- the bound, said as a bound -----------------------------------------
    //
    // Here rather than at the wait itself, and *after* the buffer is back:
    // `lent_sink` still held an in-flight buffer at the wait, and returning
    // there would drop it — which `f_ring::buffers` refuses loudly, ending the
    // frame under `panic = "abort"` instead of printing a reason. So the
    // obligation is discharged first and the diagnosis given second.
    //
    // [`Half::expects_frame`] is the predicate, and it is called rather than
    // matched on so that *which half must receive* has one definition. On the
    // other two halves an empty sink is the result and this says nothing.
    if asking.half.expects_frame() && !landed {
        return Err(Trouble::NoFrame(RECEIVE_MICROS));
    }

    // --- what is actually in the client's memory ----------------------------
    let mut untouched = 0;
    for index in 0..FRAME_BYTES as usize {
        let Some(got) = sink.bytes().get(index) else { return Err(Trouble::Geometry) };
        if *got == POISON {
            untouched += 1;
        }
    }
    let matched = landed && is_reply_to_us(sink.bytes());
    // Read back rather than trusted: the source buffer must still hold the frame
    // this client formed, because a device that wrote into the *transmit* buffer
    // would be a corruption nothing else here would notice.
    let mut formed = [0u8; FRAME_BYTES as usize];
    form_request(&mut formed)?;
    for index in 0..FRAME_BYTES as usize {
        let (Some(got), Some(want)) = (source.bytes().get(index), formed.get(index)) else {
            return Err(Trouble::Geometry);
        };
        if got != want {
            return Err(Trouble::Transfer(0));
        }
    }

    Ok(Observed {
        registered_at,
        refused_without_grant,
        transmitted,
        landed,
        frame_bytes,
        matched,
        untouched,
    })
}

/// Form the address-resolution request this boot puts on the link.
///
/// By hand, in the client, and both of those are decisions. **By hand** because
/// there is no network stack in this system and building one to send one frame
/// would be building the thing E2 owes rather than the thing E1-B03 owes. **In
/// the client** because the frame's addresses are the client's — a driver that
/// chose them would be a driver with an identity of its own, which is what
/// `VIRTIO_NET_F_MAC` would have given it and which this driver deliberately does
/// not negotiate.
///
/// # Errors
///
/// [`Trouble::Geometry`] for a buffer shorter than the frame, which the caller
/// has already refused.
fn form_request(into: &mut [u8]) -> Result<(), Trouble> {
    let Some(frame) = into.get_mut(..FRAME_BYTES as usize) else { return Err(Trouble::Geometry) };
    for byte in frame.iter_mut() {
        *byte = 0;
    }
    let put = |frame: &mut [u8], at: usize, bytes: &[u8]| -> Result<(), Trouble> {
        let Some(slot) = frame.get_mut(at..at + bytes.len()) else { return Err(Trouble::Geometry) };
        slot.copy_from_slice(bytes);
        Ok(())
    };
    // Broadcast, because nothing on this machine knows the gateway's hardware
    // address — which is the question the frame asks.
    put(frame, at::DESTINATION, &[0xFF; 6])?;
    put(frame, at::SOURCE, &OUR_MAC)?;
    put(frame, at::ETHERTYPE, &ETHERTYPE_ARP.to_be_bytes())?;
    // One is Ethernet and 0x0800 is IPv4, in the body's own numbering. Written
    // big-endian because every multi-byte field on a wire is, which is the one
    // place in this tree where that is true and is worth the explicit
    // conversion rather than a comment.
    put(frame, at::HARDWARE, &1u16.to_be_bytes())?;
    put(frame, at::PROTOCOL, &0x0800u16.to_be_bytes())?;
    put(frame, at::HARDWARE_LEN, &[6])?;
    put(frame, at::PROTOCOL_LEN, &[4])?;
    put(frame, at::OPERATION, &ARP_REQUEST.to_be_bytes())?;
    put(frame, at::SENDER_MAC, &OUR_MAC)?;
    put(frame, at::SENDER_IP, &OUR_IP)?;
    // The target hardware address is zero in a request: it is what is being
    // asked for.
    put(frame, at::TARGET_IP, &GATEWAY_IP)?;
    Ok(())
}

/// Is this frame a reply to the request [`form_request`] formed?
///
/// Five fields, and the last is the one that makes this a demonstration rather
/// than an observation. A host network backend answers address resolution for
/// its own gateway, so *an ARP reply arrived* would be satisfied by a link with
/// any traffic on it at all. A reply whose **target hardware address is the one
/// this boot invented** is a reply to this boot's request: nothing outside this
/// machine had that address until the request carried it.
fn is_reply_to_us(frame: &[u8]) -> bool {
    let field = |at: usize, len: usize| frame.get(at..at + len);
    let two = |at: usize| {
        field(at, 2).and_then(|bytes| <[u8; 2]>::try_from(bytes).ok()).map(u16::from_be_bytes)
    };
    two(at::ETHERTYPE) == Some(ETHERTYPE_ARP)
        && two(at::OPERATION) == Some(ARP_REPLY)
        && field(at::SENDER_IP, 4) == Some(&GATEWAY_IP[..])
        && field(at::TARGET_IP, 4) == Some(&OUR_IP[..])
        && field(at::TARGET_MAC, 6) == Some(&OUR_MAC[..])
}

/// What the component wrote about itself into the half of its board that is its
/// own.
#[derive(Clone, Copy)]
struct Reported {
    counters: driver::Counters,
    /// Unit: entries.
    drained: u64,
    /// One of `f_virtio_net::routing::stopped`. Unit: none — an ordinal.
    outcome: u64,
}

impl Reported {
    /// Read it, and answer zeroes for a component that never wrote one.
    ///
    /// The magic is what tells those two apart, and it matters: a component that
    /// faulted before it reached its own report would otherwise publish a copy
    /// counter of zero — which is the number this whole subsystem publishes as a
    /// property, arrived at by the component never having run.
    ///
    /// Every `u32` field is converted rather than cast, because this is the frame
    /// reading a page a ring-3 component writes and R04 applies: a component that
    /// put `1 << 32` into `SERVED` would otherwise be reported as having served
    /// none, and a truncation that lands on a plausible tally is worse than one
    /// that lands on an implausible one.
    fn of(board: &Window) -> Self {
        let read = |offset: u32| board.read64(offset).unwrap_or(0);
        if read(routing::reported::MAGIC) != routing::MAGIC {
            return Self::NOTHING_AT_ALL;
        }
        let (Ok(served), Ok(refused), Ok(escaped)) = (
            u32::try_from(read(routing::reported::SERVED)),
            u32::try_from(read(routing::reported::REFUSED)),
            u32::try_from(read(routing::reported::ESCAPED)),
        ) else {
            return Self::NOTHING_AT_ALL;
        };
        let (Ok(shortfall), Ok(unadmitted)) = (
            u32::try_from(read(routing::reported::SHORTFALL)),
            u32::try_from(read(routing::reported::UNADMITTED)),
        ) else {
            return Self::NOTHING_AT_ALL;
        };
        let (Ok(sent), Ok(received), Ok(posted), Ok(cancelled), Ok(halted)) = (
            u32::try_from(read(routing::reported::SENT)),
            u32::try_from(read(routing::reported::RECEIVED)),
            u32::try_from(read(routing::reported::POSTED)),
            u32::try_from(read(routing::reported::CANCELLED)),
            u32::try_from(read(routing::reported::HALTED)),
        ) else {
            return Self::NOTHING_AT_ALL;
        };
        Self {
            counters: driver::Counters {
                served,
                refused,
                bytes: read(routing::reported::BYTES),
                copies: read(routing::reported::COPIES),
                escaped,
                provoked: read(routing::reported::PROVOKED),
                shortfall,
                unadmitted,
                sent,
                received,
                posted,
                spun: read(routing::reported::SPUN),
                cancelled,
                halted,
            },
            drained: read(routing::reported::DRAINED),
            outcome: read(routing::reported::OUTCOME),
        }
    }

    /// What the frame knows about a component that reported nothing it can
    /// believe.
    ///
    /// Zero everywhere, and the zero on `provoked` is the one that matters:
    /// [`Report::verdict`] requires it to move, so a component that never ran
    /// fails on the same line a component whose self-check stopped working would.
    const NOTHING_AT_ALL: Self = Self { counters: NOTHING, drained: 0, outcome: 0 };
}

/// What a component that never reported has done, as far as the frame knows.
const NOTHING: driver::Counters = driver::Counters {
    served: 0,
    refused: 0,
    bytes: 0,
    copies: 0,
    escaped: 0,
    provoked: 0,
    shortfall: 0,
    unadmitted: 0,
    sent: 0,
    received: 0,
    posted: 0,
    spun: 0,
    cancelled: 0,
    halted: 0,
};

/// The allocator order that covers `bytes`, exactly.
///
/// Exactly, and not the next order up: a manifest declaring a quantity that is
/// not a whole number of frames at some order is a manifest that cannot be
/// satisfied by one allocation, and rounding up would hand a component more than
/// it declared.
fn order_for(bytes: u64) -> Option<Order> {
    if bytes == 0 || !bytes.is_multiple_of(FRAME_SIZE) {
        return None;
    }
    let pages = bytes / FRAME_SIZE;
    if !pages.is_power_of_two() {
        return None;
    }
    let order = u8::try_from(pages.trailing_zeros()).ok()?;
    Order::new(order)
}
