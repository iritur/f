// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The half of a driver supervisor that is the same in all three of them.
//!
//! # What this is
//!
//! `kernel/src/blk.rs`, `kernel/src/net.rs` and `kernel/src/gpu.rs` each stand
//! a driver up, hand it a device, serve its control ring and take it down.
//! Three quarters of that was the same code written three times. RFC 0051 said
//! a third driver was what would merge it; RFC 0054 refused to do it inside
//! `E1-B04` because that task's evidence was a picture on a screen; RFC 0071 is
//! the entry that measures the duplication and decides what moves.
//!
//! What moved is what was **byte-identical with comments stripped**:
//! [`Registers`] and its two methods, [`Supervising`] and its implementation,
//! [`Declared`], [`declared`] and [`order_for`].
//!
//! **One parameter, and it is the one `kernel/src/net.rs` predicted.** That
//! file said the three copies were "adapted only in which manifest name they
//! look for and which counters they read". The name is [`declared`]'s `driver`
//! argument. The counters are `Reported`, which stayed behind. Nothing else
//! here takes anything from a caller, and no driver's name appears in this
//! file.
//!
//! # What is deliberately not here
//!
//! **`Reported` is not one type.** The three carry a name and a mechanism in
//! common and not a layout: `blk`'s has `capacity`, `overtaken`, `queued_max`
//! and `in_flight`, which the other two have no counterpart for, because a
//! block request has a depth and an order and a display command does not.
//! Merging it behind a shared prefix would move four of `blk`'s fields by eight
//! bytes; every writer uses the symbolic name so a full rebuild is safe, a
//! partial one is not, and the magic word written to catch exactly that would
//! still pass on a stale component image reading shifted fields. RFC 0071 says
//! what would change that, and it is a compile-time offset assertion written
//! *before* the offsets move.
//!
//! **`prepare_driver(` stays called from `kernel/src/blk.rs`.** It is the
//! needle `xtask`'s `CHAOS_GAP` keys on, and moving the call here would turn
//! that check red without closing the gap it declares — a red build for a false
//! reason, which is worse than no check. The gap is about a driver scheduled
//! outside the place its manifest is spawned into and has nothing to do with
//! which file the call sits in.
//!
//! **Each driver keeps its own `Trouble`.** They are not the same enum —
//! `net`'s carries a `NoFrame` the others have no use for — so this module
//! answers its own [`Trouble`], which is exactly the four refusals the shared
//! code can produce, and each driver converts. The conversion is one arm per
//! variant and changes no message, which is what keeps a boot log the same
//! bytes it was.
//!
//! # Why it is in the frame
//!
//! Because it holds the frame's things: [`Supervising`] borrows a `Domain`, a
//! `FrameAllocator` and a `cap::Table` and builds an `iommu::Grant`, and
//! [`Registers`] reads `paging::DEVICE_OFFSET` and
//! `process::BLK_REGISTER_PAGES`. There is no version of this that lives above
//! the frame, which is a statement about what a driver supervisor *is* rather
//! than about where a file was put.

use f_abi::manifest::{self, ContentId, Record, route};
use f_abi::{control, error};
// `Domains` is the trait carrying `map` and `unmap`; without it in scope the
// compiler reads `asking.map(..)` as an iterator adaptor and says so.
use f_ring::registry::Domains;
use f_ring::{Collector, Consumer, Poster};

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging;
use crate::arch::x86_64::virtio;
use crate::arch::x86_64::vtd::Unit;
use crate::cap::Table;
use crate::component;
use crate::iommu;
use crate::mem::{FRAME_SIZE, FrameAllocator, Order};

/// How long a supervisor waits for a driver to answer before it gives up.
///
/// Five seconds, and the same five in all three supervisors before this module
/// existed. It is an anti-wedge bound and not a measurement of anything: a
/// driver that has not answered in five seconds is stuck, and a machine slower
/// than that is a machine this boot cannot tell from a stuck one.
/// Unit: microseconds.
pub const ANSWER_MICROS: u64 = 5_000_000;

/// The need in a driver's manifest that names the register pages.
///
/// The same two bytes in all three drivers, which is why they are here and not
/// a parameter: `mmio` and `queues` are the shape of a virtio driver's manifest
/// rather than one driver's choice of words.
const NEED_MMIO: &[u8] = b"mmio";

/// The need that names the untyped region the driver splits into its queues.
const NEED_QUEUES: &[u8] = b"queues";

/// What the shared half of a supervisor can refuse with.
///
/// Exactly four, because exactly four is what the code in this module produces.
/// Each driver keeps its own richer `Trouble` and converts — see the module
/// comment on why they are not one enum, and note that the conversion is
/// required to preserve each driver's own message, because a boot log that
/// changed wording would be a change this module is not entitled to make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trouble {
    /// No boot module carried a component file for this driver.
    NoManifest,
    /// A component file was carried and could not be read as one.
    Manifest,
    /// A ring this build wrote it cannot read back, or one with no room left.
    Channel(i32),
    /// The driver did not answer inside the bound, carrying the bound.
    /// Unit: microseconds.
    NoAnswer(u64),
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
    /// The reservation class the manifest declares, as
    /// `f_abi::class` reads it — the ceiling this component is admitted for and
    /// therefore the most urgent class it can serve anything at.
    ///
    /// Read out of the record like everything else here and never written down
    /// in the frame, for the reason RFC 0025 bound 2 gives about ceilings: a
    /// ceiling somebody restated is a ceiling that can drift from the manifest
    /// it was declared in, and the drift is invisible until a hard-class client
    /// is quietly served as batch. Unit: none — an `f_abi::class` ordinal.
    pub admitted: u16,
    /// The component's own image, out of the same component file.
    ///
    /// Read here rather than found again later, because it is the same
    /// module: a datapath that read a *manifest* from one place and an *image*
    /// from another would be a datapath whose content hash named neither.
    pub image: &'static [u8],
}

/// Find the driver's component file and read what it declares.
///
/// `driver` is the manifest name to look for, and it is the one thing in this
/// module a caller supplies. `kernel/src/net.rs`'s module comment said as much
/// before this file existed — the three copies were "adapted only in which
/// manifest name they look for and which counters they read". The name is this
/// parameter; the counters are `Reported`, which stayed behind for the reason
/// the module comment gives.
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
pub unsafe fn declared(boot: &BootInfo, driver: &[u8]) -> Result<Declared, Trouble> {
    // SAFETY: the caller's guarantee, passed down.
    let (modules, count) = unsafe { component::modules(boot) };
    for module in modules.iter().take(count) {
        let Ok(record) = Record::read(module) else { continue };
        if record.label() != driver {
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
        // R04 at a byte the frame did not write: a class this build cannot name
        // is a record from a schema this build cannot read, and reading it as
        // the nearest class would admit a component at a ceiling nobody
        // declared.
        let Some(admitted) = manifest::class::admitted(record.class) else {
            return Err(Trouble::Manifest);
        };
        return Ok(Declared { id: ContentId::of(module), frames, bytes, image, admitted });
    }
    Err(Trouble::NoManifest)
}

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
pub struct Registers {
    /// The first page of the span, physical. Unit: bytes, physical.
    pub base: u64,
    /// How many pages it covers. Unit: pages.
    pub pages: u32,
    /// Each structure's offset into the span and its length, in the order
    /// common, notify, ISR, device configuration. Unit: bytes.
    pub each: [(u32, u32); 4],
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
    pub fn of(found: &virtio::Found, declared: &Declared) -> Result<Self, Trouble> {
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
    pub fn physical(structure: &virtio::Structure) -> Result<u64, Trouble> {
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
pub struct Supervising<'a, 'm> {
    /// What the driver asked for.
    pub asks: &'a Consumer<'m>,
    /// Where its answers go, and where the frame's notices go.
    pub answers: &'a Poster<'m>,
    /// The client's end of the data ring, on the frame's side.
    pub reaper: &'a Collector<'m>,
    /// The remapping unit the translation is programmed into.
    pub unit: &'a mut Unit,
    /// The device's own domain, which is the whole of what a translation may
    /// reach: a driver asking for one cannot name a page outside it.
    pub domain: &'a mut crate::arch::x86_64::vtd::Domain,
    /// Where a page table for that domain comes from, when the walk needs one.
    pub frames: &'a mut FrameAllocator,
    /// The **client's** table. Every handle a driver names in a translation
    /// request is resolved against this one, which is what makes a driver
    /// unable to grant itself anything: it is asking about somebody else's
    /// capability, and the answer is somebody else's rights.
    pub table: &'a Table,
    /// The device address the last translation answered.
    ///
    /// Kept here because it is the frame's knowledge and the client's need:
    /// nothing in the completion a *client* reaps carries an address — RFC 0024
    /// — so a client that needs to know where the device sees its buffer, in
    /// order to say where a refused transaction should have faulted, asks the
    /// frame that answered it. A client on the far side of a boundary could not
    /// ask this and would not be entitled to; this one is the frame.
    /// Unit: bytes, in the device's address space.
    pub answered_at: u64,
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
    pub answered: u32,
}

impl Supervising<'_, '_> {
    /// Where the last translation this served put the memory it was asked about.
    /// Unit: bytes, in the device's address space.
    pub const fn answered_at(&self) -> u64 {
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
    pub fn serve(&mut self) -> Result<u32, Trouble> {
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
    pub fn execute(&mut self, entry: &f_abi::Sqe) -> f_abi::Cqe {
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
    pub fn within(&mut self, tsc_khz: u64, micros: u64) -> Result<Option<f_abi::Cqe>, Trouble> {
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
    pub fn awaited(&mut self, tsc_khz: u64) -> Result<f_abi::Cqe, Trouble> {
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
    pub fn stop(&self) -> Result<(), Trouble> {
        self.answers
            .post(control::entry(control::notice::STOP, 0, 0, 0))
            .map_err(|_| Trouble::Channel(0))
    }

    /// Take a translation away, on the frame's own initiative.
    ///
    /// The `outside` half, and it is deliberately not an operation the driver
    /// asked for: RFC 0024 says *the memory is the client's and it is entitled
    /// to take it back*, so what happens here happens under a driver that holds
    /// a live registration and is doing nothing wrong.
    pub fn withdraw(&mut self, cap: u32, address: u64, len: u32) {
        let mut asking = iommu::Grant {
            unit: &mut *self.unit,
            domain: &mut *self.domain,
            frames: &mut *self.frames,
            table: self.table,
        };
        asking.unmap(cap, address, len);
    }
}

/// The allocator order that covers `bytes`, exactly.
///
/// Exactly, and not the next order up: a manifest declaring a quantity that is
/// not a whole number of frames at some order is a manifest that cannot be
/// satisfied by one allocation, and rounding up would hand a component more
/// than it declared — which is the same fault as handing it less, pointing the
/// other way. `docs/manifest.md` already requires `bytes` to be a positive
/// multiple of a frame; this is the second half of that arithmetic.
pub fn order_for(bytes: u64) -> Option<Order> {
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
