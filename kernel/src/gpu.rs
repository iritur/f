// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The display datapath, end to end: a third driver outside the frame, a client
//! with a registered buffer full of pixels, a real display controller, and a
//! picture that leaves the machine.
//!
//! # What this file is, and the one thing it is evidence of
//!
//! It is the **supervisor's half** of E1-B04. `user/virtio-gpu` is the driver:
//! the transport handshake, one virtqueue, the registration table, six display
//! commands and the service loop, in a crate that forbids `unsafe` and holds no
//! mapping of any client's memory. What is here is everything a supervisor does
//! around one — find the device, program its domain, route it four register
//! windows and one untyped region, stand a client up on the other end of a ring,
//! and judge what happened.
//!
//! **And here it cannot finish the judging, which is the difference.**
//! `kernel/src/blk.rs` decides whether a sector went out and came back by
//! reading the client's own memory. `kernel/src/net.rs` decides whether a frame
//! arrived by reading the client's own memory and the unit's fault registers.
//! Neither of those is available for a scanout: the 2D display protocol has no
//! command that reads a resource back, so **nothing inside this machine can
//! observe what is on the screen**. Every counter this file prints is a
//! statement about commands the display *accepted*, and a display that accepted
//! all six and drew nothing would move every one of them.
//!
//! So this file computes one number the harness can check its own observation
//! against — [`Report::display_hash`], taken over the client's own pixels in the
//! order a screen capture would report them — prints it, and then **holds the
//! machine still** while `cargo xtask gpu` captures the framebuffer from outside
//! the emulator. RFC 0054 argues why that is the honest reading of the exit
//! criterion and why a driver's own report cannot stand in for it.
//!
//! # Three halves, and the middle one is the control
//!
//! `gpu=inside` registers the client's page, fills one buffer of it with a
//! pattern, and submits one `show`. The driver creates a resource, attaches the
//! client's buffer as its backing, transfers, sets scanout zero, flushes and
//! detaches. The harness must then find that pattern on the host's display.
//!
//! `gpu=blank` is the identical client with the `show` removed. The pattern is
//! written into the registered buffer and **nothing is submitted**, so nothing
//! may reach the screen. That is the control the exit criterion needs and it is
//! sharper than a control that wrote nothing: the pixels are in guest memory the
//! whole time, and a harness that found them on the display would have found
//! them by some route other than the ring.
//!
//! `gpu=escape` submits the identical `show` and has the driver add [`BEYOND`]
//! to the address the registration answered before it becomes the backing entry
//! the display reads out of. The remapping unit must fault it — on a **read**,
//! which is the direction a display controller works in — the display must
//! refuse the command, and nothing may appear.
//!
//! The three provocations in this tree now cover the three things a device can
//! do with an address it should not have: `blk` reads memory it was not granted
//! into a client's buffer, `net` writes into memory nobody granted at a moment
//! nothing chose, and this one **reads memory it was not granted and puts it on
//! a screen**. That last one is the only one of the three whose consequence
//! leaves the machine.
//!
//! # What the demonstration does not show
//!
//! One frame, one format, one scanout, sixteen pixels square, no window system.
//! It says nothing about refresh rate, nothing about partial updates, nothing
//! about more than one client drawing at once, and nothing about what a display
//! does when two resources want the same scanout. It also cannot show that a
//! *person* would see the picture: what it shows is that the emulator's display
//! surface holds the client's bytes, which is as far outside the machine as this
//! tree can currently reach.

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
use f_virtio_gpu::driver;
use f_virtio_gpu::routing;

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
/// third time, in a third crate.
///
/// `f_virtio_gpu::routing::AT` is written down in the component and
/// `kernel::process::BLK_BOARD` in the frame, and they are linked separately.
/// The kernel is the one artefact that links every definition, so the agreement
/// is a check rather than a comment.
///
/// **Three of these assertions is what RFC 0051 said should stop happening at
/// three.** It has not stopped, and RFC 0054 says why: the layout belongs in
/// `abi/`, moving it touches two closed tasks' evidence, and a third assertion
/// costs one line while the move costs a review of `user/virtio-blk`.
const _: () = assert!(
    crate::process::BLK_BOARD == routing::AT,
    "the frame and the display driver disagree about where the routing page is"
);

/// The three drivers' routing pages are told apart by their magic and by nothing
/// else.
///
/// They are mapped at the same address, in the same shape, by the same loader.
/// If any two magics were equal, a build that routed one driver's supervisor at
/// another driver's image would find a page whose magic matched and whose fields
/// meant something else. The assertion is here rather than in any component
/// because this is the only place all three constants exist at once — and it
/// names all three rather than only the new one, because the cheap thing to get
/// wrong at three is to check the third against the first and forget the second.
const _: () = assert!(
    routing::MAGIC != f_virtio_blk::routing::MAGIC
        && routing::MAGIC != f_virtio_net::routing::MAGIC
        && f_virtio_blk::routing::MAGIC != f_virtio_net::routing::MAGIC,
    "the drivers' routing pages are mapped at one address and must not answer to one magic"
);

/// Entries on the client's data ring.
///
/// Sixteen, which is what fits one frame beside its completion ring and its
/// index ring — the same number and the same reason as a control ring's. The
/// manifest declares two hundred and fifty-six, which is what a real client gets
/// when a component pays for its own channel; this demonstration submits three
/// entries in total.
const ENTRIES: u32 = 16;

/// Buffers the client's registered set holds.
///
/// Four, over one page, so each is exactly [`FRAME_BYTES`] and the first one
/// starts at the base of the page. That is what makes
/// [`Report::expected_fault`] the registration's own answer plus [`BEYOND`] with
/// no per-buffer arithmetic between the boot log and the unit's fault record.
///
/// Three of them are never used, and that is deliberate rather than untidy: a
/// set with one buffer in it is a set where *the buffer* and *the set* are the
/// same object, and every off-by-one in the registration path would be invisible.
const BUFFERS: u32 = 4;

/// Which buffer of the set holds the pixels. Unit: buffers, zero-based.
const CANVAS: usize = 0;

/// How wide the frame this boot draws is. Unit: pixels.
///
/// Sixteen, and small on purpose. What has to be true of this number is that the
/// bytes it produces fit one buffer of a one-page registration, that the picture
/// is large enough for a *transposed* or *shifted* capture to disagree with the
/// original, and that the harness can hold the whole of it in memory to hash. A
/// larger frame would test nothing further and would put a longer wait between
/// the flush and the capture.
const WIDTH: u32 = 16;

/// How tall it is. Unit: pixels.
///
/// The same as [`WIDTH`], which would normally be a defect in a fixture — a
/// square frame cannot tell a transposed image from a correct one. It is not one
/// here because the *pattern* is not symmetric: [`pixel`] is a different colour
/// at `(x, y)` than at `(y, x)` for every pixel off the diagonal, so a capture
/// with the axes swapped hashes differently. Saying that here is cheaper than
/// making the frame oblong and leaving the reason to be rediscovered.
const HEIGHT: u32 = 16;

/// Bytes one pixel occupies, in the one format the driver creates resources in.
/// Unit: bytes.
const BYTES_PER_PIXEL: u32 = driver::BYTES_PER_PIXEL;

/// Bytes of pixels one frame is. Unit: bytes.
const FRAME_BYTES: u32 = WIDTH * HEIGHT * BYTES_PER_PIXEL;

const _: () = assert!(FRAME_BYTES * BUFFERS <= FRAME_SIZE as u32);

/// The byte the whole page is filled with before the pattern is written.
///
/// Not a byte the pattern contains at any position where it matters, so a buffer
/// still full of it is a buffer nothing wrote. It is also what the *unused*
/// three buffers of the set keep, which is what makes a display that transferred
/// the wrong buffer produce a hash that does not match rather than a picture
/// that is merely wrong.
const POISON: u8 = 0xA5;

/// The colour of the pixel at `(x, y)`, as the four bytes it occupies in memory.
///
/// # Why a gradient rather than a flag or a solid colour
///
/// Because the check has to fail for a picture that is *nearly* right. A solid
/// colour survives a transposition, a one-row shift, a wrong stride and a
/// half-height transfer; a gradient in three channels survives none of them. The
/// exclusive-or in the blue channel is what makes it asymmetric, which is what
/// lets [`WIDTH`] and [`HEIGHT`] be equal without the fixture losing the ability
/// to notice swapped axes.
///
/// # The one place two definitions of a format meet
///
/// `user/virtio-gpu/src/driver.rs` names the format
/// (`VIRTIO_GPU_FORMAT_B8G8R8X8_UNORM`) and this function is what knows what
/// that means in memory: four bytes, blue first, then green, then red, then a
/// byte the display ignores. [`rgb_at`] is the same knowledge in the other
/// direction — how the bytes come back out when a screen capture reports them —
/// and the pair of them is the only thing in this tree that could be wrong about
/// a pixel layout without anything else noticing.
///
/// What keeps that honest is that the harness holds **neither** of them:
/// `cargo xtask gpu` hashes the bytes it captured and compares the number with
/// the one this file printed, so a wrong belief about the layout here produces a
/// hash that does not match a capture rather than one that agrees with itself.
const fn pixel(x: u32, y: u32) -> [u8; 4] {
    // Sixteen steps of eight, so the whole of each channel's range below 128 is
    // used and no channel is ever zero everywhere. Truncation is intended and
    // cannot lose anything: `x` and `y` are below `WIDTH` and `HEIGHT`.
    #[allow(clippy::cast_possible_truncation)]
    let (red, green, blue) = ((x * 8) as u8, (y * 8) as u8, ((x ^ y) * 8) as u8);
    [blue, green, red, 0]
}

/// The three bytes a screen capture reports for the pixel whose four bytes are
/// `memory`.
///
/// The inverse of [`pixel`]'s layout knowledge, and stated as its own function
/// so that the two are beside each other: a capture reports red, green and blue
/// in that order, and the fourth byte of the pixel is not reported at all.
const fn rgb_at(memory: [u8; 4]) -> [u8; 3] {
    [memory[2], memory[1], memory[0]]
}

/// Hash the pixels of a frame as a screen capture would report them.
///
/// FNV-1a over sixty-four bits, which is chosen for exactly one property: it is
/// short enough to implement identically in two places without either copy being
/// a thing anybody has to check. `cargo xtask gpu` holds the other copy and runs
/// it over the bytes it captured from the emulator.
///
/// **It is not a checksum of the client's buffer**, and the difference is the
/// whole point: what is hashed is the buffer *transformed the way a display
/// would report it*, so a match means the picture on the host is the picture the
/// client owns, and not merely that some bytes arrived.
///
/// A hash and not a byte-for-byte comparison, because the alternative is putting
/// a kilobyte of pixels in a boot log. What it costs is stated rather than
/// hidden: this is not a cryptographic hash and a deliberate collision is
/// constructible. The adversary here is a defect and not an attacker — nothing
/// on the far side of this comparison is chosen by anybody — and the day
/// something adversarial writes to a framebuffer, this is a comparison over the
/// bytes themselves and the boot log is not where it happens.
fn hash_frame(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut at = 0;
    while at + BYTES_PER_PIXEL as usize <= bytes.len() {
        let Some(chunk) = bytes.get(at..at + BYTES_PER_PIXEL as usize) else { break };
        let mut memory = [0u8; 4];
        for (slot, byte) in memory.iter_mut().zip(chunk) {
            *slot = *byte;
        }
        for byte in rgb_at(memory) {
            hash ^= u64::from(byte);
            hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        }
        at += BYTES_PER_PIXEL as usize;
    }
    hash
}

/// How many turns the driver's loop may spend with nothing on either ring.
///
/// Told to the component rather than chosen by it, and it is a **backstop and
/// not the mechanism**: on every half of this demonstration the frame's own
/// client gives up first and posts a stop, so a run that reached this number is
/// a run where the frame stopped serving — a different failure, wanting a
/// different answer. `f_virtio_gpu::routing::stopped::IDLE` is its own outcome
/// for exactly that reason.
///
/// A count and not a duration, because RFC 0004 offers a component no clock and
/// because a count is the same number on every host. A hundred million, which is
/// larger than anything the frame's own bounds allow.
/// Unit: turns.
const IDLE_SPINS: u64 = 100_000_000;

/// The deadline the client's `show` carries.
/// Unit: nanoseconds, monotonic, in the channel's epoch.
///
/// A millisecond, which is outside [`FLOOR_NS`] by two orders of magnitude, so
/// this request is *not* floored. That matters because `cflags::SHORTFALL` is
/// **one bit**: a completion says the request got less than it asked for and not
/// which of the three ways, so the frame cannot tell a class demotion from a
/// floored deadline. This constant is what leaves only one of them possible, and
/// it is an argument rather than a check — the check would need a field on the
/// completion, which is an ABI change under RFC 0011.
const HARD_DEADLINE_NS: u64 = 1_000_000;

/// The floor the driver is told it needs. Unit: nanoseconds.
///
/// Ten microseconds, the same figure both other datapaths route and for the same
/// reason: what it bounds on this boot is nothing, because a component has no
/// clock and the arrival it floors from is zero. It is routed anyway, because a
/// frame that left the field at zero would be telling the driver it needs no
/// time at all — a different claim from *this driver cannot measure the
/// difference*.
const FLOOR_NS: u64 = 10_000;

/// The manifest this datapath routes for, by name.
const DRIVER: &[u8] = b"virtio-gpu";

/// The need in that manifest that names the register pages.
const NEED_MMIO: &[u8] = b"mmio";

/// The need that names the untyped region the driver splits into its queue.
const NEED_QUEUES: &[u8] = b"queues";

/// The rights a component holds over memory it means to hand to a device.
///
/// `GRANT` is the load-bearing one and [`iommu::Grant::map`] argues why: putting
/// a page in a device's domain is a transfer to something the capability system
/// does not mediate.
///
/// `WRITE` is here and **should not need to be**, which is RFC 0051's second gap
/// arriving for the second time and from the other side. A display controller
/// only ever *reads* a client's backing — there is no 2D command that writes one
/// — so this registration is the clearest case in the tree of a set that wants to
/// be device-read-only and has no field to say so. `iommu::Grant::map` derives
/// writability from the capability's own rights, so a client that held its
/// canvas through a read-only `Frame` would get a read-only translation and this
/// datapath would still work. That is the workaround RFC 0051 named as available
/// today, and it is deliberately **not** taken here: taking it would make this
/// datapath prove something about a careful client rather than about the ABI,
/// and the gap would stop being visible in the one place it is easiest to see.
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
    /// A `show` was refused on a half where it had to be served. Carries the
    /// packed refusal.
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
            Self::Transfer(_) => "the driver refused a show that was supposed to be served",
            Self::Leaked => "the demonstration's frames did not all come back",
            Self::NoManifest => "no boot module declares the virtio-gpu component",
            Self::Manifest => {
                "the driver's manifest and this machine disagree about what has to be routed"
            }
            Self::Process(_) => "the driver could not be built as a process",
            Self::Scheduled(_) => {
                "the core the driver was given never took it or never gave it back"
            }
            Self::NoAnswer(_) => "the driver did not answer a completion inside the bound",
            Self::Overdue(_) => "the driver's core did not report finished inside the bound",
        }
    }

    /// The wall-clock bound this refusal is, when it is one.
    ///
    /// Two of these variants are not findings: they are spins that ran out of a
    /// number derived from `tsc_khz`, so they fire for a component that is
    /// wedged and for a runner slower than the number alike, and nothing here
    /// can tell those apart. Printing them under the same sentence as the rest
    /// is how a slow CI machine comes to be read as a datapath defect.
    /// Unit: microseconds.
    #[must_use]
    pub const fn bound(self) -> Option<u64> {
        match self {
            Self::NoAnswer(micros) | Self::Overdue(micros) => Some(micros),
            _ => None,
        }
    }
}

/// What the driver's manifest says it must be given.
///
/// Read out of the record `cargo xtask component` compiled, on every run, rather
/// than repeated as constants here. That is the whole point of the detour:
/// `user/virtio-gpu/manifest.toml` was written before the driver *and before this
/// file*, and a datapath that routed numbers of its own choosing would leave the
/// manifest as decoration.
#[derive(Clone, Copy, Debug)]
pub struct Declared {
    /// The content hash a spawn would name: one hash over the record and the
    /// image together. Unit: none — an identity.
    pub id: ContentId,
    /// Register pages the manifest routes. Unit: pages.
    pub frames: u32,
    /// Untyped bytes it routes for the queue. Unit: bytes.
    pub bytes: u64,
    /// The reservation class the manifest declares, as `f_abi::class` reads it.
    /// Unit: none — a class ordinal.
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
    /// The client fills a registered buffer and submits one `show`. The picture
    /// must reach the host's display.
    Inside,
    /// The identical client with the `show` removed. The pixels are in guest
    /// memory the whole time and nothing may reach the display.
    Blank,
    /// The `show` happens and the driver points the device past what the
    /// registration answered before the address becomes the resource's backing.
    /// The unit must fault it on a read and nothing may appear.
    Escape,
}

impl Half {
    /// The word the boot log and the harness's parameter share.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inside => "inside",
            Self::Blank => "blank",
            Self::Escape => "escape",
        }
    }

    /// Does this half submit a `show`?
    #[must_use]
    pub const fn shows(self) -> bool {
        matches!(self, Self::Inside | Self::Escape)
    }

    /// Must the picture reach the display?
    ///
    /// True on exactly one half, and that is the design. The other two are the
    /// two different ways a picture can fail to appear — nothing was submitted,
    /// and something was submitted and the unit refused where the pixels were to
    /// be read from — and a suite that could not tell those apart would be
    /// claiming one of them while showing the other.
    #[must_use]
    pub const fn expects_picture(self) -> bool {
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
    /// Two and not three: `inside` and `blank` differ in what the *client* does
    /// and not in what the driver does at all, which is what makes the second a
    /// control for the first.
    /// Unit: none — a selector ordinal.
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
    /// Whether the `show` completed without a refusal.
    ///
    /// **Not evidence that anything is on the screen.** It says the display
    /// accepted six commands. What stands in for the picture is
    /// [`Report::display_hash`] and the harness's own capture.
    pub shown: bool,
    /// The hash of the client's pixels, as a screen capture would report them.
    ///
    /// The number `cargo xtask gpu` compares its capture against.
    /// Unit: none — an FNV-1a-64 digest.
    pub display_hash: u64,
    /// Whether the client's canvas still holds the pattern it wrote.
    ///
    /// A display controller reads a backing and never writes one, so this must
    /// be true on every half — including the one where the transfer was refused.
    /// It is the check that a device given an address it should not have did not
    /// also scribble on the address it should.
    pub intact: bool,
    /// What the driver counted, read out of the board rather than out of a
    /// structure in this address space. Unit: see [`driver::Counters`].
    pub counters: driver::Counters,
    /// Which core the driver held. Unit: none — a core index.
    pub cpu: usize,
    /// Whether it ended by `EXIT` rather than by a fault.
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
    /// Why its loop ended, as one of `f_virtio_gpu::routing::stopped`. Zero for a
    /// component that never wrote a report at all. Unit: none — an ordinal.
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
    /// `cap`, `iommu`, `blk` and `net` already are — **for everything except the
    /// picture**, which no code inside this machine can see. That one clause is
    /// the harness's and RFC 0054 argues why it has to be.
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
        // The route RFC 0047 is about, counted on the frame's side.
        if self.asked == 0 {
            return Err("the driver asked the frame for no translation");
        }
        // The counter behind the one thing this device makes possible that
        // neither of the others does: a client's buffer the device holds across
        // four completed chains. A boot where it moved is a boot where the
        // detach failed and the display had to be reset to make *the client owns
        // its buffer* true when it was said.
        if self.counters.halted != 0 {
            return Err("a display command failed while the device held the client's buffer");
        }
        // A display controller reads a backing and never writes one, on every
        // half including the refused one.
        if !self.intact {
            return Err("the client's canvas was written by a device that only ever reads");
        }
        // R08, on this driver as on the other two. The client's `show` is the
        // one entry here that asks for the hard class, and this driver's
        // manifest declares the soft one.
        if self.half.shows() && self.counters.shortfall == 0 {
            return Err("no completion reported the demotion the manifest requires");
        }
        if !self.half.shows() && self.counters.shortfall != 0 {
            return Err("a completion reported a demotion on a half that asked for nothing");
        }

        match self.half {
            Half::Inside => {
                if !self.shown {
                    return Err("the show did not complete");
                }
                if self.counters.shown != 1 {
                    return Err("the driver did not report exactly one frame flushed");
                }
                // Six commands, and the number is brittle in the direction a
                // fixture should be brittle in: create, attach, transfer, set
                // scanout, flush, detach.
                //
                // **It is also the only thing standing behind one clause of
                // E1-B04's exit, and that is worth stating rather than
                // discovering.** The harness's capture proves the client's
                // pixels are in the host's resource; it does not prove
                // `RESOURCE_FLUSH` did anything, because a screen capture makes
                // the emulator refresh its own surface and a scanout shares the
                // resource's image. So a driver that dropped the flush would
                // show the same picture to the same capture and be caught only
                // *here*. The number is therefore not decoration and must not be
                // relaxed into a range: a command dropped from
                // `Driver::sequence` has exactly one check between it and a
                // green run.
                if self.counters.commands != 6 {
                    return Err("the driver did not send the six commands one show is made of");
                }
                if self.counters.declined != 0 {
                    return Err("the display refused a command on the half that must not fail");
                }
                if self.counters.resources != 1 {
                    return Err("the driver did not create exactly one resource");
                }
                if self.counters.escaped != 0 {
                    return Err("the driver pointed the device past a registration's answer");
                }
                if self.faults != 0 {
                    return Err("the remapping unit faulted on the half that must not fault");
                }
                Ok(())
            }
            Half::Blank => {
                if self.shown {
                    return Err("the control half submitted a show");
                }
                if self.counters.shown != 0 || self.counters.commands != 0 {
                    return Err("the control half sent a display command");
                }
                if self.counters.resources != 0 {
                    return Err("the control half created a resource");
                }
                if self.faults != 0 {
                    return Err("the remapping unit faulted with nothing submitted");
                }
                Ok(())
            }
            Half::Escape => {
                // The provocation ran. An isolation proof whose provocation
                // never ran is the same green as a protection that held.
                if self.counters.escaped == 0 {
                    return Err("the driver never pointed the device past what it was answered");
                }
                // **Nothing is asserted about what the display said, and the
                // first version of this arm asserted the opposite and went
                // red.** It required `declined` to have moved, on the reasoning
                // that a virtio-gpu command carries a typed response and a
                // backing the device cannot map should come back as a refusal.
                // This emulator answers `OK`: a translation the remapping unit
                // refuses still produces a mapping — of a bounce buffer holding
                // none of the client's bytes — so the attach succeeds, the
                // transfer copies that buffer, and the flush puts it on the
                // screen. The unit records the fault and the device notices
                // nothing.
                //
                // That is the third time this tree has had to learn the same
                // sentence, and `kernel/src/arch/x86_64/dma.rs` wrote it first:
                // *a completion is evidence the device finished and never
                // evidence that bytes moved.* RFC 0051 records the network
                // driver finding it again from the other direction — a used
                // entry with a length for a receive the unit refused — and this
                // is the display's version, which is the sharpest of the three
                // because a display's completion is a *typed response* and
                // therefore the most convincing thing to believe.
                //
                // So `declined` and `shown` are published and required to be
                // nothing. What stands instead is the unit's own fault record
                // below, at the address the driver invented and on the direction
                // a display reads — and, outside this machine entirely, the
                // capture `cargo xtask gpu` takes of the screen, which must not
                // hold the client's pixels. Neither of those is a device's word.
                // And the unit's own fault-recording registers, which are the
                // one piece of evidence on this boot that neither the component
                // nor the device wrote. Checked *at the address the driver
                // invented* rather than merely counted: a fault somewhere else
                // would mean something other than this provocation was refused,
                // and the count alone cannot tell those apart.
                match self.fault {
                    None => {
                        return Err("the remapping unit recorded no fault for the refused read");
                    }
                    Some(fault) => {
                        if fault.address != self.expected_fault() {
                            return Err(
                                "the unit faulted somewhere other than the address the driver \
                                 invented",
                            );
                        }
                        if !fault.read {
                            return Err(
                                "the unit faulted on a write, so the transaction under test was \
                                 not the display reading its backing",
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

    /// How wide the frame this boot drew is. Unit: pixels.
    #[must_use]
    pub const fn width(&self) -> u32 {
        WIDTH
    }

    /// How tall it is. Unit: pixels.
    #[must_use]
    pub const fn height(&self) -> u32 {
        HEIGHT
    }
}

/// What the frame stands a scheduled driver up with.
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
/// which the driver narrows with `Window::slice`.
///
/// **This is `kernel/src/blk.rs`'s type and `kernel/src/net.rs`'s type, for the
/// third time.** RFC 0051 predicted that a third driver would be the moment the
/// shared half moved out of all of them; RFC 0054 declines to move it and says
/// why, and `OWED_REVERSALS` in xtask carries the deviation so that the day
/// somebody does move it, the build says which documents describe a duplication
/// that is gone.
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
    /// `user/virtio-gpu/manifest.toml` insists on: *a device whose BAR is larger
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
    /// other two datapaths use — so the check that stands between a component's
    /// clients and each other's memory is the same check, on the same table,
    /// with the same refusal, for all three drivers.
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

    // The same finder both other datapaths use, and the **one** place the frame's
    // device discovery had to change for a third driver: a display controller has
    // no transitional PCI device id, because it was defined after the modern
    // transport, and `virtio::route` took one as an ordinary argument.
    // `virtio::VIRTIO_GPU_MODERN` argues the `Option` at length. RFC 0054.
    // SAFETY: the caller's guarantee, passed down.
    let found = unsafe {
        virtio::route(frames, space, features, window, survey, virtio::VIRTIO_GPU_MODERN, None)
    }
    .map_err(Trouble::Device)?;

    // The device has to fit what the manifest declares, and the refusal is in
    // that direction on purpose.
    if found.pages > declared.frames {
        return Err(Trouble::Manifest);
    }

    let before = frames.free_count();
    // What the *unit* keeps, as opposed to what the demonstration spends.
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
    // its domain is freed.
    //
    // **And the display keeps its picture**, which is the one thing about this
    // teardown that is not either of the other two datapaths'. Clearing the
    // bus-master bit and detaching the function stop the device reaching guest
    // memory; neither touches the resource the scanout is made of, because that
    // resource lives on the host's side of the emulator — which is what
    // `TRANSFER_TO_HOST_2D` put it there for. That is why `cargo xtask gpu` can
    // capture the framebuffer after this function has returned, and why
    // `user/virtio-gpu` must not reset the device on its way out.
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
        // of `f_abi::deadline::inherit`, which all three drivers call.
        (routing::at::CLIENT_ADMITTED, u64::from(class::HARD)),
        (routing::at::FLOOR, FLOOR_NS),
        (routing::at::IDLE_SPINS, IDLE_SPINS),
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
    // client that has gone is a core this boot never gets back.
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

    if observed.is_err() {
        match ended.death {
            crate::process::Death::Killed { vector, error, address, rip } => crate::kprintln!(
                "  gpu stalled   the component was killed: vector {vector}, error {error:#x}, \
                 address {address:#018x}, at {rip:#018x}"
            ),
            crate::process::Death::Exited(status) => {
                crate::kprintln!(
                    "  gpu stalled   the component ended by EXIT with status {status}"
                );
            }
            crate::process::Death::Running => {
                crate::kprintln!("  gpu stalled   the component never reported an ending");
            }
        }
        crate::kprintln!(
            "  gpu stalled   it reported outcome {}, having drained {} entr(ies), served {}, \
             refused {}, {} command(s) answered, {} declined",
            reported.outcome,
            reported.drained,
            reported.counters.served,
            reported.counters.refused,
            reported.counters.commands,
            reported.counters.declined,
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
        shown: observed.shown,
        display_hash: observed.display_hash,
        intact: observed.intact,
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
    shown: bool,
    /// Unit: none — an FNV-1a-64 digest.
    display_hash: u64,
    intact: bool,
}

/// The token each of the client's entries carries.
mod token {
    /// The registration that must be refused for want of `GRANT`.
    pub const PROBE: u64 = 1;
    /// The registration that must succeed.
    pub const REGISTER: u64 = 2;
    /// The frame put on the scanout.
    pub const SHOW: u64 = 3;
}

/// The client's whole run: register, draw, show, and look at what is left.
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
    // On this driver the rule is doing something the other two never asked of it.
    // A block transfer and a network transmit hand the device a buffer for the
    // duration of one chain; a display *attaches* one, and goes on holding it
    // across four more chains until the driver detaches it. The `InFlight` here
    // therefore spans a sequence and not a request, and the fact that nothing in
    // `f_ring::buffers` had to change to express that is the strongest single
    // thing E1-B04 says about RFC 0024.
    let mut set = BufferSet::bind(naming, asking.negotiated, page).map_err(Trouble::Channel)?;
    let carved = set.carve::<{ BUFFERS as usize }>().map_err(|_| Trouble::Geometry)?;
    // Destructured rather than indexed, because a submission *moves* an `Idle`
    // and there is nothing to leave behind in an array — which is RFC 0024's
    // typestate doing its job at the one place a reader might reach for an
    // index. `CANVAS` is buffer zero, and the assertion is what keeps that true:
    // a reader who renamed it would move an address the boot log prints and
    // nothing else would notice.
    //
    // The three spares are never submitted. They exist so that *the set* and
    // *the buffer* are different objects, which is what makes an off-by-one in
    // the registration path visible at all.
    const _: () = assert!(CANVAS == 0);
    let [mut canvas, _spare_one, _spare_two, _spare_three] = carved;
    if canvas.len() < FRAME_BYTES as usize {
        return Err(Trouble::Geometry);
    }

    // Poison first, then the pattern over it. The poison is what makes *the
    // buffer was not written* and *the buffer was written* different
    // observations; the pattern is what the display has to show.
    for byte in canvas.bytes_mut().iter_mut() {
        *byte = POISON;
    }
    draw(canvas.bytes_mut())?;
    let Some(drawn) = canvas.bytes().get(..FRAME_BYTES as usize) else {
        return Err(Trouble::Geometry);
    };
    let display_hash = hash_frame(drawn);

    // --- the frame on the scanout -------------------------------------------
    let mut shown = false;
    let canvas = if asking.half.shows() {
        // The hard class, written explicitly, because `Sqe::ZERO` writes
        // `class::BATCH` and a batch entry at a soft-class service is not
        // demoted at all — so a client that left the field alone would never
        // exercise R08 here and the counter beside it would read zero for a
        // reason that has nothing to do with the driver.
        let mut entry = driver::show(token::SHOW, WIDTH, HEIGHT);
        entry.class = f_abi::deadline::pack(class::HARD, 0);
        entry.deadline = HARD_DEADLINE_NS;
        let (lent, _) = canvas.submit(producer, entry).map_err(|_| Trouble::Channel(0))?;
        let answer = supervising.awaited(asking.tsc_khz)?;
        shown = !answer.is_error();
        lent.complete(&answer).map_err(|_| Trouble::Transfer(0))?
    } else {
        canvas
    };

    // --- what is actually in the client's memory ----------------------------
    //
    // Read back rather than trusted, and on every half. A display controller
    // reads a backing and never writes one, so a canvas that changed is a device
    // doing something no command in this protocol asks for — which is exactly the
    // shape of thing an escape provocation might produce and which no counter
    // would show.
    let mut formed = [0u8; FRAME_BYTES as usize];
    draw(&mut formed)?;
    let mut intact = true;
    for index in 0..FRAME_BYTES as usize {
        let (Some(got), Some(want)) = (canvas.bytes().get(index), formed.get(index)) else {
            return Err(Trouble::Geometry);
        };
        if got != want {
            intact = false;
        }
    }
    // And the rest of the page, which no command named at all.
    for index in FRAME_BYTES as usize..canvas.len() {
        let Some(got) = canvas.bytes().get(index) else { return Err(Trouble::Geometry) };
        if *got != POISON {
            intact = false;
        }
    }

    if asking.half.expects_picture() && !shown {
        return Err(Trouble::Transfer(0));
    }

    Ok(Observed { registered_at, refused_without_grant, shown, display_hash, intact })
}

/// Write the frame this boot draws into `into`.
///
/// By hand, in the client, and both of those are decisions. **By hand** because
/// there is nothing in this system that draws, and building one to fill sixteen
/// rows would be building the thing E2 owes rather than the thing E1-B04 owes.
/// **In the client** because what is in a frame is the client's — a driver that
/// chose the pixels would be a driver with a picture of its own.
///
/// # Errors
///
/// [`Trouble::Geometry`] for a buffer shorter than the frame, which the caller
/// has already refused.
fn draw(into: &mut [u8]) -> Result<(), Trouble> {
    let Some(frame) = into.get_mut(..FRAME_BYTES as usize) else { return Err(Trouble::Geometry) };
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let at = ((y * WIDTH + x) * BYTES_PER_PIXEL) as usize;
            let Some(slot) = frame.get_mut(at..at + BYTES_PER_PIXEL as usize) else {
                return Err(Trouble::Geometry);
            };
            slot.copy_from_slice(&pixel(x, y));
        }
    }
    Ok(())
}

/// What the component wrote about itself into the half of its board that is its
/// own.
#[derive(Clone, Copy)]
struct Reported {
    counters: driver::Counters,
    /// Unit: entries.
    drained: u64,
    /// One of `f_virtio_gpu::routing::stopped`. Unit: none — an ordinal.
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
    /// reading a page a ring-3 component writes and R04 applies.
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
        let (Ok(shown), Ok(commands), Ok(declined), Ok(resources), Ok(halted)) = (
            u32::try_from(read(routing::reported::SHOWN)),
            u32::try_from(read(routing::reported::COMMANDS)),
            u32::try_from(read(routing::reported::DECLINED)),
            u32::try_from(read(routing::reported::RESOURCES)),
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
                shown,
                commands,
                declined,
                resources,
                spun: read(routing::reported::SPUN),
                halted,
            },
            drained: read(routing::reported::DRAINED),
            outcome: read(routing::reported::OUTCOME),
        }
    }

    /// What the frame knows about a component that reported nothing it can
    /// believe.
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
    shown: 0,
    commands: 0,
    declined: 0,
    resources: 0,
    spun: 0,
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
