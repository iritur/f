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
//! # Why the driver's code runs in the frame today
//!
//! Two reasons, both of them somebody else's task, and both recorded here
//! rather than left for a reader to infer:
//!
//! - **Nothing schedules a component.** There is no scheduler until E1-B08, so
//!   an instance runs when the frame hands it a core. E1-B05 hit this and said
//!   so; `component::demonstrate` spawns, kills and refills a place without
//!   ever running its occupant.
//! - **A component cannot drive a ring.** Draining one means adopting a mapped
//!   channel and `f_ring::Mapping::adopt` is `unsafe`, which a `user/` crate
//!   may not write. RFC 0033 supplies the safe accessor a driver needs for its
//!   *device* and deliberately does not supply one for a *channel*: a channel is
//!   shared with a hostile peer and a device window is not, and one argument
//!   made for both would be the wrong argument used twice.
//!
//! So the frame calls `f_virtio_blk::driver::Driver::execute` where a scheduled
//! component would call it from its own polling loop, and it passes
//! [`iommu::Grant`] where a scheduled component would ask for a translation
//! over its control ring. Everything else — the registers, the descriptors, the
//! registration table, the counters — is the component's own code doing the
//! component's own work.
//!
//! *Reversal, and it is a date rather than a measurement:* E1-B08 lands a
//! scheduler and a safe channel adoption, at which point this file keeps the
//! supervisor's half and loses the two calls that stand in for a component's.
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
//! `outside` is *a grant being taken away under a driver*. What neither shows
//! is that the code doing the reaching runs at ring 3 — nothing schedules a
//! component until E1-B08, so the frame calls it, and these boots establish the
//! descriptor's provenance and the unit's refusal rather than the privilege
//! level of the code that built it.
//!
//! # What the zero-copy counter is worth on *this* boot, exactly
//!
//! `f_virtio_blk::driver::Counters::copies` is zero because there is no type in
//! the driver crate that turns a client's buffer into bytes: a
//! [`Reach`](f_ring::registry::Reach) is an address and a length, a
//! [`Region`] is the component's own memory, and the one function in that crate
//! which moves bytes takes the tally it moves as an argument and is not called
//! from the data path.
//!
//! So node 23 is a **structural property published as a number**, and not a
//! tally of copies some code path performed and this boot happened to find at
//! zero — those two read identically and only one of them is what is being
//! claimed. The mechanism behind it is `cargo xtask lint-datapath`, which
//! requires that crate to define exactly one function that moves bytes, to call
//! it exactly once, and to call it from `provoke_copy` and nowhere else — and
//! which refuses any line of shipped component source that mints a
//! [`Region`] or a `Window` out of a bare address, because a safe `const fn`
//! constructor over the direct map is the one way a crate that forbids `unsafe`
//! could still read a client's bytes with `stage` left honest. Node 24 is the
//! other half: it says the counting works at all.
//!
//! What is **not** true on this boot is that the driver's code is prevented from
//! reaching those bytes by an address space. It executes in the frame, where the
//! direct map covers all of physical memory, so what enforces the property here
//! is the driver crate's own types and the absence of any accessor that could
//! yield a slice — a property `cargo xtask lint-unsafe` and the workspace's
//! `unsafe_code = "forbid"` make load-bearing rather than aspirational, and
//! which `cargo xtask lint-datapath` checks rather than leaving to a reader's
//! search.
//! The second enforcement arrives with the scheduler: when the driver runs at
//! ring 3 its page tables will refuse what its types already do. That is
//! E1-B08's, and saying it here is cheaper than a reader inferring the stronger
//! claim from a zero.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, rights};
use f_abi::manifest::{ContentId, Record, route};
use f_abi::{ABI_VERSION, Negotiated, error};
use f_ring::device::{Region, Window};
use f_ring::registry::{Domains, registration};
use f_ring::{BufferSet, Collector, Consumer, Fixed, Mapping, Poster, Producer};
use f_virtio_blk::driver::{self, Driver};
use f_virtio_blk::transport::{SECTOR_BYTES, Windows};

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging::{AddressSpace, Features};
use crate::arch::x86_64::pci::{self, Bdf, Survey};
use crate::arch::x86_64::virtio;
use crate::arch::x86_64::vtd::{Fault, Unit};
use crate::cap::Table;
use crate::component;
use crate::iommu;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};

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
    /// The driver refused to start. Its own reason.
    Driver(f_virtio_blk::Trouble),
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
            Self::Driver(why) => why.message(),
            Self::Channel(_) => "the client's data ring could not be laid out or bound",
            Self::Geometry => "the client's region does not divide into the buffers declared",
            Self::Registration(_) => "the driver refused to register the client's buffer set",
            Self::Transfer(_) => "the driver refused a transfer that was supposed to be served",
            Self::Leaked => "the demonstration's frames did not all come back",
            Self::NoManifest => "no boot module declares the virtio-blk component",
            Self::Manifest => {
                "the driver's manifest and this machine disagree about what has to be routed"
            }
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
        return Ok(Declared { id: ContentId::of(module), frames, bytes });
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
    /// What the driver counted. Unit: see [`driver::Counters`].
    pub counters: driver::Counters,
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
        // The half that is true on both runs, checked first, because it is
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
    let outcome =
        unsafe { run(frames, unit, &mut domain, &found, declared, granted, owned, wire, half) };

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
    // is here rather than inside `Driver::start`: a component's declared needs
    // are the supervisor's to deliver, and a driver that mapped its own queue
    // would be a driver deciding what it was granted.
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
    let region = Region::at(frames.virt(granted) as u64, region_at, region_len)
        .map_err(|packed| Trouble::Driver(f_virtio_blk::Trouble::Register(packed)))?;
    let windows = Windows {
        common: window_of(&found.common)?,
        notify: window_of(&found.notify)?,
        isr: window_of(&found.isr)?,
        config: window_of(&found.device)?,
        notify_multiplier: found.notify_multiplier,
    };
    let agreed = Negotiated { version: ABI_VERSION, features: 0 };
    let mut driver = Driver::start(windows, region, agreed).map_err(Trouble::Driver)?;

    // The self-check that makes the zero worth reading. Run before the data
    // path, so a build in which it silently did nothing fails the verdict
    // rather than being hidden by a transfer that also did nothing.
    driver.provoke_copy().map_err(Trouble::Driver)?;

    // --- the client's ring --------------------------------------------------
    //
    // A real channel over a real frame, laid out by `f_abi::layout`, with the
    // driver on the server end and the client on the other. Not a recorder
    // standing in for one: the exit criterion says *through a ring*, and a
    // client that handed entries straight to a service would be a client whose
    // entries never crossed anything.
    let at = frames.virt(wire);
    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Trouble::Authority)?;
    // SAFETY: `wire` was allocated zeroed, is frame-aligned — stronger than the
    // cache-line alignment the layout asks for — and is `FRAME_SIZE` bytes with
    // no pointer into it held anywhere else.
    let server =
        unsafe { Mapping::describe(at, bytes, ENTRIES, 0, 0, 0) }.map_err(Trouble::Channel)?;
    // SAFETY: as above; two ends over one region is what a channel is, and
    // every accessor hands out atomics and `UnsafeCell`s rather than references.
    let client = unsafe { Mapping::adopt(at, bytes, 0, 0) }.map_err(Trouble::Channel)?;

    let mut producer = Producer::new(client.channel()).ok_or(Trouble::Channel(0))?;
    let reaper = Collector::new(client.completions()).ok_or(Trouble::Channel(0))?;
    let consumer = Consumer::new(server.channel()).ok_or(Trouble::Channel(0))?;
    let poster = Poster::new(server.completions()).ok_or(Trouble::Channel(0))?;
    let wiring = Wiring { consumer: &consumer, poster: &poster, reaper: &reaper };

    // --- the refusal, before anything is registered -------------------------
    //
    // A client that holds memory it may use and may not pass on cannot put it
    // in a driver's domain. Provoked on every run because a check nobody has
    // watched fail is indistinguishable from one that cannot fail — and this is
    // the check standing between a component's clients and each other's memory.
    let probe = registration(1, ungrantable_cap, bytes, BUFFERS);
    producer.submit(probe).map_err(|_| Trouble::Channel(0))?;
    let answer = wiring.turn(&mut driver, unit, domain, frames, &client_table, 0)?;
    let refused_without_grant =
        matches!(answer.error(), Some((error::AUTHORITY, error::authority::RIGHT_NOT_HELD)));
    if !refused_without_grant {
        return Err(Trouble::NotRefused);
    }

    // --- the registration ---------------------------------------------------
    let asked = registration(2, owned_cap, bytes, BUFFERS);
    producer.submit(asked).map_err(|_| Trouble::Channel(0))?;
    let answer = wiring.turn(&mut driver, unit, domain, frames, &client_table, 0)?;
    let naming = Fixed::from_completion(&answer)
        .map_err(|(refused, code)| Trouble::Registration(error::pack(refused, code)))?;
    // Where the *device* addresses the set. Known here because `iommu::Grant`
    // answers the physical address of the memory a capability names and this
    // build makes a device address the identity of a physical one — which is a
    // decision `kernel/src/iommu.rs` argues and writes a reversal for, not an
    // assumption. It is the frame's knowledge and never the client's: nothing
    // in the completion carries an address, and RFC 0024 is why.
    let registered_at = owned.addr();

    // --- the client's buffers -----------------------------------------------
    //
    // The ownership types, over the page the registration just named. An `Idle`
    // is the only thing here that reaches bytes, a submission *moves* it, and
    // the completion is what hands it back — RFC 0024, and the reason a client
    // in this system cannot write to a buffer the device holds.
    // SAFETY: `owned` is a frame this function allocated and handed to nobody
    // else; the direct map makes it readable and writable for the whole of this
    // call, and no other reference into it exists.
    let page = unsafe { core::slice::from_raw_parts_mut(frames.virt(owned), FRAME_SIZE as usize) };
    let mut set = BufferSet::bind(naming, agreed, page).map_err(Trouble::Channel)?;
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
    let (lent, _) = source.submit(&mut producer, entry).map_err(|_| Trouble::Channel(0))?;
    let answer = wiring.turn(&mut driver, unit, domain, frames, &client_table, 0)?;
    let wrote = !answer.is_error();
    let source = taken(lent, &answer)?;

    // --- the grant, or its absence ------------------------------------------
    //
    // The whole experiment, in one branch. RFC 0024 says a client may retire a
    // registration with buffers still in flight because *the memory is the
    // client's and it is entitled to take it back*, and what makes that safe is
    // exactly this: the translation goes away with it, so a transfer the device
    // had already been pointed at faults instead of landing in memory somebody
    // is about to reuse. Here the client takes it back between the two
    // transfers, and the driver — which still holds a live registration naming
    // it — hands the device a descriptor pointing outside its grant.
    if half == Half::Outside {
        let mut asking = iommu::Grant {
            unit: &mut *unit,
            domain: &mut *domain,
            frames: &mut *frames,
            table: &client_table,
        };
        asking.unmap(owned_cap, registered_at, bytes);
    }

    // --- the read -----------------------------------------------------------
    let entry = driver::read(4, AT, TRANSFER);
    let (lent, _) = sink.submit(&mut producer, entry).map_err(|_| Trouble::Channel(0))?;
    // The read is the one entry a half can bend. `Half::beyond` is zero for the
    // other two, so the same line is the data path on `inside` and on `outside`
    // and is the provocation on `escape`.
    let answer = wiring.turn(&mut driver, unit, domain, frames, &client_table, half.beyond())?;
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

    driver.stop().map_err(Trouble::Driver)?;
    let faults = unit.faults();

    Ok(Report {
        half,
        declared,
        bdf: found.bdf,
        windows: found.pages,
        capacity: driver.capacity(),
        registered_at,
        refused_without_grant,
        wrote,
        read,
        matched,
        untouched,
        counters: driver.counters(),
        fault: faults.first,
        faults: faults.records,
    })
}

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

/// One register structure, as a component may touch it.
fn window_of(structure: &virtio::Structure) -> Result<Window, Trouble> {
    Window::at(structure.at, structure.len)
        .map_err(|packed| Trouble::Driver(f_virtio_blk::Trouble::Register(packed)))
}

/// The four ends of the client's data ring, and the one turn of the crank that
/// uses all of them.
///
/// A struct because the alternative is five arguments repeated at six call
/// sites, and because the four ends belong together: they are one channel, and
/// a caller holding three of them is a caller that has forgotten which side it
/// is on.
struct Wiring<'a, 'm> {
    consumer: &'a Consumer<'m>,
    poster: &'a Poster<'m>,
    reaper: &'a Collector<'m>,
}

impl Wiring<'_, '_> {
    /// Drain what the client submitted, let the driver answer it, post the
    /// answer, and reap it.
    ///
    /// This is the polling loop a scheduled driver runs on its own core, done
    /// here on its behalf because nothing schedules one — the same substitution
    /// `component::publish` makes for a component's control ring, and the same
    /// reason. What it is not is a shortcut past the ring: the entry really is
    /// written into the shared region by the client's [`Producer`] and really is
    /// taken out of it by the driver's [`Consumer`], through the `Release`/
    /// `Acquire` pair the whole design rests on.
    ///
    /// `now` is zero, and deliberately. A completion carries a timestamp the
    /// client reads to know *when*; the boot log is a fixture and prints none of
    /// them, and a clock read here would be a number that moved between hosts
    /// for no reason a reader could name.
    ///
    /// `beyond` is what the driver adds to the address a registration answered
    /// before it becomes a descriptor, and it is an argument here rather than a
    /// mode on the driver so that every call site says which it is. Three of the
    /// four in `run` are a literal zero.
    fn turn(
        &self,
        driver: &mut Driver,
        unit: &mut Unit,
        domain: &mut crate::arch::x86_64::vtd::Domain,
        frames: &mut FrameAllocator,
        table: &Table,
        beyond: u64,
    ) -> Result<f_abi::Cqe, Trouble> {
        let entry =
            self.consumer.pop().map_err(|_| Trouble::Channel(0))?.ok_or(Trouble::Channel(0))?;
        // The frame stands in for the driver's route to the IOMMU. A scheduled
        // component asks for a translation over its control ring; there is no
        // such opcode yet and no way for a component to submit on one, so the
        // supervisor passes the authority in. The *check* is unchanged either
        // way: the handle is resolved against the client's table and refused
        // without `GRANT`.
        let mut asking = iommu::Grant { unit, domain, frames, table };
        // Two entry points and not a flag, so the provocation is greppable: the
        // data path calls `execute`, which passes a literal zero of its own, and
        // only the `escape` half reaches `provoke_escape`.
        let answer = if beyond == 0 {
            driver.execute(&entry, &mut asking, 0)
        } else {
            driver.provoke_escape(&entry, &mut asking, 0, beyond)
        };
        self.poster.post(answer).map_err(|_| Trouble::Channel(0))?;
        self.reaper.take().map_err(|_| Trouble::Channel(0))?.ok_or(Trouble::Channel(0))
    }
}
