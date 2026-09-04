// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Unmap under churn: what it costs to take a translation back, at the rate
//! the datapath produces.
//!
//! # Why this is a workload and not an optimisation
//!
//! `TODO.md`'s `E1-B14` is written the way it is because of that list's own
//! ordering rule 3: batching is exactly the kind of thing that must not be
//! designed before the measurement exists. Every revoke-unmap in this kernel
//! was one page, one interrupt, one spin on an acknowledgement — correct, and
//! priced for a kernel that unmaps rarely. The datapath changes the rate:
//! registered buffer sets cycle, and a driver restart retires a component's
//! whole grant at once. So this file drives that rate and counts, and the
//! number decides.
//!
//! # What it found, and why the finding is not where the task was looking
//!
//! **The churn issues no shootdowns and no interrupts at all.** The task named
//! *shootdowns* and *IPIs*, and both are `crate::smp`'s: a shootdown is a page
//! taken out of a running process's address space and every other core told to
//! forget it. Nothing in the datapath's churn reaches that path:
//!
//! - a registered buffer set is a *device* translation, so retiring one edits
//!   the remapping unit's second-level tables and no processor's;
//! - a driver restart's teardown has no shootdown either, and
//!   `component::tear_down` says why in its own words — an instance's address
//!   space has never been in `CR3`, so no core holds a translation to it;
//! - the one path that does shoot down is `process::withdraw`, reached by
//!   revoking a capability that a *running* process had mapped, which is what
//!   `cargo xtask cap unmap` boots.
//!
//! So one-page-one-interrupt was already under any bound worth stating on this
//! workload, because the workload performs none — and the counters print the
//! machine's running total beside the churn's delta precisely so that the zero
//! is a finding rather than a counter nobody wired up. The total is not zero:
//! this kernel withdraws the application-processor on-ramp during bring-up, and
//! that shootdown is on every boot.
//!
//! What the churn *does* cost is on the other side of the same sentence. Every
//! page of every retired set made the remapping unit throw away everything it
//! had cached — globally, twice, each half a write followed by a spin on the
//! unit clearing the request bit. `vtd`'s own module comment named this task as
//! the measurement that would settle it. It is settled here.
//!
//! # Both halves, or the number means nothing
//!
//! One run per [`Invalidation`], over the identical geometry, in one boot. The
//! control is what this build did before `E1-B14` and it is kept rather than
//! deleted, for `claims/README.md`'s reason: a ratio whose denominator was
//! removed is a number nobody can re-check when the rate changes again.
//!
//! # What is real here and what stands in
//!
//! The registration table is `f_ring::registry::Table`, the one a driver's
//! service uses. The unmap is `iommu::Grant`, the one a driver's control ring
//! reaches. The remapping unit is the machine's, programmed and enabled by this
//! boot. So every count is a count of what the shipped code made real hardware
//! do.
//!
//! **What stands in is the caller.** A driver at ring 3 is not in this loop; the
//! frame drives the registry itself, on the boot processor, at the rate a driver
//! would. That is deliberate and it is a limit: it means this measures the cost
//! of the churn and not the *rate* a real client produces, and a datapath whose
//! clients turn out to cycle their buffers less often than this would pay less
//! than this says. It does not affect the ratios, which are what the claim is —
//! invalidations per request is a property of one request — and it is why the
//! claim's statement is about what one unmap costs rather than about what a
//! second of running costs. Closing it needs a supervisor that can start a
//! driver and a client and let them run, which is `E1-B05`'s, and it is the same
//! absence `CHAOS_GAP` already declares one subsystem over.
//!
//! # What it observes, and what it only counts
//!
//! Counting what the frame did is not the same as observing that it worked, and
//! this workload does both — the distinction is worth stating because the first
//! draft did only the first. Every retirement is followed by a walk of the
//! unit's own second-level tables asking whether the set's pages are still
//! translated ([`Counts::standing_after_unmap`], required to be zero), and
//! every registration by the same walk asking whether they *are*
//! ([`Counts::reachable_registered`], required to equal the sets registered) —
//! because a walk that answered *no* to everything would report a perfect
//! revocation over a domain that never mapped anything. That is what makes a
//! batched multi-page unmap observed rather than argued: N entries cleared, one
//! invalidation published, and nothing left translated.
//!
//! One more request shape is observed and no client can produce it: a set with a
//! page taken out from under it ([`hole`]). `Unit::unmap_range` returns the
//! first refusal *after* attempting the rest, and a version that stopped at the
//! hole would leave every page beyond it translated — so the stage makes a hole
//! on purpose and requires the batched request over the whole set to clear
//! everything either side of it.
//!
//! What it does **not** observe is a *device*. No device is attached to this
//! domain, so a translation cached inside one is out of reach here, and the
//! only boot that watches a device fault after a withdrawal — `cargo xtask
//! blk`'s `outside` half — does it over a one-page registration, where the two
//! invalidation policies are the same run. `REVOKE_GAP` in `xtask` is that
//! residual declared as a quantity rather than left as this paragraph, and it
//! goes red the day a device is attached here.
//!
//! The frames are counted the same way and for the same reason
//! ([`Counts::frames_before`] and [`Counts::frames_after`]): forty
//! register-and-retire cycles per half is where a leak of one table frame per
//! cycle stops being invisible, which is `docs/test-taxonomy`'s *frame leak
//! under churn* row and is why that row named this task.
//!
//! # The time, and where it may be published
//!
//! Counting is all a *container* may do — `bench/src/lib.rs` refuses to record
//! a timing here and is right to. Recording is not publishing, and the two are
//! separated rather than conflated: [`time`] takes a thousand and twenty-four
//! timed unmap requests through the shipped path on this machine's real
//! remapping unit and keeps the distribution, and `kernel/src/main.rs` prints
//! percentiles only when the command line says this machine is a measurement
//! environment — which `f_bench::Environment` decides in `xtask`, so there is
//! one rule about what may be quoted rather than a second one in here that
//! could disagree with it. `claims/0015` is that number and it is `pending` on
//! `E0-D10`'s machine.
//!
//! `bench/src/bin/unmap_churn.rs` remains beside the other `E1-P10` workloads
//! and remains the smaller half: it drives the identical registry churn against
//! a host clock, with no hardware under it to invalidate, so what it times is
//! the arithmetic above the unit. The boot is where the walk and the round trip
//! are.

use f_abi::cap::{CapType, rights};
use f_ring::registry::{Domains, Refusal, Table as Registrations};

use crate::arch::x86_64::read_tsc;
use crate::arch::x86_64::vtd::{Domain, Invalidation, Refuse, Unit};
use crate::cap::Table;
use crate::iommu;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};

/// Pages in one registered buffer set.
///
/// Eight, and it is the same eight `bench/src/bin/buffer_register.rs` uses:
/// thirty-two kilobytes divided into eight four-kilobyte buffers, one page
/// each, which is the grain a remapping unit works in. The number is
/// load-bearing rather than arbitrary — it is exactly the amortisation a batch
/// buys, so a set of one page would make a batched unmap and an unbatched one
/// the same run, and this workload would report an improvement of nothing while
/// nothing was wrong. `claims/0014` bounds it from below for that reason.
const SET_PAGES: u64 = 8;

/// The allocation order that gives [`SET_PAGES`] contiguous frames.
const SET_ORDER: u8 = 3;

/// Buffers in a set. One per page, so a name is a page.
const SET_BUFFERS: u32 = 8;

/// Sets a component holds when it dies.
///
/// The restart half: a driver that is killed had this many registrations live,
/// and `f_ring::registry::Table::retire_all` takes them all back in one pass.
/// Eight is the depth `user/virtio-blk` could reach and is small enough that
/// the whole workload fits in a quarter of a mebibyte.
const SETS: usize = 8;

/// Register-and-retire cycles the steady half performs.
///
/// The other churn source, and the one that runs while nothing is wrong:
/// RFC 0024 says the memory is the client's and it is entitled to take it back,
/// so a client that cycles its buffers pays this on every cycle.
const CYCLES: usize = 32;

/// Registration slots. A power of two, because a slot index is masked.
const SLOTS: usize = 16;

/// Why the churn could not be run.
///
/// A workload that could not be arranged is not a workload that reported a
/// zero, which is the distinction `dma::provoke` already draws and the reason
/// every one of these ends the boot rather than printing a smaller number.
#[derive(Clone, Copy, Debug)]
pub enum Trouble {
    /// The machine could not spare the memory a set is made of.
    NoMemory,
    /// A capability table could not be filled.
    Authority,
    /// The unit refused a domain, or a translation into one.
    Unit(Refuse),
    /// The frame refused a grant the workload holds grantably.
    Refused,
    /// A registration was refused.
    Registration,
    /// A retirement retired a different number of sets than were registered.
    Retirement,
}

impl Trouble {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NoMemory => "no memory for the buffer sets the churn cycles",
            Self::Authority => "the churn's own capability table could not be filled",
            Self::Unit(inner) => inner.message(),
            Self::Refused => "the frame refused a translation for memory held grantably",
            Self::Registration => "a registration the churn depends on was refused",
            Self::Retirement => "a restart retired a different number of sets than were live",
        }
    }
}

/// What one half of the churn cost.
///
/// Every field is a count and every count is the same number on a fast machine
/// and a slow one, which is what lets `claims/0014` gate in a container —
/// `claims/0005` is the precedent and states the rule.
#[derive(Clone, Copy, Default)]
pub struct Counts {
    /// Buffer sets registered. Unit: sets.
    pub registered: u64,
    /// Buffer sets retired. Unit: sets.
    pub retired: u64,
    /// Unmap requests the unit was given. Unit: requests.
    ///
    /// One per set retired, whichever half retired it, because a request is a
    /// `Domains::unmap` call and the registry makes exactly one per set.
    pub requests: u64,
    /// Leaf entries cleared. Unit: pages.
    pub pages: u64,
    /// Global invalidations an unmap made the unit perform. Unit: rounds.
    pub invalidations: u64,
    /// Leaf entries a registration wrote. Unit: pages.
    pub pages_mapped: u64,
    /// Global invalidations a *map* made the unit perform. Unit: rounds.
    ///
    /// Not this task's subject and counted anyway, because it is the same cost
    /// on the other side of the same cycle and leaving it out would have made
    /// the unmap's saving look like the whole answer. `CHURN_GAP` in `xtask` is
    /// what carries the number and names the task that owes the fix.
    pub map_invalidations: u64,
    /// Pages this half asked other cores to forget. Unit: pages.
    pub shootdowns: u64,
    /// Interrupts that cost. Unit: interrupts.
    pub ipis: u64,
    /// Registered sets the unit's own tables were then asked about, and
    /// answered *yes* for every page of. Unit: sets.
    ///
    /// The positive control for the row below, and it is the row that makes the
    /// row below mean anything: a walk that answered *no* to everything would
    /// report a perfect revocation over a domain that never had a translation
    /// in it. Required to equal [`Counts::registered`].
    pub reachable_registered: u64,
    /// Retired sets the unit's tables still translate a page of. Unit: sets.
    ///
    /// **Zero, and this is the row that observes rather than counts.** Every
    /// other number here is the frame reporting what it did; this one is the
    /// second-level tables being read back after a batched multi-page unmap and
    /// asked whether the pages are gone — the same walk `Grant`'s
    /// `PageWalk::reaches` answers a registration with, which is the unit's own
    /// answer rather than a record of what this kernel believes it programmed.
    ///
    /// What it does **not** observe is a device: nothing is attached to the
    /// churn's domain, so a translation cached inside a device is out of this
    /// workload's reach. `REVOKE_GAP` in `xtask` is that residual, declared.
    pub standing_after_unmap: u64,
    /// Free frames before the churn began. Unit: frames.
    pub frames_before: u64,
    /// Free frames after every set was retired, the domain released and the
    /// workload's own memory handed back. Unit: frames.
    ///
    /// Beside the row above rather than subtracted from it, because the two
    /// being *printed* is what makes a leak legible: an equality that failed
    /// would otherwise report a difference with nothing to compare it to. The
    /// churn is the one workload in this tree that registers, maps, retires and
    /// releases forty times per half, so it is where a per-cycle leak of a
    /// single table frame becomes visible at all — `docs/test-taxonomy`'s
    /// *frame leak under churn* row, which named `E1-B14` and which this
    /// closes for the registration path.
    pub frames_after: u64,
}

impl Counts {
    /// Serialising register round trips, which is what an invalidation costs.
    ///
    /// Unit: round trips. Derived rather than counted, and the one derived
    /// number here: [`crate::arch::x86_64::vtd::INVALIDATION_ROUND_TRIPS`] is a
    /// property of the register interface rather than of a run, so counting it
    /// would be counting the same event twice under two names.
    #[must_use]
    pub const fn round_trips(&self) -> u64 {
        self.invalidations * crate::arch::x86_64::vtd::INVALIDATION_ROUND_TRIPS
    }
}

/// A registration service's view of the frame, with the invalidation policy
/// under test.
///
/// The candidate half is [`iommu::Grant`] itself — the code the frame ships,
/// called through the trait a real driver's control ring calls it through — so
/// what is measured is the shipped path and not a copy of it. The control half
/// is the loop that path *was* before `E1-B14`, expressed as the same
/// [`Unit::unmap_range`] with the other policy, so the two differ in one
/// argument and in nothing else.
struct Churned<'a, 'b> {
    grant: &'a mut iommu::Grant<'b>,
    when: Invalidation,
    /// Sets the unit's tables translated every page of while they were
    /// registered. [`Counts::reachable_registered`].
    reachable: u64,
    /// Sets the unit's tables still translate a page of after the unmap.
    /// [`Counts::standing_after_unmap`].
    standing: u64,
}

impl Churned<'_, '_> {
    /// Does the domain translate every page of `len` bytes at `address`?
    ///
    /// The unit's own walk, read back out of the tables the unmap just edited.
    /// It is asked on both sides of every retirement, and both answers are
    /// counted, because only one of them is a check: *no translation after* is
    /// worth exactly as much as *a translation before* is worth, and a workload
    /// that asked only the second question would report a clean revocation over
    /// a domain that never mapped anything.
    fn reaches(&self, address: u64, len: u32) -> bool {
        // SAFETY: as everything else in this file — the allocator is rebound
        // onto the direct map of the active address space, and this walks
        // tables `iommu` made. A read, and the `&mut` fields are reborrowed
        // shared for it.
        unsafe {
            self.grant.unit.reaches(self.grant.frames, self.grant.domain, address, u64::from(len))
        }
    }
}

impl Domains for Churned<'_, '_> {
    fn map(&mut self, cap: u32, len: u32) -> Result<u64, Refusal> {
        let at = self.grant.map(cap, len)?;
        if self.reaches(at, len) {
            self.reachable = self.reachable.saturating_add(1);
        }
        Ok(at)
    }

    fn unmap(&mut self, cap: u32, address: u64, len: u32) {
        match self.when {
            Invalidation::PerRequest => self.grant.unmap(cap, address, len),
            Invalidation::PerPage => {
                // `iommu::Grant::pages` restated rather than reached, because
                // it is private and making it public to serve a control would
                // be widening an interface for a measurement. The geometry it
                // refuses cannot arise here — `Registrations::register` divides
                // the region by the buffer count and this workload's numbers
                // are whole pages — and a refusal is the same silent skip the
                // trait requires either way.
                let len = u64::from(len);
                if len == 0 || !len.is_multiple_of(FRAME_SIZE) {
                    return;
                }
                // SAFETY: as `iommu::Grant::unmap`: the allocator is rebound
                // onto the direct map of the active address space, and this
                // walks tables `iommu` made.
                let _ = unsafe {
                    self.grant.unit.unmap_range(
                        self.grant.frames,
                        self.grant.domain,
                        address,
                        len / FRAME_SIZE,
                        Invalidation::PerPage,
                    )
                };
                let _ = cap;
            }
        }

        // The observation, after the request and not inside it. A batched
        // request clears N entries and publishes one invalidation, so *the
        // entries are gone* is exactly the claim a reader is entitled to
        // disbelieve — and until this line the workload counted the clearing
        // and never looked. Asked of the same tables the unit walks, so a build
        // that cleared the wrong level, stopped at a hole, or batched away the
        // walk itself answers here rather than in a review.
        if self.reaches(address, len) {
            self.standing = self.standing.saturating_add(1);
        }
    }
}

/// Run the churn once, under one invalidation policy, and answer what it cost.
///
/// # Errors
///
/// [`Trouble`]. Every one of them is the workload failing to be arranged rather
/// than a finding, and the caller ends the boot on all of them.
///
/// # Safety
///
/// The kernel's address space must be active, `frames` rebound onto its direct
/// map, and `unit` a remapping unit this kernel has programmed and enabled —
/// which is the whole list [`Unit::map`] asks for.
pub unsafe fn run(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    when: Invalidation,
) -> Result<Counts, Trouble> {
    let Some(order) = Order::new(SET_ORDER) else { return Err(Trouble::NoMemory) };

    // Before anything is allocated, and read again after everything has been
    // given back. Every frame this workload takes — the sets' memory, the
    // domain's root, every second-level table a registration made the unit
    // build — is supposed to return, and forty register-and-retire cycles per
    // half is where a leak of one frame per cycle stops being invisible.
    let frames_before = frames.free_count();

    // The memory the sets are made of, and a capability table naming it.
    //
    // Built here rather than borrowed from whatever process the boot last ran,
    // for `dma::grant`'s reason: a measurement that depends on what another
    // stage left in a table is a measurement of that stage.
    let mut blocks = [Frame::from_addr(0); SETS];
    let mut caps = [0u32; SETS];
    let mut table = Table::EMPTY;
    let mut held = 0usize;
    let mut trouble = None;
    for index in 0..SETS {
        let Some(block) = frames.alloc(order) else {
            trouble = Some(Trouble::NoMemory);
            break;
        };
        blocks[index] = block;
        held += 1;
        // `GRANT` as well as `READ` and `WRITE`, because `iommu::Grant::map`
        // refuses without it and argues at length why a device translation is a
        // transfer rather than a read.
        match table.grant(
            CapType::Frame,
            rights::READ | rights::WRITE | rights::GRANT,
            block.addr(),
            block.bytes(),
        ) {
            Ok(handle) => caps[index] = handle.bits(),
            Err(_) => {
                trouble = Some(Trouble::Authority);
                break;
            }
        }
    }

    let counted = if let Some(why) = trouble {
        Err(why)
    } else {
        // SAFETY: the caller's guarantee. A domain of the workload's own, so
        // that nothing it maps can collide with a device the boot is driving.
        let domain = unsafe { unit.domain(frames) }.map_err(Trouble::Unit);
        match domain {
            Err(why) => Err(why),
            Ok(mut domain) => {
                // SAFETY: as above.
                let out = unsafe { churn(frames, unit, &mut domain, &table, &caps, when) };
                // The domain's tables go back whether the churn held or not: a
                // failed measurement that leaked a domain would make the *next*
                // half fail for a reason that has nothing to do with it.
                // SAFETY: as above, and nothing was ever attached to this
                // domain — no device is given to it, because this workload
                // measures the frame's bookkeeping and not a transfer.
                unsafe { unit.release(frames, domain) };
                out
            }
        }
    };

    // Given back in every case, including the failures, and last. A workload
    // that could not be arranged and also kept a quarter of a mebibyte would
    // make the boot's own frame accounting the next thing to go wrong.
    for block in blocks.iter().take(held) {
        // SAFETY: every one of these came from this allocator, at this order,
        // a few lines above, and nothing holds a translation to them — the
        // domain that did has been released.
        unsafe { frames.free(*block) };
    }

    // Last, and after the frees rather than before them: the question is
    // whether the churn gave back what it took, and a count read while it still
    // held its own blocks would answer a different one.
    counted.map(|counts| Counts { frames_before, frames_after: frames.free_count(), ..counts })
}

/// The churn proper, with everything allocated.
///
/// Two sources, because the datapath has two and they are different shapes.
/// Kept in one function and one set of counters on purpose: the question is
/// what an unmap costs, and splitting the answer by which caller asked would
/// invite a reader to believe the two paths differ where they do not.
unsafe fn churn(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    domain: &mut Domain,
    table: &Table,
    caps: &[u32; SETS],
    when: Invalidation,
) -> Result<Counts, Trouble> {
    let before =
        (unit.unmaps(), unit.pages_unmapped(), unit.unmap_invalidations(), unit.pages_mapped());
    let before_all = unit.invalidations();
    let shot = crate::smp::shootdowns();

    let mut registrations = Registrations::<SLOTS>::new();
    let mut registered = 0u64;
    let mut retired = 0u64;
    let bytes = u32::try_from(SET_PAGES * FRAME_SIZE).map_err(|_| Trouble::Registration)?;

    // Scoped, because the counters are read off the unit afterwards and the
    // grant holds it. A borrow that outlived the churn would make the answer
    // unreachable from the code that produced it.
    let (swept, reachable, standing) = {
        let mut grant =
            iommu::Grant { unit: &mut *unit, domain: &mut *domain, frames: &mut *frames, table };
        let mut churned = Churned { grant: &mut grant, when, reachable: 0, standing: 0 };

        // Half one: a client cycling its buffers. One set, registered and taken
        // back, over and over — the rate a datapath produces while nothing is
        // wrong. RFC 0024: the memory is the client's and it is entitled to
        // take it back, so this is the cost of it being right about that.
        let Some(&cap) = caps.first() else { return Err(Trouble::Registration) };
        for _ in 0..CYCLES {
            let set = registrations
                .register(cap, bytes, SET_BUFFERS, &mut churned)
                .map_err(|_| Trouble::Registration)?;
            registered += 1;
            registrations.unregister(set, &mut churned).map_err(|_| Trouble::Registration)?;
            retired += 1;
        }

        // Half two: a driver restart. Every set the dead instance held, retired
        // in one pass, which is `f_ring::registry::Table::retire_all` and the
        // call RFC 0008's teardown makes.
        for &cap in caps.iter() {
            registrations
                .register(cap, bytes, SET_BUFFERS, &mut churned)
                .map_err(|_| Trouble::Registration)?;
            registered += 1;
        }
        let swept = registrations.retire_all(&mut churned) as u64;
        retired += swept;
        (swept, churned.reachable, churned.standing)
    };

    let after =
        (unit.unmaps(), unit.pages_unmapped(), unit.unmap_invalidations(), unit.pages_mapped());
    let after_all = unit.invalidations();
    let shot_after = crate::smp::shootdowns();

    // Every set that was registered has to have been retired, and the sweep has
    // to have found the ones the second half made. Without this a run whose
    // registrations were all refused — a full table, a domain out of room —
    // would report zero unmaps, zero invalidations, and an infinite improvement.
    if retired != registered || swept == 0 {
        return Err(Trouble::Retirement);
    }

    Ok(Counts {
        registered,
        retired,
        requests: after.0.saturating_sub(before.0),
        pages: after.1.saturating_sub(before.1),
        invalidations: after.2.saturating_sub(before.2),
        pages_mapped: after.3.saturating_sub(before.3),
        // Everything the unit was made to do, less the part an unmap caused.
        // A subtraction rather than a fifth counter, because *the rest* is what
        // this is: a third counter would have to be kept in step with the
        // definition of the first two and would drift the day something else in
        // this file invalidates.
        map_invalidations: after_all
            .saturating_sub(before_all)
            .saturating_sub(after.2.saturating_sub(before.2)),
        shootdowns: shot_after.0.saturating_sub(shot.0),
        ipis: shot_after.1.saturating_sub(shot.1),
        reachable_registered: reachable,
        standing_after_unmap: standing,
        // Filled in by `run`, which is where the allocator is whole: the churn
        // still holds its own memory here.
        frames_before: 0,
        frames_after: 0,
    })
}

/// Rounds the timed pass performs, each of [`CYCLES`] register-and-retire
/// cycles.
///
/// Thirty-two, so that [`OBSERVATIONS`] is a thousand and twenty-four. The
/// counting pass does one round because a boot is evidence and one is enough of
/// it; a percentile is a different kind of number and forty observations is not
/// one. A thousand is the smallest sample where the claim's `p999` is a
/// position in the distribution rather than a synonym for the maximum, and
/// `claims/0015` states that beside the metric rather than leaving it to be
/// inferred from this constant.
const TIMED_ROUNDS: usize = 32;

/// Timed unmap requests one boot takes. Unit: observations.
pub const OBSERVATIONS: usize = TIMED_ROUNDS * CYCLES;

/// Ticks below which a bucket is one tick wide.
const EXACT: u64 = 16;

/// Sub-buckets per octave above [`EXACT`]: eight, so a reported value is within
/// 12.5% of the observation it stands for.
const SUB_BITS: u32 = 3;

/// Octaves the histogram covers: every one a `u64` tick count can occupy above
/// [`EXACT`].
const OCTAVES: usize = 60;

/// Buckets in a [`Cost`].
const BUCKETS: usize = EXACT as usize + (OCTAVES << SUB_BITS);

/// What one unmap request took, as a distribution rather than a summary.
///
/// # Why a histogram and not a mean
///
/// `claims/README.md`'s rule, applied one layer down: a mean computed at
/// collection time destroys what cannot be recovered, and under-reports exactly
/// the stalls this architecture exists to eliminate. So every observation is
/// kept, at a resolution stated in the type — 12.5% above sixteen ticks, exact
/// below it — and the percentile is computed at the end from what was seen.
///
/// # Why the kernel keeps it and `f_bench` does not
///
/// Because the cost this claim is about is on the other side of a device
/// register. `bench/src/bin/unmap_churn.rs` times the registry's arithmetic on
/// the host and says so; what it cannot reach is the page-table walk and the
/// global invalidation — two register writes and two spins on the unit clearing
/// a request bit — which are most of the number and exist only inside a boot on
/// a machine with a remapping unit. So the workload for `claims/0015` is this,
/// and the host binary is the part of it that runs where there is no unit.
///
/// # What the boot does with it
///
/// Records always; publishes only where a nanosecond may be published.
/// `kernel/src/main.rs` prints percentiles when the command line says this
/// machine is a measurement environment, and prints the refusal and its reason
/// otherwise — which is `f_bench::Environment` deciding, one privilege boundary
/// away, rather than a second rule in the kernel that could disagree with it.
#[derive(Clone, Copy)]
pub struct Cost {
    /// Observations per bucket. `u16` because [`OBSERVATIONS`] is a thousand
    /// and a `u32` array of [`BUCKETS`] would be two kilobytes of a boot
    /// processor's stack, which `kernel/linker.ld` accounts for by hand.
    counts: [u16; BUCKETS],
    /// Observations recorded. Unit: observations.
    taken: u32,
    /// The largest observation, kept exactly. Unit: ticks.
    ///
    /// Beside the buckets because the maximum is the one number a bucketed
    /// histogram should not round: a tail is what this claim is about, and a
    /// maximum reported 12.5% low is a tail nobody would go looking for.
    worst: u64,
}

/// This structure is on a boot processor's kernel stack, which `linker.ld`
/// sizes by hand and whose growth that file asks to be told about.
const _: () = assert!(core::mem::size_of::<Cost>() <= 2048);

impl Cost {
    /// An empty distribution.
    #[must_use]
    pub const fn new() -> Self {
        Self { counts: [0; BUCKETS], taken: 0, worst: 0 }
    }

    /// Which bucket `ticks` falls in.
    fn bucket(ticks: u64) -> usize {
        if ticks < EXACT {
            // Small counts are their own bucket. At a few gigahertz a tick is a
            // fraction of a nanosecond, so this is the region where a bucketed
            // reading would be reporting resolution it does not have.
            return ticks as usize;
        }
        let octave = 63 - u64::from(ticks.leading_zeros());
        let sub = (ticks >> (octave - u64::from(SUB_BITS))) & ((1 << SUB_BITS) - 1);
        EXACT as usize + (((octave as usize) - 4) << SUB_BITS) + sub as usize
    }

    /// The smallest tick count a bucket can hold.
    ///
    /// Reported rather than the largest, deliberately: a percentile must never
    /// exceed a value that was actually seen, which is the rule
    /// `bench/src/lib.rs` states for its own histogram and the reason a summary
    /// is allowed to be quoted at all.
    fn floor(index: usize) -> u64 {
        if index < EXACT as usize {
            return index as u64;
        }
        let above = index - EXACT as usize;
        let octave = ((above >> SUB_BITS) + 4) as u64;
        let sub = (above & ((1 << SUB_BITS) - 1)) as u64;
        ((1u64 << SUB_BITS) + sub) << (octave - u64::from(SUB_BITS))
    }

    /// Record one observation. Unit: ticks.
    pub fn record(&mut self, ticks: u64) {
        let index = Self::bucket(ticks);
        if let Some(slot) = self.counts.get_mut(index) {
            *slot = slot.saturating_add(1);
        }
        self.taken = self.taken.saturating_add(1);
        if ticks > self.worst {
            self.worst = ticks;
        }
    }

    /// Observations recorded. Unit: observations.
    #[must_use]
    pub const fn taken(&self) -> u32 {
        self.taken
    }

    /// The largest observation. Unit: ticks.
    #[must_use]
    pub const fn worst(&self) -> u64 {
        self.worst
    }

    /// The observation at `per_mille`, in ticks.
    ///
    /// `999` is p99.9. Per mille rather than a fraction because this kernel has
    /// no floating point on its own stack and a percentile of a thousand
    /// observations needs the third digit.
    #[must_use]
    pub fn ticks_at(&self, per_mille: u64) -> u64 {
        if self.taken == 0 {
            return 0;
        }
        // Rounded up, and at least one: the p50 of a two-observation sample is
        // the second of them, not the first.
        let target =
            (u64::from(self.taken).saturating_mul(per_mille).saturating_add(999) / 1_000).max(1);
        let mut seen = 0u64;
        for (index, count) in self.counts.iter().enumerate() {
            seen = seen.saturating_add(u64::from(*count));
            if seen >= target {
                return Self::floor(index);
            }
        }
        self.worst
    }

    /// The bucketing and the percentile, checked against their own definitions.
    ///
    /// # Why this is a boot stage and not a unit test
    ///
    /// Because there is no host to run one on: `kernel/` builds for
    /// `x86_64-unknown-none` and `cargo xtask test` cannot reach a `#[test]` in
    /// it — which is exactly the shape `lint-arch-tests` exists to keep visible.
    /// So the arithmetic underneath `claims/0015`'s percentiles is checked the
    /// way `mem::self_test` and `smp::self_test` check theirs: on every boot
    /// that takes the measurement, before it is taken.
    ///
    /// It is worth checking rather than reading. A `floor` that disagreed with
    /// its `bucket` by one octave would report a p99 half or twice the truth,
    /// on a machine nobody has yet, with nothing in the output that looked
    /// wrong — which is the failure this whole registry is built to prevent.
    ///
    /// # Errors
    ///
    /// A sentence naming which of the three properties failed.
    pub fn self_test() -> Result<(), &'static str> {
        // One: a bucket's floor is never above an observation it holds, which
        // is what lets a percentile be quoted at all — `bench/src/lib.rs`'s
        // rule that a summary never exceeds a value that was actually seen.
        // Two: it is never more than a bucket below it either, or the
        // resolution this type advertises is a fiction.
        let mut octave = 0;
        while octave < 63 {
            let base = 1u64 << octave;
            for ticks in [base, base + base / 3, base + base / 2, base.saturating_mul(2) - 1] {
                let floor = Self::floor(Self::bucket(ticks));
                if floor > ticks {
                    return Err("a bucket's floor is above an observation in it, so a percentile \
                                could exceed a value that was never seen");
                }
                // An eighth is `SUB_BITS`, and below `EXACT` the bucket is the
                // observation, so the slack is zero and this holds trivially.
                if ticks.saturating_sub(floor) > floor / 8 {
                    return Err("a bucket is wider than the resolution this type advertises, so \
                                the percentiles it reports are further from the truth than they \
                                claim to be");
                }
            }
            octave += 1;
        }

        // Three: the percentile is a position in the sample rather than an
        // average of it. Nine hundred and ninety small observations and ten
        // large ones — p50 and p99 must land in the small bucket, p999 in the
        // large one. A mean of this distribution is a hundred times the p50 and
        // would pass nothing here, which is the point.
        let mut sample = Self::new();
        let mut index = 0;
        while index < 1_000 {
            sample.record(if index < 990 { 1_024 } else { 1_048_576 });
            index += 1;
        }
        if sample.ticks_at(500) != 1_024 || sample.ticks_at(990) != 1_024 {
            return Err("the p50 or the p99 of a sample with a 1% tail is in the tail, so the \
                        percentile is not a position in the distribution");
        }
        if sample.ticks_at(999) != 1_048_576 || sample.worst() != 1_048_576 {
            return Err(
                "the p999 of a sample with a 1% tail is not in the tail, so the tail this \
                        claim is about is being averaged away",
            );
        }
        Ok(())
    }
}

impl Default for Cost {
    fn default() -> Self {
        Self::new()
    }
}

/// Nanoseconds `ticks` of the timestamp counter are, at `khz`.
///
/// # The unit, spelled out
///
/// `khz` is kilohertz, which is *ticks per millisecond*: `apic::calibrate`
/// produces it as ticks per microsecond times a thousand. So nanoseconds are
/// `ticks × 1_000_000 / khz`, and the division is done first over the whole
/// milliseconds and then over the remainder, for the reason E0-B08 recorded one
/// file over: the product overflows a `u64` at a few hours of ticks and wraps
/// rather than failing.
#[must_use]
pub const fn nanos(ticks: u64, khz: u64) -> u64 {
    if khz == 0 {
        return 0;
    }
    let millis = ticks / khz;
    let remainder = ticks % khz;
    millis.saturating_mul(1_000_000).saturating_add(remainder.saturating_mul(1_000_000) / khz)
}

/// Time one unmap request, [`OBSERVATIONS`] times, under the policy the frame
/// ships.
///
/// # What is inside the measurement and what is not
///
/// Inside: `Registrations::unregister`, which is the slot lookup, the
/// generation retirement, the in-flight word, and then `Grant::unmap` — the
/// page-table walk over the set's pages and the one global invalidation that
/// publishes it. That is the operation `claims/0015`'s statement names, taken
/// from the service's side of the ring.
///
/// Outside: the registration that precedes it, because `claims/0004` is the
/// cost of registering and averaging the two would make both claims unreadable.
///
/// # Why a clock is allowed here
///
/// RFC 0004 says nondeterminism reaches the *system* through `f_env::Env`. This
/// is not the system: it is an instrument, behind a boot parameter, and the
/// argument is `DETERMINISM_ALLOW`'s own words for `bench/` — *the harness
/// measures the system and is not part of it; a clock is what an instrument
/// is*. Two things keep that honest and both are checkable rather than
/// promised. Nothing this function measures changes what anything decides: the
/// distribution is printed, and no verdict, count or branch in the boot reads a
/// duration. And the stage runs only under `churn=unmap`, so the boot log that
/// is a fixture — the one `cargo xtask trace` hashes — contains none of it,
/// which is the boundary `boot_time` draws for the same reason.
///
/// # Errors
///
/// [`Trouble`], as [`run`].
///
/// # Safety
///
/// As [`run`].
pub unsafe fn time(frames: &mut FrameAllocator, unit: &mut Unit) -> Result<Cost, Trouble> {
    let Some(order) = Order::new(SET_ORDER) else { return Err(Trouble::NoMemory) };
    let Some(block) = frames.alloc(order) else { return Err(Trouble::NoMemory) };

    let mut table = Table::EMPTY;
    let cap = match table.grant(
        CapType::Frame,
        rights::READ | rights::WRITE | rights::GRANT,
        block.addr(),
        block.bytes(),
    ) {
        Ok(handle) => handle.bits(),
        Err(_) => {
            // SAFETY: from this allocator, at this order, and never mapped.
            unsafe { frames.free(block) };
            return Err(Trouble::Authority);
        }
    };

    // SAFETY: the caller's guarantee. A domain of the workload's own, as [`run`]
    // takes: a timing taken in a domain a device is attached to would be timing
    // whatever else that device made the unit do.
    let taken = match unsafe { unit.domain(frames) } {
        Err(why) => Err(Trouble::Unit(why)),
        Ok(mut domain) => {
            // SAFETY: as above.
            let out = unsafe { timed(frames, unit, &mut domain, &table, cap) };
            // SAFETY: as above, and nothing was ever attached to this domain.
            unsafe { unit.release(frames, domain) };
            out
        }
    };

    // SAFETY: as above; the domain that translated it has been released.
    unsafe { frames.free(block) };
    taken
}

/// [`time`], with everything allocated.
///
/// # Errors
///
/// As [`time`].
///
/// # Safety
///
/// As [`time`].
unsafe fn timed(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    domain: &mut Domain,
    table: &Table,
    cap: u32,
) -> Result<Cost, Trouble> {
    let mut cost = Cost::new();
    let mut registrations = Registrations::<SLOTS>::new();
    let bytes = u32::try_from(SET_PAGES * FRAME_SIZE).map_err(|_| Trouble::Registration)?;

    // The shipped path with no adapter around it: `iommu::Grant` is what a
    // driver's control ring reaches, and a wrapper here would put a virtual
    // call of the measurement's own making inside the number.
    let mut grant =
        iommu::Grant { unit: &mut *unit, domain: &mut *domain, frames: &mut *frames, table };

    for _ in 0..TIMED_ROUNDS {
        for _ in 0..CYCLES {
            let set = registrations
                .register(cap, bytes, SET_BUFFERS, &mut grant)
                .map_err(|_| Trouble::Registration)?;
            let start = read_tsc();
            registrations.unregister(set, &mut grant).map_err(|_| Trouble::Registration)?;
            // Saturating rather than wrapping: a counter that went backwards
            // across this pair is a machine whose timestamp counter is not
            // invariant, and a huge positive number would be a tail this claim
            // would then have to explain. Zero is the honest reading of it, and
            // a whole run of zeros is what the boot's verdict fails on.
            cost.record(read_tsc().saturating_sub(start));
        }
    }

    Ok(cost)
}

/// What a set with a hole in it costs the pages after the hole: nothing.
///
/// Unit: pages, in every field.
#[derive(Clone, Copy, Default)]
pub struct Holed {
    /// Pages the set was mapped with.
    pub mapped: u64,
    /// Pages taken out from under it before the request, one at a time, to make
    /// the hole. The request will be refused at each of them.
    pub punched: u64,
    /// Pages still translated after one batched request over the whole set.
    ///
    /// **Zero.** This is the number the check is: an unmap that stopped at the
    /// first page it could not clear would leave every page after the hole
    /// standing, which is a device still reaching memory a client took back,
    /// arrived at by an error path being tidy.
    pub standing: u64,
}

/// Unmap a set that has a hole in it, and answer what is left translated.
///
/// # Why this exists as a stage rather than as a sentence
///
/// `Unit::unmap_range` attempts every page of a request and returns the first
/// refusal at the end. It broke out of the loop on the first refusal for one
/// revision of `E1-B14`, and nothing in the tree would have noticed: no caller
/// can construct a hole today, because `Grant::map` undoes a partial mapping
/// before it refuses, so a set is wholly mapped or wholly absent. *No caller
/// can construct it* is exactly the kind of sentence that stops being true one
/// diff after somebody reads it as permission — so this constructs one, with
/// the unit's own single-page entry point, and requires the batched request
/// over the whole set to clear everything either side of it.
///
/// It is a separate pass and a separate domain on purpose: it is a correctness
/// check and not a measurement, and folding it into the counted halves would
/// put a deliberately refused request into `claims/0014`'s ratios.
///
/// # Errors
///
/// [`Trouble`], as [`run`]. A page that would not map, or a hole that could not
/// be punched, is the stage failing to be arranged rather than a finding.
///
/// # Safety
///
/// As [`run`].
pub unsafe fn hole(frames: &mut FrameAllocator, unit: &mut Unit) -> Result<Holed, Trouble> {
    let Some(order) = Order::new(SET_ORDER) else { return Err(Trouble::NoMemory) };
    let Some(block) = frames.alloc(order) else { return Err(Trouble::NoMemory) };

    let mut table = Table::EMPTY;
    let cap = match table.grant(
        CapType::Frame,
        rights::READ | rights::WRITE | rights::GRANT,
        block.addr(),
        block.bytes(),
    ) {
        Ok(handle) => handle.bits(),
        Err(_) => {
            // SAFETY: from this allocator, at this order, and never mapped.
            unsafe { frames.free(block) };
            return Err(Trouble::Authority);
        }
    };

    // SAFETY: the caller's guarantee. A domain of the workload's own, as [`run`]
    // takes, and nothing was ever attached to this domain.
    let out = match unsafe { unit.domain(frames) } {
        Err(why) => Err(Trouble::Unit(why)),
        Ok(mut domain) => {
            // SAFETY: as above.
            let out = unsafe { holed(frames, unit, &mut domain, &table, cap) };
            // SAFETY: as above, and nothing was ever attached to this domain.
            unsafe { unit.release(frames, domain) };
            out
        }
    };

    // SAFETY: as above; the domain that translated it has been released.
    unsafe { frames.free(block) };
    out
}

/// [`hole`], with everything allocated.
///
/// # Errors
///
/// As [`hole`].
///
/// # Safety
///
/// As [`hole`].
unsafe fn holed(
    frames: &mut FrameAllocator,
    unit: &mut Unit,
    domain: &mut Domain,
    table: &Table,
    cap: u32,
) -> Result<Holed, Trouble> {
    /// Which page of the set is taken out from under the request. Not the
    /// first and not the last: a hole at either end is a shorter request, and
    /// the bug this stage exists to catch is about the pages *after* it.
    const HOLE: u64 = 3;

    let bytes = u32::try_from(SET_PAGES * FRAME_SIZE).map_err(|_| Trouble::Registration)?;
    let mut grant =
        iommu::Grant { unit: &mut *unit, domain: &mut *domain, frames: &mut *frames, table };
    let base = grant.map(cap, bytes).map_err(|_| Trouble::Refused)?;

    // Every page, before anything is taken away. Without this the zero at the
    // end is a walk that answers no to everything.
    let mut mapped = 0u64;
    for page in 0..SET_PAGES {
        if translated(&grant, base, page) {
            mapped = mapped.saturating_add(1);
        }
    }
    if mapped != SET_PAGES {
        return Err(Trouble::Refused);
    }

    // The hole, made with the unit's own single-page entry point — the one
    // `Grant::map`'s undo loop uses — because no path a client can reach makes
    // one.
    let at = base.saturating_add(HOLE.saturating_mul(FRAME_SIZE));
    // SAFETY: as everything else in this file, over a translation the map above
    // just made.
    if unsafe { grant.unit.unmap(grant.frames, grant.domain, at) }.is_err() {
        return Err(Trouble::Refused);
    }

    // One request over the whole set, refused at the hole and expected to have
    // cleared everything either side of it.
    grant.unmap(cap, base, bytes);

    let mut standing = 0u64;
    for page in 0..SET_PAGES {
        if translated(&grant, base, page) {
            standing = standing.saturating_add(1);
        }
    }

    Ok(Holed { mapped, punched: 1, standing })
}

/// Is one page of the set translated in the grant's domain?
fn translated(grant: &iommu::Grant<'_>, base: u64, page: u64) -> bool {
    let at = base.saturating_add(page.saturating_mul(FRAME_SIZE));
    // SAFETY: the allocator is rebound onto the direct map of the active
    // address space, and this walks tables `iommu` made. A read.
    unsafe { grant.unit.reaches(grant.frames, grant.domain, at, FRAME_SIZE) }
}
