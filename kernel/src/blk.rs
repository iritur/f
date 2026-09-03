// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The block datapath, end to end: a driver outside the frame, a client with a
//! registered buffer, a real device, and a counter that says the bytes went
//! nowhere near either of them.
//!
//! # What this file is and what it is not
//!
//! It is the **supervisor's half** of E1-B02. `user/virtio-blk` is the driver:
//! the transport handshake, the virtqueue, the registration table and the
//! service loop, in a crate that forbids `unsafe` and holds no mapping of any
//! client's memory. What is here is everything a supervisor does around one —
//! find the device, program its domain, route it four register windows and one
//! untyped region, stand a client up on the other end of a ring, and then judge
//! what happened.
//!
//! It is **not** a second implementation of the driver, and that distinction is
//! the one to check first if this file ever looks like it is growing one. There
//! is one body of driver code, in `user/virtio-blk`, and this calls it.
//!
//! # Where the driver's code runs, and what this file is left holding
//!
//! At ring 3, on a core of its own, in its own polling loop. That is RFC 0047
//! and it is what two sentences that used to be here stopped being true of:
//!
//! - *Nothing schedules a component.* E1-B08 landed the mechanism —
//!   `kernel/src/runtime.rs` — and RFC 0047 pointed it at a driver, which needed
//!   three things a runtime did not: more than one page of text, a device's
//!   registers mapped uncached into a component's address space, and its queue
//!   memory mapped whole.
//! - *A component cannot drive a ring, because adopting a mapped channel is
//!   `unsafe`.* RFC 0037 answered that, and answered it with a different
//!   argument from the one RFC 0033 made for a device window — a channel is
//!   shared with a peer that may be hostile and a window is not.
//!
//! What is left here is the supervisor's half, and it is exactly the half a
//! driver may not have: the remapping unit, the domain its device is attached
//! to, the frame allocator, and the *client's* capability table. [`Supervising`]
//! is that list as a type. The one thing a scheduled driver cannot do for
//! itself is turn a client's capability into an address its device may use, so
//! it asks — `f_abi::control::op::DEVICE_MAP`, on its control ring — and this
//! file answers, from a polling loop on the boot processor, out of the same
//! [`iommu::Grant`] it used to pass in as an argument. The check is unchanged:
//! the client's handle, against the client's table, refused without `GRANT`.
//!
//! *Reversal:* a supervisor that is a component. When one exists, the answering
//! below is its work rather than the frame's, and what this file keeps is the
//! device discovery underneath it. E1-B05 owes that, and `CHAOS_GAP` in xtask
//! carries what is still owed as a set rather than as a sentence.
//!
//! # Three halves, and none of them means anything alone
//!
//! `blk=inside` registers the client's buffer, writes a sector, reads it back
//! and requires the bytes to match. It is the positive control, and without it
//! the two refusals below are worthless — the reason `mutate` gives about
//! defects and `dma.rs` about its own two halves: a refusal proves nothing if
//! the same setup also refuses when it should not.
//!
//! `blk=outside` does the same and takes the client's page out of the driver's
//! device domain between the two — RFC 0024's stated case, *the memory is the
//! client's and it is entitled to take it back* — and requires the read to be a
//! **fault** rather than a transfer into memory the driver no longer has. The
//! driver's descriptor is correct throughout; what changed is underneath it.
//!
//! `blk=escape` takes nothing away. The driver resolves the registration and
//! then **adds [`BEYOND`] to the address itself** before writing it into a
//! descriptor, which is arithmetic no type in that crate can prevent — a
//! `Reach` is an address and a length and an address is an integer. The unit
//! must fault it, at the address the driver invented rather than at the one it
//! was answered, and nothing may land.
//!
//! The third is the clause E1-B01's exit could not observe. That exit proved
//! the property at the device with the frame's own adversary one bus over, and
//! wrote down that *the word component in it belongs to E1-B02*. This is that
//! word: the descriptor is written by `user/virtio-blk`, out of its own
//! arithmetic on an address the frame answered its client's registration with.
//!
//! [`Half`] says why `outside` and `escape` are different questions rather than
//! one question run twice, and review is what found that they were being
//! conflated: only `escape` is *a driver reaching outside its grant*, and
//! `outside` is *a grant being taken away under a driver*. The sentence that
//! used to close this paragraph — *what neither shows is that the code doing
//! the reaching runs at ring 3* — is the one RFC 0047 removed. It does now, and
//! the arithmetic that produces the bad descriptor happens in an address space
//! where the only memory it can reach is what its manifest declared.
//!
//! # What the zero-copy counter is worth on *this* boot, exactly
//!
//! `f_virtio_blk::driver::Counters::copies` is zero because there is no type in
//! the driver crate that turns a client's buffer into bytes: a
//! [`Reach`](f_ring::registry::Reach) is an address and a length, a
//! `Region` is the component's own memory, and the one function in that crate
//! which moves bytes takes the tally it moves as an argument and is not called
//! from the data path.
//!
//! So node 23 is a **structural property published as a number**, and not a
//! tally of copies some code path performed and this boot happened to find at
//! zero — those two read identically and only one of them is what is being
//! claimed. The mechanism behind it is `cargo xtask lint-datapath`, which
//! requires that crate to define exactly one function that moves bytes, to call
//! it exactly once, and to call it from `provoke_copy` and nowhere else. Node
//! 24 is the other half: it says the counting works at all.
//!
//! **The second enforcement has arrived, and it is what closes this paragraph
//! rather than lengthening it.** What used to be missing was an address space:
//! the driver executed in the frame, where the direct map covers all of
//! physical memory, so the property rested on the crate's own types and on a
//! source check that refused any line minting an accessor over a bare address.
//! It runs at ring 3 now. The pages it can reach are its text, its stack, its
//! two rings, its board, its device's registers and its own queue memory, and
//! nothing else in the machine is mapped for it — so an address it invents is a
//! page fault rather than somebody's bytes. RFC 0047 retires the source check
//! and says why the page tables are the stronger statement.
//!
//! The number itself no longer comes from a counter in this address space: the
//! component writes its own tallies into the half of its board that is its own,
//! and this file reads them. RFC 0013's *read, never delivered*. A component
//! that never ran writes nothing, which is why [`Reported`] refuses a board
//! with no magic in it rather than reading a page of zeroes as six zeroes.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, rights};
use f_abi::control;
use f_abi::manifest::{ContentId, Record, route};
use f_abi::{ABI_VERSION, Negotiated, error, feature};
use f_ring::device::Window;
use f_ring::registry::{Domains, registration};
use f_ring::{BufferSet, Collector, Consumer, Fixed, Mapping, Poster, Producer};
use f_virtio_blk::driver;
use f_virtio_blk::routing;
use f_virtio_blk::transport::SECTOR_BYTES;

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging::{self, AddressSpace, Features};
use crate::arch::x86_64::pci::{self, Bdf, Survey};
use crate::arch::x86_64::virtio;
use crate::arch::x86_64::vtd::{Fault, Unit};
use crate::cap::Table;
use crate::component;
use crate::iommu;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};

/// The one address a driver component holds as a constant, agreed.
///
/// `f_virtio_blk::routing::AT` is written down in the component and
/// `kernel::process::BLK_BOARD` in the frame, and they are linked separately —
/// the component is a flat image built by a different invocation of the
/// compiler. There is nothing to share a constant through, which is the same
/// position `user/init/link.ld` and `INIT_TEXT` are in. What is different is
/// that the kernel links *both* definitions, so the agreement can be a check
/// rather than a comment, and this is that check: a build where the two
/// disagree does not link, instead of booting into a page fault at the
/// component's first read.
const _: () = assert!(
    crate::process::BLK_BOARD == routing::AT,
    "the frame and the driver disagree about where the routing page is"
);

/// Entries on the client's data ring.
///
/// Sixteen, which is what fits one frame beside its completion ring and its
/// index ring — the same number and the same reason as a control ring's. The
/// manifest declares two hundred and fifty-six, which is what a real client
/// gets when a component pays for its own channel; this demonstration submits
/// four entries in total and a deeper ring would be a larger fixture proving
/// the same thing.
const ENTRIES: u32 = 16;

/// Buffers the client's registered set holds.
///
/// Two: one to write from and one to read into. They have to be different
/// buffers, and that is the whole design of the check — reading back into the
/// buffer that was written from would compare memory against itself and pass on
/// a device that did nothing at all.
const BUFFERS: u32 = 2;

/// Bytes moved in each direction. Unit: bytes.
///
/// One sector. The claim is about the path rather than about throughput —
/// `E1-P10` is where a number attaches to it — and one sector is the smallest
/// transfer that is a transfer.
const TRANSFER: u32 = SECTOR_BYTES;

/// The byte the sink is filled with before the read.
///
/// Not a byte the pattern below can produce and not a byte the disk holds, so a
/// sink still full of it is a sink nothing wrote. The same trick `dma.rs` uses
/// and for the same reason: *the transfer was refused* and *the transfer
/// happened and wrote nothing* are different claims, and only one of them is an
/// exit criterion.
const POISON: u8 = 0xA5;

/// Where on the disk the demonstration works. Unit: bytes.
const AT: u64 = 0;

/// The manifest this datapath routes for, by name.
///
/// A name and not an index, because the loader's module order is a contract
/// about `user/init` and about nothing else — `component::modules` reads
/// component files by magic rather than by position for exactly that reason.
/// The bytes are what `manifest.toml`'s `name` compiles to.
const DRIVER: &[u8] = b"virtio-blk";

/// The need in that manifest that names the register pages.
const NEED_MMIO: &[u8] = b"mmio";

/// The need that names the untyped region the driver splits into its queues.
const NEED_QUEUES: &[u8] = b"queues";

/// The rights a component holds over memory it means to hand to a device.
///
/// `GRANT` is the load-bearing one and [`iommu::Grant::map`] argues why: putting
/// a page in a device's domain is a transfer to something the capability system
/// does not mediate. `WRITE` because a block read is the device writing.
const GRANTABLE: u8 = rights::READ | rights::WRITE | rights::GRANT;

/// Why the demonstration could not be run.
///
/// None of these is the result being looked for. A datapath that could not be
/// set up is not a datapath that was exercised, and the boot path says so
/// rather than reporting a pass.
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
    /// The frame *gave* a translation for a capability carrying no right to
    /// hand memory to a device.
    ///
    /// The one variant that is a failure of the thing being tested rather than
    /// of the test: a client that could put memory in a driver's domain without
    /// `rights::GRANT` has an authority the capability system never issued.
    NotRefused,
    /// A channel could not be laid out, or one of its four ends could not bind.
    Channel(i32),
    /// The client's region does not divide into the buffers the registration
    /// declared, which is this file's arithmetic and not a peer's.
    Geometry,
    /// A registration was refused. Carries the packed refusal.
    Registration(i32),
    /// A transfer was refused. Carries the packed refusal.
    Transfer(i32),
    /// The frame's own count of what it took and what it gave back disagreed.
    Leaked,
    /// No component file among the boot modules declares this driver.
    NoManifest,
    /// The manifest and the machine disagree: a need this datapath has to
    /// route is missing from the record, declares less than the driver's own
    /// layout needs, or declares fewer register pages than the device
    /// describes.
    ///
    /// A refusal rather than a shrug, and the direction matters: the manifest
    /// is what a reviewer reads and the device is what the machine has, so a
    /// device that does not fit is *a different device and a different
    /// manifest* — `user/virtio-blk/manifest.toml` says so in as many words —
    /// and not a number to quietly enlarge here.
    Manifest,
    /// The driver could not be built as a process, carrying which step.
    Process(crate::process::Error),
    /// The core the driver was given never took it, or never gave it back.
    ///
    /// Carries the core. A machine with one core reaches this too, and
    /// deliberately: a driver and its client cannot be the same core, because
    /// the client would be inside the driver.
    Scheduled(usize),
    /// The driver did not answer a completion inside the frame's own bound.
    /// Carries that bound. Unit: microseconds.
    NoAnswer(u64),
    /// The driver's core was still holding its job when the frame's bound
    /// passed. Carries that bound. Unit: microseconds.
    ///
    /// Apart from [`Trouble::Scheduled`] on purpose, and the difference is the
    /// only thing standing between a slow runner and a datapath defect: a core
    /// that went back to waiting answered the wrong thing and is a finding,
    /// while a bound that passed is a wall-clock number scaled off this
    /// machine's timestamp counter and fires for a wedge and for a slow machine
    /// alike. `smp::NotJoined` is where the two are told apart, and
    /// [`Trouble::bound`] is what the boot log renders them by.
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
            Self::Transfer(_) => "the driver refused a transfer that was supposed to be served",
            Self::Leaked => "the demonstration's frames did not all come back",
            Self::NoManifest => "no boot module declares the virtio-blk component",
            Self::Manifest => {
                "the driver's manifest and this machine disagree about what has to be routed"
            }
            Self::Process(_) => "the driver could not be built as a process",
            Self::Scheduled(_) => {
                "the core the driver was given never took it or never gave it \
                                   back"
            }
            Self::NoAnswer(_) => "the driver did not answer a completion inside the bound",
            Self::Overdue(_) => "the driver's core did not report finished inside the bound",
        }
    }

    /// The wall-clock bound this refusal is, when it is one.
    ///
    /// # Why the boot log asks
    ///
    /// Because two of these variants are not findings. Every other arm is
    /// something the frame *observed* going wrong — a refusal, a fault, a
    /// mismatch — and a red line on it means a protection fired. These two are
    /// spins that ran out of a number derived from `tsc_khz`, so they fire for
    /// a component that is wedged and for a runner slower than the number, and
    /// nothing here can tell those apart. Printing them under the same sentence
    /// as the rest is how a slow CI machine comes to be read as a datapath
    /// defect, and how a real wedge comes to be dismissed as one.
    ///
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
/// Read out of the record `cargo xtask component` compiled, on every run,
/// rather than repeated as constants here. That is the whole point of the
/// detour: `user/virtio-blk/manifest.toml` was written before the driver *and
/// before this file*, and a datapath that routed numbers of its own choosing
/// would leave the manifest as decoration — a document describing a component
/// nobody had checked against it.
#[derive(Clone, Copy, Debug)]
pub struct Declared {
    /// The content hash a spawn would name: one hash over the record and the
    /// image together. Unit: none — an identity.
    pub id: ContentId,
    /// Register pages the manifest routes. Unit: pages.
    pub frames: u32,
    /// Untyped bytes it routes for the queues. Unit: bytes.
    pub bytes: u64,
    /// The component's own image, out of the same component file.
    ///
    /// Read here rather than found again later, because it is the same
    /// module: a datapath that read a *manifest* from one place and an *image*
    /// from another would be a datapath whose content hash named neither.
    pub image: &'static [u8],
}

/// Find the driver's component file and read what it declares.
///
/// # Errors
///
/// [`Trouble::NoManifest`] when no module carries it, [`Trouble::Manifest`]
/// when the record is not one this build can read or does not declare both
/// needs this datapath routes.
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
            // powerbox is not one this datapath supplies. The manifest's
            // `powerbox` endpoint is exactly that, and skipping it here is the
            // same rule `component::check_needs` applies.
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
        return Ok(Declared { id: ContentId::of(module), frames, bytes, image });
    }
    Err(Trouble::NoManifest)
}

/// How far past a registration's answer the `escape` half points the device.
///
/// One frame, so the address lands in the page *after* the one the client
/// registered — outside the driver's domain by a whole page rather than by a
/// byte, because an address that straddles the end of a grant is a second
/// question (what a unit does with a partially translated transaction) and this
/// run is asking the first. Unit: bytes.
pub const BEYOND: u64 = FRAME_SIZE;

/// Which of the three experiments a run is.
///
/// # Why three and not two
///
/// `E1-B01`'s exit says *a driver component provably cannot address memory
/// outside its grant*, and there are two different things a boot can show about
/// that sentence. [`Half::Outside`] shows the frame's half: a translation
/// withdrawn under a live registration makes an in-flight transfer fault
/// instead of landing, which is RFC 0024's reclaim and which the driver
/// experiences without ever doing anything wrong. [`Half::Escape`] shows the
/// driver's: the component's own arithmetic produces an address it was never
/// granted and hands it to the device.
///
/// Review found that gap and it is worth keeping named. Only the second is *a
/// driver reaching outside its grant*; the first is *a grant being taken away
/// under a driver*. Both must be refused, they are refused by the same unit for
/// different reasons, and a suite with only one of them would be claiming the
/// other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Half {
    /// The client's page stays in the driver's domain and the driver names
    /// exactly the address the registration answered. The sector must come
    /// back, byte for byte. The positive control, without which neither refusal
    /// below proves anything.
    Inside,
    /// The client takes its page back between the write and the read. The
    /// driver's descriptor is unchanged and correct; the translation under it
    /// is gone. RFC 0024.
    Outside,
    /// The driver adds [`BEYOND`] to the address the registration answered
    /// before writing it into a descriptor. Nothing is taken away from it; it
    /// reaches for memory it never held.
    Escape,
}

impl Half {
    /// The word the boot log and the harness's parameter share.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Inside => "inside",
            Self::Outside => "outside",
            Self::Escape => "escape",
        }
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
    /// Two and not three: `inside` and `outside` differ in what the *frame*
    /// does between the two transfers and not in what the driver does at all,
    /// which is the point of that half — the driver's descriptor is correct
    /// throughout. `escape` is a different life because it is a different code
    /// path in the component, and a selector that could not tell them apart
    /// would be a provocation the boot could not ask for.
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
    /// What the device says it holds. Unit: sectors of [`SECTOR_BYTES`].
    pub capacity: u64,
    /// Where the device addresses the client's registered set.
    /// Unit: bytes, in the device's address space.
    pub registered_at: u64,
    /// Whether a registration of a capability carrying no `GRANT` was refused.
    pub refused_without_grant: bool,
    /// Whether the write completed without a refusal.
    pub wrote: bool,
    /// Whether the read completed without a refusal.
    pub read: bool,
    /// Whether every byte the client read back is a byte it wrote.
    ///
    /// Read out of the client's own buffer rather than inferred from a
    /// completion, because `dma.rs` records this emulator answering a refused
    /// transfer with a *successful* status: a completion is evidence the device
    /// finished and never evidence that bytes moved.
    pub matched: bool,
    /// How many bytes of the sink still hold [`POISON`]. Unit: bytes.
    ///
    /// The whole transfer, on a run where nothing landed. Published rather than
    /// reduced to a boolean because *nothing landed* and *some of it landed*
    /// are different failures and only one of them is the expected result of
    /// the refused half.
    pub untouched: u32,
    /// What the driver counted, read out of the board rather than out of a
    /// structure in this address space. Unit: see [`driver::Counters`].
    pub counters: driver::Counters,
    /// Which core the driver held. Unit: none — a core index.
    pub cpu: usize,
    /// Whether it ended by `EXIT` rather than by a fault.
    ///
    /// The frame's own reading, taken from `process::reap` rather than from
    /// anything the component wrote: a driver that faulted mid-run could write
    /// nothing afterwards, and one that scribbled its own board could write
    /// anything.
    pub exited: bool,
    /// How many entries it took off its data ring. Unit: entries.
    ///
    /// Beside [`driver::Counters::served`] rather than derived from it, because
    /// they are two claims: one is what the component's executor counted and
    /// the other is what its loop saw arrive.
    pub drained: u64,
    /// How many operations the driver submitted on its control ring and this
    /// frame answered.
    ///
    /// Counted by the frame and not reported by the component, which is what
    /// makes it evidence: it is the one number here a component could not
    /// produce if the route it names had never been used.
    /// Unit: operations.
    pub asked: u32,
    /// Why its loop ended, as one of `f_virtio_blk::routing::stopped`.
    /// Zero for a component that never wrote a report at all.
    /// Unit: none — an ordinal.
    pub stopped: u64,
    /// The first fault the remapping unit recorded, if it recorded one.
    pub fault: Option<Fault>,
    /// How many it recorded. Unit: transactions.
    pub faults: u32,
}

impl Report {
    /// Whether the run produced what the half it was asked for requires.
    ///
    /// The verdict is the kernel's rather than the harness's, exactly as
    /// `user`, `cap` and `iommu` already are: this knows which half it was
    /// asked for, what the unit recorded and what is in the client's buffer
    /// afterwards, and a harness reading an exit code could not tell a refused
    /// transfer from a device that never answered.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold. Every one of them fails the boot: a
    /// protection that did not fire is not a smaller result than a fault, it is
    /// the opposite result.
    pub const fn verdict(&self) -> Result<(), &'static str> {
        // The component ran and ended the way a component ends, checked before
        // anything it said about itself. Both halves are the *frame's* reading:
        // a driver that faulted before its loop started would write nothing
        // into its board, and every counter below would read zero — including
        // `copies`, which is the number this subsystem publishes as a property.
        // A zero arrived at by never running and a zero arrived at by a
        // structural impossibility are the same zero, and this is the line that
        // tells them apart.
        if !self.exited {
            return Err("the driver did not end by EXIT, so nothing it reported is its own");
        }
        if self.stopped != routing::stopped::TOLD {
            return Err("the driver's loop ended for a reason of its own rather than because its \
                        supervisor told it to, so the run is shorter than the one asked for");
        }
        if self.drained == 0 {
            return Err("the driver took nothing off its data ring, so its client's entries \
                        crossed no boundary");
        }
        // Two, because two registrations were submitted — one refused for want
        // of `GRANT` and one served — and each of them is a `DEVICE_MAP` the
        // driver could not answer for itself. A run with fewer is a run in
        // which the component got its translations from somewhere that is not
        // the frame, which is the arrangement RFC 0047 refused by name.
        if self.asked < 2 {
            return Err("the driver did not ask the frame for the translations its \
                        registrations needed, so the route this run is about was not used");
        }
        // The half that is true on both runs, checked next, because it is
        // about this component rather than about the hardware — and a datapath
        // that copied would be wrong whichever way the second half went.
        if self.counters.copies != 0 {
            return Err("the driver copied bytes on the data path");
        }
        if self.counters.provoked == 0 {
            return Err("nothing moved the copy counter, so its zero measures nothing");
        }
        if !self.refused_without_grant {
            return Err("a capability with no right to grant was given a device translation");
        }
        if !self.wrote {
            return Err("the write was refused, so the read proves nothing either way");
        }

        // The provocation has to have run. An `escape` half whose driver never
        // built the bad descriptor would reach the fault checks below with
        // nothing to have faulted, and would report a protection holding when
        // what actually happened is that nothing tried it — the same defect
        // `provoked` takes out of the copy counter, one property over.
        if matches!(self.half, Half::Escape) != (self.counters.escaped != 0) {
            return Err("the escape provocation did not run, or ran on a half that is not it");
        }

        if matches!(self.half, Half::Inside) {
            if !self.read {
                return Err("the read was refused over memory the driver's domain translates");
            }
            if !self.matched {
                return Err("the device completed a granted transfer and the bytes did not land");
            }
            if self.faults != 0 {
                return Err("the device faulted on memory it was granted");
            }
            if self.untouched != 0 {
                return Err(
                    "part of a granted transfer did not land, which the byte comparison                             should already have caught",
                );
            }
            return Ok(());
        }

        if self.faults == 0 {
            return Err("the device addressed memory outside the driver's grant and the unit \
                        recorded nothing");
        }
        // Which address faulted, and the two refused halves disagree about it on
        // purpose: `outside` faults on the very page the registration answered,
        // because that page's translation is what was withdrawn, and `escape`
        // faults a whole frame past it, because that is where the driver's own
        // arithmetic pointed. A run that faulted at the wrong one refused the
        // wrong thing, and would otherwise pass.
        match self.fault {
            Some(fault) if fault.address == self.expected_fault() => {}
            Some(_) => {
                return Err("the unit faulted, and not at the address this half points the device");
            }
            None => return Err("a fault was counted and none was recorded"),
        }
        if self.matched {
            return Err("the unit recorded a fault and the transfer landed anyway, which is a \
                        corruption");
        }
        if self.untouched != TRANSFER {
            return Err("part of a refused transfer landed, which is a partial corruption");
        }
        Ok(())
    }

    /// Where this half's refused transaction must have faulted.
    /// Unit: bytes, in the device's address space.
    #[must_use]
    pub const fn expected_fault(&self) -> u64 {
        self.registered_at.wrapping_add(self.half.beyond())
    }
}

/// Run the datapath once.
///
/// `half` decides the whole experiment, the way `inside` does in `dma.rs`. The
/// difference from `dma.rs` is that there are three of them and only two are
/// refusals: [`Half::Inside`] is the positive control, [`Half::Outside`] takes
/// the client's page back under a correct descriptor, and [`Half::Escape`] has
/// the driver point past what it was answered. [`Half`] argues why the last two
/// are different questions.
///
/// # Errors
///
/// [`Trouble`], every variant of which means the datapath did not run.
///
/// # Safety
///
/// Call on the boot processor with the kernel's address space in `CR3`,
/// `frames` rebound onto its direct map, `unit` enabled, and nothing else in
/// this kernel driving the device this finds.
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
    // sized from it. A datapath that allocated first and read the declaration
    // afterwards would be a datapath whose numbers were its own.
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let declared = unsafe { declared(boot) }?;

    // SAFETY: the caller's guarantee, passed down.
    let found = unsafe {
        virtio::route(
            frames,
            space,
            features,
            window,
            survey,
            virtio::VIRTIO_BLK_MODERN,
            virtio::VIRTIO_BLK_TRANSITIONAL,
        )
    }
    .map_err(Trouble::Device)?;

    // The device has to fit what the manifest declares, and the refusal is in
    // that direction on purpose. `user/virtio-blk/manifest.toml`: *four pages is
    // that layout; a device whose BAR is larger is a different device and a
    // different manifest, not a bigger number.*
    if found.pages > declared.frames {
        return Err(Trouble::Manifest);
    }

    let before = frames.free_count();
    // What the *unit* keeps, as opposed to what the demonstration spends. A
    // bus's context table is the unit's for the life of the machine —
    // `Unit::detach` says why it is not freed, and it is right — so a leak
    // check that compared the free count alone would report the first attach on
    // a bus as a leak, every time, and be ignored within a week.
    let kept = unit.tables().len();
    // A domain of the component's own, before anything is allocated for it: a
    // driver with no domain is a driver whose device addresses physical memory,
    // and the order here is what makes that impossible rather than unlikely.
    // SAFETY: the caller's guarantee that frames are addressable.
    let mut domain = unsafe { unit.domain(frames) }.map_err(Trouble::Unit)?;

    // Everything the run is made of. The driver's region is the untyped need
    // its manifest declares — one allocation, split by the driver — and the
    // client's page and the channel are the client's.
    //
    // The order is derived from the declaration rather than written here, so a
    // manifest that changed its `bytes` would change what this routes rather
    // than disagreeing with it silently. A declaration the driver's own layout
    // does not fit in is refused: the manifest is the reviewable statement of
    // what a component is made of, and a driver quietly using more than it
    // declared is the thing that statement exists to prevent.
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
    // its domain is freed. This ordering is what makes the free safe rather
    // than tidy: a device left walking tables the allocator has handed to
    // somebody else is the corruption this whole task is about, arrived at
    // through the teardown.
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
    // Everything the demonstration took, back where it started — with the
    // unit's own retained tables taken out, because those are not the
    // demonstration's to give back. Two numbers rather than a tolerance: a
    // check with slack in it is a check that stops noticing the first frame.
    let retained = unit.tables().len().saturating_sub(kept) as u64;
    if frames.free_count().saturating_add(retained) != before {
        return Err(Trouble::Leaked);
    }
    Ok(report)
}

/// What the frame stands a scheduled driver up with.
///
/// Separate from the frames [`run`] is handed because these are not
/// allocations: they are what the boot path knows and this file cannot find out
/// — which core the topology left free, what the clocks came out at, and where
/// the state tree was published. The same bundle `runtime::demonstrate` takes as
/// loose arguments, given a name here because a driver takes more of them.
#[derive(Clone, Copy)]
pub struct Scheduling {
    /// The core the driver is given. Unit: none — a core index.
    pub cpu: usize,
    /// The rate that core arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks it asks for. A bound rather than a schedule: the client's
    /// work is what ends the run. Unit: timer ticks.
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

/// How long the frame waits for one completion from a driver it is a client of.
///
/// Five seconds, the same bound `main::run_one` and `runtime::demonstrate` use
/// and for the same reason: it is the answer to a component that is wedged
/// rather than a schedule for one that is working. Generous under an emulator
/// that compiles each block of guest code the first time it reaches it, which
/// is what the first transfer of a run pays for.
/// Unit: microseconds.
const ANSWER_MICROS: u64 = 5_000_000;

/// How long it waits for the core afterwards. Unit: microseconds.
const EXIT_MICROS: u64 = 5_000_000;

/// The register window as a *component* sees it: one base and four offsets.
///
/// # Why this is computed rather than routed structure by structure
///
/// Because a component may not be told four unrelated addresses. A modern
/// virtio transport publishes its four structures inside one base-address
/// register, and what the manifest declares is *four register frames* — one
/// window, whole, which the driver narrows with `Window::slice`. Narrowing only
/// ever goes inwards, so a driver that got an offset wrong reads its own
/// registers wrongly and cannot read anybody else's; four separate mappings
/// would have given it four chances to be handed something it did not declare.
///
/// The span is taken from the pages the structures actually fall in rather than
/// assumed to start at the register's own base, because a device that put its
/// common configuration at a non-zero offset is a device this has to route and
/// not one it may refuse.
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
    /// [`Trouble::Manifest`] for a span wider than the manifest declares or
    /// than the driver's address space reserves — which is the direction
    /// `user/virtio-blk/manifest.toml` insists on: *a device whose window is
    /// larger is a different device and a different manifest, not a bigger
    /// number.*
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
    ///
    /// `Structure::at` is where the *frame* reads it, which is the physical
    /// address plus the direct device window's offset. The component is mapped
    /// the physical page, so the offset comes back off here — and a value it
    /// cannot come off is a structure this build did not map through that
    /// window, which is refused rather than wrapped.
    fn physical(structure: &virtio::Structure) -> Result<u64, Trouble> {
        structure.at.checked_sub(paging::DEVICE_OFFSET).ok_or(Trouble::Manifest)
    }
}

/// The frame's half of a scheduled driver's run: the client's ring, the
/// driver's control ring, and the authority behind both.
///
/// # Why one struct and not six arguments
///
/// Because the six belong together and are used together at every polling
/// point. What is here is exactly what a *supervisor* holds and a driver does
/// not: the remapping unit, the domain the device is attached to, the
/// allocator, and the client's capability table. The driver holds none of them
/// and that is the whole architecture — RFC 0047 — so a type that names them as
/// one thing is the type that says so.
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
    /// request is resolved against this one, which is what makes a driver
    /// unable to grant itself anything: it is asking about somebody else's
    /// capability, and the answer is somebody else's rights.
    table: &'a Table,
    /// The device address the last translation answered.
    ///
    /// Kept here because it is the frame's knowledge and the client's need:
    /// nothing in the completion a *client* reaps carries an address — RFC 0024
    /// — so a client that needs to know where the device sees its buffer, in
    /// order to say where a refused transaction should have faulted, asks the
    /// frame that answered it. A client on the far side of a boundary could not
    /// ask this and would not be entitled to; this one is the frame.
    /// Unit: bytes, in the device's address space.
    answered_at: u64,
    /// How many operations this has answered on the driver's control ring.
    ///
    /// **The frame's own evidence that the driver asked**, and it is what makes
    /// RFC 0047's third clause a measurement rather than a design note: a build
    /// in which the translation route had quietly stopped being used — because
    /// somebody put the answers somewhere the component could read them, which
    /// is the alternative that RFC rejects by name — would publish zero here
    /// and fail the verdict. Counted on this side of the boundary, because the
    /// other side's tally is the other side's.
    /// Unit: operations.
    answered: u32,
}

impl Supervising<'_, '_> {
    /// Where the last translation this served put the memory it was asked
    /// about. Unit: bytes, in the device's address space.
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
    /// [`Trouble::Channel`] for a ring that stopped validating — a driver that
    /// scribbled its own control ring, which RFC 0008 treats as a component
    /// that has stopped speaking.
    fn serve(&mut self) -> Result<u32, Trouble> {
        let mut answered = 0;
        loop {
            let Some(entry) = self.asks.pop().map_err(|_| Trouble::Channel(0))? else {
                return Ok(answered);
            };
            // Room before the entry is acted on, because an operation performed
            // and then not answered is a driver waiting forever for a reply
            // that was dropped on the floor — and for a translation that would
            // be a device left holding a mapping nobody knows about.
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
    /// R04 at the bottom: an opcode this build does not implement is refused
    /// and never ignored. The two it does implement are the ones a driver
    /// cannot perform for itself, and both go through the same
    /// [`iommu::Grant`] the frame used when it called the driver's code
    /// directly — so the check that stands between a component's clients and
    /// each other's memory is the same check, on the same table, with the same
    /// refusal.
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

    /// Take the client's next completion, serving the driver until it arrives.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoAnswer`] for a driver that did not answer inside
    /// [`ANSWER_MICROS`] — a wedge, or a machine slower than that bound, and
    /// this cannot tell those apart, which is what [`Trouble::bound`] exists to
    /// say in the boot log; otherwise whatever [`Supervising::serve`]
    /// refuses.
    fn awaited(&mut self, tsc_khz: u64) -> Result<f_abi::Cqe, Trouble> {
        let deadline = crate::smp::deadline_after(tsc_khz, ANSWER_MICROS);
        loop {
            self.serve()?;
            if let Some(answer) = self.reaper.take().map_err(|_| Trouble::Channel(0))? {
                return Ok(answer);
            }
            if crate::smp::past(deadline) {
                return Err(Trouble::NoAnswer(ANSWER_MICROS));
            }
            core::hint::spin_loop();
        }
    }

    /// Take a translation away, on the frame's own initiative.
    ///
    /// The `outside` half, and it is deliberately not an operation the driver
    /// asked for: RFC 0024 says *the memory is the client's and it is entitled
    /// to take it back*, so what happens here happens under a driver that holds
    /// a live registration and is doing nothing wrong.
    fn withdraw(&mut self, cap: u32, address: u64, len: u32) {
        let mut asking = iommu::Grant {
            unit: &mut *self.unit,
            domain: &mut *self.domain,
            frames: &mut *self.frames,
            table: self.table,
        };
        asking.unmap(cap, address, len);
    }

    /// Tell the driver to stop.
    ///
    /// RFC 0008's stop, as the one notice this run posts: a component ends
    /// because its supervisor said so, on the ring, drained at the same polling
    /// point as everything else.
    ///
    /// # Errors
    ///
    /// [`Trouble::Channel`] for a control ring with no room left, which is a
    /// driver that stopped draining.
    fn stop(&self) -> Result<(), Trouble> {
        self.answers
            .post(control::entry(control::notice::STOP, 0, 0, 0))
            .map_err(|_| Trouble::Channel(0))
    }
}

/// The datapath proper, with every allocation already made.
///
/// Split out so the teardown in [`demonstrate`] runs on every path, including
/// the ones that refuse. A leak on the path where something else went wrong is
/// a leak nobody ever sees, because the boot that took that path fails for the
/// other reason and the free count is never compared.
///
/// # Safety
///
/// As [`demonstrate`], and every frame must be one the caller allocated for
/// this.
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
    // Two, and the split is the authority story rather than bookkeeping. The
    // driver's table holds the region its manifest declares; the client's holds
    // the page it is about to register. A registration resolves the *client's*
    // handle against the *client's* table and maps it into the *driver's*
    // domain, which is exactly what `Domains::map` means and what a supervisor
    // arranges. One table holding both would have made every check below pass
    // for a reason nobody chose.
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
    // the point: two handles naming the same bytes, separated by authority
    // alone.
    let ungrantable_cap = client_table
        .grant(CapType::Frame, rights::READ | rights::WRITE, owned.addr(), owned.bytes())
        .map(|handle| handle.bits())
        .map_err(|_| Trouble::Authority)?;

    // --- the driver's own grant ---------------------------------------------
    //
    // Put in the domain by the *spawn* and not by the driver, which is why this
    // is here rather than inside the component: a component's declared needs
    // are the supervisor's to deliver, and a driver that mapped its own queue
    // would be a driver deciding what it was granted. It is also why the
    // component is *told* the device address of its queues rather than asking
    // for it — `f_virtio_blk::routing`.
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
    // mastering, so the device cannot issue a transaction until there is a
    // domain to translate it.
    // SAFETY: the caller's guarantee, and `bdf` is the function whose registers
    // are about to be driven.
    unsafe { unit.attach(frames, found.bdf, domain) }.map_err(Trouble::Unit)?;
    // SAFETY: `found.config` is the function's mapped configuration space.
    unsafe { pci::command_set(found.config, pci::COMMAND_BUS_MASTER) };

    // --- the component ------------------------------------------------------
    //
    // Built the way `runtime::demonstrate` builds a runtime, and given three
    // things a runtime is not: more than one page of text, its device's
    // registers mapped uncached, and its queue memory mapped whole. RFC 0047.
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
    // SAFETY: the caller's guarantee, passed down; `registers.base` is the
    // first page of a device window this boot mapped and nothing else is
    // driving, `granted` and `wire` are frames this call's caller allocated and
    // holds, and `cpu` is a core that is up and idle.
    let (prepared, pages) =
        unsafe { crate::process::prepare_driver(frames, setup.space, setup.features, plan) }
            .map_err(Trouble::Process)?;

    // --- the two rings ------------------------------------------------------
    //
    // The frame is the grantor, so the frame writes both headers and the
    // component adopts them and believes nothing — `f_ring::adopt`, RFC 0037 —
    // which is exactly what it would do if the peer were hostile.
    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Authority)?;
    let at = frames.virt(wire);
    // SAFETY: `wire` was allocated zeroed by the caller, is frame-aligned —
    // stronger than the cache-line alignment the layout asks for — and is
    // `FRAME_SIZE` bytes with no pointer into it held anywhere else.
    // Written and then let go of: the frame is the grantor and writes the
    // header, and the *server* end over these bytes is the component's. Binding
    // it here as well would be the frame holding an end of a channel it does
    // not serve — which is what it did while the driver's code ran in the
    // frame, and what stopped being true at RFC 0047.
    let _ = unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // The client's end. The server's end is the component's, at ring 3, and
    // that is the whole change: two ends of one region on two sides of a
    // privilege boundary rather than two ends in one address space.
    // SAFETY: as above; two ends over one region is what a channel is, and
    // every accessor hands out atomics and `UnsafeCell`s rather than references.
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
    // zero rather than a plausible layout. Nothing here races — the core is
    // idle until the line below — and the order is kept anyway, because the day
    // something does race is the day nobody remembers this was safe.
    board.write64(routing::at::MAGIC, routing::MAGIC).map_err(Trouble::Channel)?;

    // --- the driver runs -----------------------------------------------------
    // SAFETY: `cpu` reports ready, everything `process::execute` depends on was
    // put in its shards by `prepare_driver`, and this core has interrupts
    // enabled — which `run_on`'s contract requires so that a shootdown can be
    // answered.
    unsafe { crate::smp::start_on(setup.scheduling.cpu) }.map_err(Trouble::Scheduled)?;

    let asks = Consumer::new(control.channel()).ok_or(Trouble::Channel(0))?;
    let answers = Poster::new(control.completions()).ok_or(Trouble::Channel(0))?;
    let reaper = Collector::new(client_end.completions()).ok_or(Trouble::Channel(0))?;
    let mut producer = Producer::new(client_end.channel()).ok_or(Trouble::Channel(0))?;
    // The client's page, taken before the allocator is borrowed for the length
    // of the run.
    // SAFETY: `owned` is a frame the caller allocated and handed to nobody
    // else; the direct map makes it readable and writable for the whole of this
    // call, and no other reference into it exists.
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
    // SAFETY: `start_on` was called for this core and nothing else has joined
    // it. `serve` touches only the driver's control ring, whose two ends are
    // single-producer and single-consumer by construction.
    let joined = unsafe {
        crate::smp::join_serviced(setup.scheduling.cpu, tsc_khz, EXIT_MICROS, &mut || {
            let _ = supervising.serve();
        })
    };

    let asked = supervising.answered;

    // What the component said about itself, read out of memory the frame
    // granted it. RFC 0013's *read, never delivered*: it was never asked.
    let reported = Reported::of(&board);

    // SAFETY: on the core that prepared it, after the core that ran it reported
    // finished — which is what `join_serviced` returning `Ok` means, and which
    // the refusal below is checked against before anything reads its report.
    let ended = unsafe { crate::process::reap(frames, prepared) }.map_err(Trouble::Process)?;
    let exited = matches!(ended.death, crate::process::Death::Exited(_));

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
        capacity: reported.capacity,
        registered_at: observed.registered_at,
        refused_without_grant: observed.refused_without_grant,
        wrote: observed.wrote,
        read: observed.read,
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
    wrote: bool,
    read: bool,
    matched: bool,
    /// Unit: bytes.
    untouched: u32,
}

/// The client's whole run: register, write, read, compare.
///
/// Every wait in here is [`Supervising::awaited`], which serves the driver's
/// control ring while it waits — so the client and its server make progress
/// against each other on two cores, through one ring, with the frame answering
/// the one question the driver cannot answer for itself.
fn client(
    supervising: &mut Supervising<'_, '_>,
    producer: &mut Producer<'_>,
    page: &mut [u8],
    asking: Asking,
) -> Result<Observed, Trouble> {
    // --- the refusal, before anything is registered -------------------------
    //
    // A client that holds memory it may use and may not pass on cannot put it
    // in a driver's domain. Provoked on every run because a check nobody has
    // watched fail is indistinguishable from one that cannot fail — and this is
    // the check standing between a component's clients and each other's memory.
    let probe = registration(1, asking.ungrantable_cap, asking.bytes, BUFFERS);
    producer.submit(probe).map_err(|_| Trouble::Channel(0))?;
    let answer = supervising.awaited(asking.tsc_khz)?;
    let refused_without_grant =
        matches!(answer.error(), Some((error::AUTHORITY, error::authority::RIGHT_NOT_HELD)));
    if !refused_without_grant {
        return Err(Trouble::NotRefused);
    }

    // --- the registration ---------------------------------------------------
    let asked = registration(2, asking.owned_cap, asking.bytes, BUFFERS);
    producer.submit(asked).map_err(|_| Trouble::Channel(0))?;
    let answer = supervising.awaited(asking.tsc_khz)?;
    let naming = Fixed::from_completion(&answer)
        .map_err(|(refused, code)| Trouble::Registration(error::pack(refused, code)))?;
    // Where the *device* addresses the set. The frame knows it because the
    // frame answered the translation, and it is the frame's knowledge and never
    // the client's: nothing in the completion the *client* reaped carries an
    // address, and RFC 0024 is why.
    let registered_at = supervising.answered_at();

    // --- the client's buffers -----------------------------------------------
    //
    // The ownership types, over the page the registration just named. An `Idle`
    // is the only thing here that reaches bytes, a submission *moves* it, and
    // the completion is what hands it back — RFC 0024, and the reason a client
    // in this system cannot write to a buffer the device holds.
    let mut set = BufferSet::bind(naming, asking.negotiated, page).map_err(Trouble::Channel)?;
    let [mut source, mut sink] = set.carve::<2>().map_err(|_| Trouble::Geometry)?;
    if source.len() < TRANSFER as usize {
        return Err(Trouble::Geometry);
    }

    // A pattern the disk cannot produce and the poison is not. Position-derived
    // so that a transfer that landed the right number of bytes in the wrong
    // order fails the comparison — a constant fill would not.
    for (index, byte) in source.bytes_mut().iter_mut().take(TRANSFER as usize).enumerate() {
        *byte = pattern(index);
    }
    for byte in sink.bytes_mut().iter_mut().take(TRANSFER as usize) {
        *byte = POISON;
    }

    // --- the write ----------------------------------------------------------
    let entry = driver::write(3, AT, TRANSFER);
    let (lent, _) = source.submit(producer, entry).map_err(|_| Trouble::Channel(0))?;
    let answer = supervising.awaited(asking.tsc_khz)?;
    let wrote = !answer.is_error();
    let source = taken(lent, &answer)?;

    // --- the grant, or its absence ------------------------------------------
    //
    // The whole experiment, in one branch. RFC 0024 says a client may retire a
    // registration with buffers still in flight because *the memory is the
    // client's and it is entitled to take it back*, and what makes that safe is
    // exactly this: the translation goes away with it, so a transfer the device
    // had already been pointed at faults instead of landing in memory somebody
    // is about to reuse.
    if asking.half == Half::Outside {
        supervising.withdraw(asking.owned_cap, registered_at, asking.bytes);
    }

    // --- the read -----------------------------------------------------------
    let entry = driver::read(4, AT, TRANSFER);
    let (lent, _) = sink.submit(producer, entry).map_err(|_| Trouble::Channel(0))?;
    let answer = supervising.awaited(asking.tsc_khz)?;
    let read = !answer.is_error();
    let sink = taken(lent, &answer)?;

    // --- what is actually in the client's memory ----------------------------
    let mut matched = true;
    let mut untouched = 0;
    for index in 0..TRANSFER as usize {
        let Some(got) = sink.bytes().get(index) else { return Err(Trouble::Geometry) };
        if *got != pattern(index) {
            matched = false;
        }
        if *got == POISON {
            untouched += 1;
        }
    }
    // Read back rather than trusted: the source buffer must still hold what the
    // client put in it, because a device that wrote into the *source* would be
    // a corruption this comparison would otherwise report as a success.
    for index in 0..TRANSFER as usize {
        let Some(got) = source.bytes().get(index) else { return Err(Trouble::Geometry) };
        if *got != pattern(index) {
            return Err(Trouble::Transfer(0));
        }
    }

    Ok(Observed { registered_at, refused_without_grant, wrote, read, matched, untouched })
}

/// What the component wrote about itself into the half of its board that is
/// its own.
#[derive(Clone, Copy)]
struct Reported {
    counters: driver::Counters,
    /// Unit: sectors.
    capacity: u64,
    /// Unit: entries.
    drained: u64,
    /// One of `f_virtio_blk::routing::stopped`. Unit: none — an ordinal.
    outcome: u64,
}

impl Reported {
    /// Read it, and answer zeroes for a component that never wrote one.
    ///
    /// The magic is what tells those two apart, and it matters: a component
    /// that faulted before it reached its own report would otherwise publish a
    /// copy counter of zero — which is the number this whole subsystem
    /// publishes as a property, arrived at by the component never having run.
    /// `Report::verdict` refuses a run whose outcome is not
    /// `routing::stopped::TOLD` for exactly that reason.
    ///
    /// # Why three fields are converted rather than cast
    ///
    /// This is the frame reading a page a ring-3 component writes, so it is a
    /// value crossing a trust boundary and R04 applies: the transport is `u64`
    /// and `driver::Counters` holds `u32`, so a component that put `1 << 32`
    /// into `SERVED` would have been reported to the boot log as having served
    /// none, and a truncation that happens to land on a plausible tally is
    /// worse than one that lands on an implausible one. A value that does not
    /// fit is a component that scribbled on its own board rather than one with
    /// a large tally — four billion entries do not fit in this boot's ring —
    /// so it is answered the way a missing magic is, and the run fails
    /// `verdict` on the line that says the component did not report.
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
        Self {
            counters: driver::Counters {
                served,
                refused,
                bytes: read(routing::reported::BYTES),
                copies: read(routing::reported::COPIES),
                escaped,
                provoked: read(routing::reported::PROVOKED),
            },
            capacity: read(routing::reported::CAPACITY),
            drained: read(routing::reported::DRAINED),
            outcome: read(routing::reported::OUTCOME),
        }
    }

    /// What the frame knows about a component that reported nothing it can
    /// believe. See [`NOTHING`] for why every field of it is zero.
    const NOTHING_AT_ALL: Self = Self { counters: NOTHING, capacity: 0, drained: 0, outcome: 0 };
}

/// What a component that never reported has done, as far as the frame knows.
///
/// Zero everywhere, and the zero on `provoked` is the one that matters:
/// `Report::verdict` requires it to move, so a component that never ran fails
/// on the same line a component whose self-check stopped working would.
const NOTHING: driver::Counters =
    driver::Counters { served: 0, refused: 0, bytes: 0, copies: 0, escaped: 0, provoked: 0 };

/// Take a buffer back from the completion that answers it.
///
/// Unreachable in its failure arm and written out rather than unwrapped: the
/// completion this is asked about is the one the driver produced for this
/// entry, carrying this token and no `MORE` flag, so
/// [`InFlight::complete`](f_ring::InFlight::complete) always succeeds here.
///
/// What happens if it ever does not is the right thing and is worth knowing
/// about: the returned buffer is dropped, and dropping a buffer the device
/// still holds is the one misuse RFC 0024's types cannot refuse — so
/// `f_ring::buffers` refuses it at the drop, loudly, and the frame ends. A
/// quiet `Trouble` here would be this file deciding that a device writing into
/// memory nobody owns is a reportable condition.
fn taken<'m>(
    lent: f_ring::InFlight<'m, Fixed>,
    answer: &f_abi::Cqe,
) -> Result<f_ring::Idle<'m, Fixed>, Trouble> {
    lent.complete(answer).map_err(|_| Trouble::Transfer(0))
}

/// The allocator order that covers `bytes`, exactly.
///
/// Exactly, and not the next order up: a manifest declaring a quantity that is
/// not a whole number of frames at some order is a manifest that cannot be
/// satisfied by one allocation, and rounding up would hand a component more
/// than it declared — which is the same fault as handing it less, pointing the
/// other way. `docs/manifest.md` already requires `bytes` to be a positive
/// multiple of a frame; this is the second half of that arithmetic.
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

/// The byte the pattern puts at one position.
///
/// Position-derived and not a constant fill, because a transfer that landed the
/// right number of bytes in the wrong place would pass a comparison against a
/// constant. Cheap arithmetic rather than a seeded draw: this is a fixture, and
/// RFC 0004's substrate is for the things that have to *vary* reproducibly.
///
/// It never produces [`POISON`], and that is load-bearing rather than tidy:
/// [`Report::untouched`] counts bytes of the sink that still hold the poison,
/// so a pattern that could coincide with it would make *nothing landed here*
/// and *exactly the right byte landed here* the same observation at two
/// positions out of every five hundred and twelve.
const fn pattern(index: usize) -> u8 {
    let byte = (index as u8) ^ 0x5A;
    if byte == POISON { byte ^ 1 } else { byte }
}
