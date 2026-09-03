// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The remapping unit: what a device may address, and the fault it takes when
//! it tries to address anything else.
//!
//! # What this is for
//!
//! Every protection this kernel has so far is a protection against the
//! *processor* reaching memory it should not. A device is not the processor. It
//! has a bus master bit, a descriptor ring somebody filled in, and a physical
//! address it will read or write without consulting a page table — so a driver
//! at ring 3 that can program a device can address every byte of the machine,
//! and the whole capability system it is running inside is decoration. This is
//! the piece that makes that false: a device's addresses go through a second
//! set of page tables the kernel owns, and an address with no translation is a
//! fault the unit records rather than a transfer that lands.
//!
//! E1-B02, E1-B03 and E1-B04 are the three drivers that rest on it, and RFC
//! 0031 is the decision that put the emulator on a machine with one.
//!
//! # An IOMMU domain is not a speculation domain
//!
//! RFC 0005's `shared`, `private` and `hostile` are about what a *core* may
//! speculate across. A domain here is about what a *device* may address. The
//! two are different mechanisms answering different questions, and they are
//! deliberately not the same object — but they are related, and the relation is
//! worth stating because a reader will otherwise assume one:
//!
//! **One IOMMU domain per component, whatever its speculation kind.** A
//! `shared` component gets its own domain exactly as a `hostile` one does,
//! because a device that can address another component's buffers is a leak the
//! processor's speculation had no part in. RFC 0005 says a spawner may go more
//! isolated and never less; there is no *less* available here, and that
//! asymmetry is the point. A component that legitimately shares a buffer with
//! another shares it by *grant* — the same buffer mapped into both domains,
//! which is a fact recorded in two second-level page tables — rather than by
//! the two components being put in one domain.
//!
//! *Reversal:* a machine whose number of domains is smaller than its number of
//! **domain creations over a boot**, at which point domains have to be shared
//! and the sharing has to be by speculation kind, because that is the only
//! grouping already argued for. Creations and not live components, and the
//! difference is the part worth stating: [`Unit::domain`] hands ids out
//! monotonically and [`Unit::release`] does not take one back, so a component
//! RFC 0008 restarts consumes a fresh id each time. [`Unit::domains`] is the
//! number and [`Refuse::NoDomains`] is what happens at the end of it — a
//! refusal rather than a silent reuse, which is the property that matters and
//! the reason the exhaustion is bounded rather than free.
//!
//! # A domain ends when the capability that paid for it does
//!
//! RFC 0008 says everything a component is made of is retyped from one supplied
//! `Untyped`, so revoking it ends the component. A domain's second-level page
//! tables are frames like any other, and [`Domain::release`] gives back exactly
//! the ones it took — so the domain is torn down by the same revocation that
//! ends everything else, and there is no separate lifecycle for it to get out
//! of step with.
//!
//! # Everything read from this device crossed a trust boundary
//!
//! The capability and extended-capability registers are the unit describing
//! itself, and this build refuses politely rather than proceeding on anything
//! it does not implement (R04). The list is short and each entry is a real
//! machine somewhere: an address width whose page-table depth this build does
//! not construct, a unit that requires explicit write-buffer flushing, a unit
//! whose fault recording registers sit past the window this maps, and a unit
//! firmware has already enabled — the last being the one case where carrying on
//! would silently take over tables somebody else owns.
//!
//! One property on that list is *implemented* rather than refused, and it is
//! the one the machine this pins to actually asserts. `ECAP.C` — page-walk
//! coherency — says whether the unit's own reads of the root table, the context
//! tables and the second-level tables snoop the processor's caches. On the
//! emulator RFC 0031 pins to it is **clear**, and every one of those tables is
//! written through the direct map, which is write-back cacheable. So a table
//! this kernel wrote and did not flush is a table the unit may read as whatever
//! was in memory before — which under emulation never happens, because QEMU
//! reads guest RAM directly and has no cache to be behind. That is precisely
//! why it has to be handled rather than observed: the one machine that can run
//! this check cannot exhibit the failure. [`Coherency`] is the answer, and it
//! is a flush on every entry written and every table allocated, ordered ahead
//! of the invalidation that follows it.
//!
//! # Register-based invalidation, not the queue
//!
//! Every unit implements the two invalidation registers; the queued interface
//! is a performance feature and a second code path. This build uses the
//! registers, does a global invalidation after every change to a table the unit
//! walks, and pays a serialising round trip for it. That is the wrong trade for
//! a datapath and the right one for a first implementation, and the number that
//! decides when to change it is `E1-B14`'s unmap-under-churn workload.
//!
//! *Reversal:* an unmap cost that shows the round trip dominating, which is the
//! same measurement E1-B14 already owes.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use super::acpi::Dmar;
use super::paging::{self, AddressSpace, BuildError, Features};
use super::pci::Bdf;
use crate::mem::{Frame, FrameAllocator};

// --- the register file ------------------------------------------------------

/// Version. Reads as zero or all-ones when the window is not the device.
const REG_VER: u64 = 0x00;

/// What this unit can do.
const REG_CAP: u64 = 0x08;

/// What else it can do.
const REG_ECAP: u64 = 0x10;

/// The one register that changes the unit's mode.
const REG_GCMD: u64 = 0x18;

/// What the unit says its mode actually is.
const REG_GSTS: u64 = 0x1C;

/// Where the root table is.
const REG_RTADDR: u64 = 0x20;

/// The context-cache invalidation register.
const REG_CCMD: u64 = 0x28;

/// Fault status: whether anything has been recorded, and where.
const REG_FSTS: u64 = 0x34;

/// Fault event control, whose one bit this build uses is the mask.
const REG_FECTL: u64 = 0x38;

/// Translation enable, in the command register.
const GCMD_TE: u32 = 1 << 31;

/// Set root table pointer, in the command register.
const GCMD_SRTP: u32 = 1 << 30;

/// Write-buffer flush, in the command register.
const GCMD_WBF: u32 = 1 << 27;

/// Translation enable status.
const GSTS_TES: u32 = 1 << 31;

/// Root table pointer status.
const GSTS_RTPS: u32 = 1 << 30;

/// Write-buffer flush status: set while a flush is outstanding.
const GSTS_WBFS: u32 = 1 << 27;

/// Queued invalidation is enabled.
const GSTS_QIES: u32 = 1 << 26;

/// Interrupt remapping is enabled.
const GSTS_IRES: u32 = 1 << 25;

/// Mask the unit's own fault interrupt.
///
/// Set, deliberately and for the life of the boot. A fault event is a
/// message-signalled interrupt, and this kernel has installed no vector for one:
/// an unmasked fault event would be a device interrupt arriving at a gate that
/// does not exist, which is a fault *about* a fault. The fault records are read
/// by polling instead — see [`Unit::faults`] — which is the right shape for a
/// provocation that knows when it provoked, and the wrong shape for a running
/// system.
///
/// *Reversal:* E1-B02, which has a driver that would like to be told rather
/// than to ask, and by then there is a vector to deliver to.
const FECTL_IM: u32 = 1 << 30;

/// A fault has been recorded and not yet cleared.
const FSTS_PPF: u32 = 1 << 1;

/// A fault arrived while every recording register was full.
const FSTS_PFO: u32 = 1 << 0;

/// Page-walk coherency, in the extended capability register.
///
/// Set when the unit's reads of its own tables snoop the processor's caches.
/// Clear on the machine this pins to, which is why [`Coherency`] exists rather
/// than an assertion that it is set.
const ECAP_COHERENT: u64 = 1 << 0;

/// Invalidate the context cache, in the context command register.
const CCMD_ICC: u64 = 1 << 63;

/// Global granularity, in the context command register.
const CCMD_CIRG_GLOBAL: u64 = 1 << 61;

/// Invalidate, in the translation-buffer register.
const IOTLB_IVT: u64 = 1 << 63;

/// Global granularity, in the translation-buffer register.
const IOTLB_IIRG_GLOBAL: u64 = 1 << 60;

/// How many times a status bit is read before the unit is called unresponsive.
///
/// A count rather than a duration, for the reason the whole tree gives: a
/// duration needs a clock, a clock is nondeterminism, and what is being waited
/// for here is a device acknowledging a register write — which takes tens of
/// cycles on hardware and a function call under emulation. A million is six
/// orders of magnitude past either, and reaching it means the unit has stopped
/// answering rather than that the machine is slow.
const SPIN_LIMIT: u32 = 1_000_000;

// --- the second-level page tables -------------------------------------------
//
// Structurally the processor's, semantically not. `paging` owns the walk; the
// three constants below own the meaning.

/// The device may read through this entry.
///
/// Bit zero, where a processor page table has *present*. An entry with neither
/// this nor [`SL_WRITE`] is not present, which is why there is no separate
/// present bit to set: a translation nobody may use is a translation that does
/// not exist.
const SL_READ: u64 = 1 << 0;

/// The device may write through this entry.
const SL_WRITE: u64 = 1 << 1;

/// This entry is the page itself rather than a pointer to a finer table.
///
/// Bit seven, as in a processor page table, and this build never sets it: a
/// grant is mapped a page at a time. It is named because the walk has to
/// *recognise* it — a unit whose firmware left a superpage in a table this
/// kernel then descends into would be a walk into the middle of a mapping.
const SL_LARGE: u64 = 1 << 7;

/// Flags for an entry pointing at another table.
///
/// Readable and writable, because the unit takes the logical and of these bits
/// down the walk exactly as the processor does with the ring-3 bit: a leaf that
/// permits a write under a table that does not is a write the device cannot
/// make. The restriction lives in the leaf.
const SL_TABLE: u64 = SL_READ | SL_WRITE;

/// How many tables one domain may hold.
///
/// Eight. A three-level domain covering one two-mebibyte region needs three —
/// the root, the directory and the page table — and a four-level one needs
/// four; eight is that with room for a second region. It is a bound and not a
/// quota, exactly as [`paging::MAX_USER_TABLES`] is, and for the same reason: a
/// table that is not on the list is a frame that is never given back.
///
/// *Reversal:* the same one that file names — the day a component pays for its
/// own tables out of an `Untyped`, at which point this becomes an accounted
/// list. That day is E1-B05's, when a supervisor spawns a driver rather than
/// the boot path building one by hand.
pub const MAX_DOMAIN_TABLES: usize = 8;

/// Whether the unit sees what the processor wrote, and what to do when it does
/// not.
///
/// # Why this is a type rather than a bit checked at one call site
///
/// Because the obligation is not one call site. Every table this file writes —
/// the root table, a context table, a second-level entry, and the whole of a
/// freshly zeroed frame about to become one — is read by the unit rather than
/// by the processor, and a `write_volatile` through the direct map leaves it in
/// a write-back cache. On a unit that snoops, that is free and this type does
/// nothing. On a unit that does not, every one of those writes needs a flush,
/// and a build that flushed three of the four would have a bug that appears
/// once, on somebody else's machine, as a device reading a table that is
/// present in memory and stale in fact.
///
/// *Reversal:* a unit that reports coherency and lies about it, which is a
/// machine erratum rather than a design question — the answer there is to stop
/// consulting the bit, which is one line here and nothing anywhere else.
#[derive(Clone, Copy)]
struct Coherency {
    /// Whether the unit's page walks snoop the processor's caches.
    snooped: bool,
    /// The machine's own flush granularity. Unit: bytes.
    line: u64,
}

impl Coherency {
    /// What the unit said, and what the processor said about its cache lines.
    fn of(ecap: u64) -> Self {
        Self { snooped: ecap & ECAP_COHERENT != 0, line: cache_line() }
    }

    /// Push one written entry out to memory, if the unit will not see it
    /// otherwise.
    ///
    /// # Safety
    ///
    /// As [`paging::entry_flush`].
    unsafe fn entry(self, frames: &FrameAllocator, table: u64, slot: usize) {
        if self.snooped {
            return;
        }
        // SAFETY: the caller's guarantee, passed down.
        unsafe { paging::entry_flush(frames, table, slot) };
    }

    /// Push a whole freshly zeroed table out to memory.
    ///
    /// The zeroing is a write like any other: a table the allocator zeroed and
    /// nobody flushed is a table the unit may read as whatever its last owner
    /// left, and about half of those words would be *present*.
    ///
    /// # Safety
    ///
    /// As [`paging::table_flush`].
    unsafe fn table(self, frames: &FrameAllocator, at: u64) {
        if self.snooped {
            return;
        }
        // SAFETY: the caller's guarantee, passed down.
        unsafe { paging::table_flush(frames, at, self.line) };
    }

    /// Make every flush issued so far visible before whatever comes next.
    ///
    /// `clflush` is ordered against a later access only by a fence, so a build
    /// that flushed a table and then told the unit to re-read it would have
    /// written the two in an order the processor is entitled to undo. Called
    /// before the invalidation that follows every table change, which is the
    /// one place *next* is the unit rather than this program.
    fn settle(self) {
        if self.snooped {
            return;
        }
        // SAFETY: `mfence` has no operands and no memory effect the compiler
        // needs to model beyond the ordering it is being asked for.
        unsafe { core::arch::asm!("mfence", options(nostack, preserves_flags)) };
    }
}

/// The machine's flush granularity, from `cpuid`. Unit: bytes.
///
/// Leaf 1, `EBX` bits 15:8, counted in eight-byte units. A machine that reports
/// zero is answered with eight rather than with the usual sixty-four, and that
/// direction is the whole point: a stride larger than the real line size skips
/// lines, so guessing high is a flush that silently does nothing and guessing
/// low is a flush that costs instructions. R04.
fn cache_line() -> u64 {
    // SAFETY: `cpuid` is unprivileged and has no memory effect.
    let (ebx, _, _) = unsafe { super::cpuid(1) };
    let reported = u64::from((ebx >> 8) & 0xFF).saturating_mul(8);
    if reported == 0 { 8 } else { reported }
}

/// Why the unit could not be used.
///
/// Every variant is a description of the machine or of this build's limits, and
/// none is a bug in a caller. The boot path prints them and carries on without
/// an IOMMU, because a kernel that refused to boot on a machine with a
/// remapping unit it does not implement would be a kernel that boots on fewer
/// machines than one with no IOMMU support at all.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refuse {
    /// The register window could not be mapped.
    Mapping(BuildError),
    /// The version register read as zero or all-ones. Either the address
    /// firmware gave is not a remapping unit, or the mapping is not the device.
    NotResponding,
    /// Firmware has already enabled translation, interrupt remapping or the
    /// invalidation queue. Taking over a unit that is already running means
    /// re-pointing tables something else is walking, and there is no way to do
    /// that which is correct for a transfer this kernel did not arrange.
    AlreadyEnabled,
    /// The unit supports no address width whose page-table depth this build
    /// constructs. Three levels and four are built; two, five and six are not.
    NoUsableWidth,
    /// The unit requires an explicit write-buffer flush after every table
    /// change. Implemented as [`Unit::flush_write_buffer`] and refused at
    /// bring-up anyway on this build, because a flush that is never exercised
    /// is a flush nobody has checked and the emulator does not ask for one.
    RequiresWriteBuffer,
    /// The fault recording registers sit past the window this build maps.
    FaultsOutOfWindow,
    /// The `DMAR` table describes several units and at least one of them does
    /// not cover every device in its segment.
    ///
    /// Which device is behind which unit is then a question only the device
    /// scopes answer, and this build does not read them. See [`Unit::open`] for
    /// why a *single* unit without the flag is accepted and what makes that
    /// safe rather than optimistic.
    Scoped,
    /// Every domain id is in use.
    NoDomains,
    /// A domain needed more tables than [`MAX_DOMAIN_TABLES`].
    TooManyTables,
    /// A translation was asked for at an address a larger mapping already
    /// covers, or where one already exists.
    Overlap,
    /// An unmap was asked for where nothing is mapped.
    NotMapped,
    /// The unit did not answer a register write within [`SPIN_LIMIT`].
    Stuck,
    /// The allocator had no frame for a table.
    NoFrames,
}

impl Refuse {
    /// A sentence for the boot log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Mapping(e) => e.message(),
            Self::NotResponding => "the remapping unit's version register is not a version",
            Self::AlreadyEnabled => "firmware left the remapping unit enabled",
            Self::NoUsableWidth => "the unit offers no address width this build builds tables for",
            Self::RequiresWriteBuffer => "the unit requires write-buffer flushing, untested here",
            Self::FaultsOutOfWindow => "the fault records are past the window this build maps",
            Self::Scoped => "several units, at least one scoped, and scopes are not read",
            Self::NoDomains => "every domain id is in use",
            Self::TooManyTables => "a domain needs more tables than are tracked",
            Self::Overlap => "a translation was asked for where one already is",
            Self::NotMapped => "an unmap was asked for where nothing is mapped",
            Self::Stuck => "the remapping unit did not answer a register write",
            Self::NoFrames => "no frame for a remapping table",
        }
    }
}

impl From<BuildError> for Refuse {
    fn from(e: BuildError) -> Self {
        match e {
            BuildError::NoFrames => Self::NoFrames,
            BuildError::Overlap => Self::Overlap,
            other => Self::Mapping(other),
        }
    }
}

/// One domain: a device address space, and the frames it is built from.
pub struct Domain {
    id: u16,
    root: u64,
    levels: u8,
    tables: [Frame; MAX_DOMAIN_TABLES],
    count: usize,
    pages: u32,
}

impl Domain {
    /// Which domain id the context entries carry.
    #[must_use]
    pub const fn id(&self) -> u16 {
        self.id
    }

    /// How many pages are translated in it right now.
    #[must_use]
    pub const fn pages(&self) -> u32 {
        self.pages
    }

    /// Every frame this domain is built from, so a teardown gives back exactly
    /// what it took.
    #[must_use]
    pub fn tables(&self) -> &[Frame] {
        self.tables.get(..self.count).unwrap_or(&[])
    }

    /// Note a frame as one this domain will have to give back.
    fn record(&mut self, frame: Frame) -> Result<(), Refuse> {
        let slot = self.tables.get_mut(self.count).ok_or(Refuse::TooManyTables)?;
        *slot = frame;
        self.count = self.count.saturating_add(1);
        Ok(())
    }

    /// The shift of each level of this domain's walk, outermost first.
    ///
    /// Three levels reach 39 bits and four reach 48. The array is fixed and the
    /// slice is taken from the end, so a three-level walk starts at 30 and a
    /// four-level one at 39 — which is the same arithmetic the processor's
    /// tables use and is why [`paging::level_slot`] serves both.
    fn shifts(&self) -> &'static [u32] {
        const ALL: [u32; 4] = [39, 30, 21, 12];
        if self.levels >= 4 { &ALL } else { ALL.get(1..).unwrap_or(&ALL) }
    }
}

/// What the fault recording registers said.
#[derive(Clone, Copy, Debug)]
pub struct Fault {
    /// The requester that faulted, as a packed bus, device and function.
    pub source: u16,
    /// Which address it asked for. Unit: bytes, in the device's address space.
    pub address: u64,
    /// The unit's own reason code. `0x05` is the one this file provokes: no
    /// translation for the address in the domain's second-level tables.
    pub reason: u8,
    /// Whether the faulting transaction was a read. A write otherwise.
    pub read: bool,
}

/// What a poll of the fault registers found.
#[derive(Clone, Copy, Debug)]
pub struct Faults {
    /// The first record, if there was one.
    pub first: Option<Fault>,
    /// How many records were found and cleared this time.
    pub records: u32,
    /// Whether a fault arrived while every recording register was full, which
    /// means faults were lost. Reported rather than hidden: a count that
    /// silently stops rising is worse than a count with a flag beside it.
    pub overflowed: bool,
}

/// A remapping unit, mapped and described.
pub struct Unit {
    /// Where the register window is.
    regs: u64,
    /// What the unit said about itself.
    cap: u64,
    ecap: u64,
    /// Offset of the first fault recording register, in bytes from `regs`.
    fault_offset: u64,
    /// How many fault recording registers there are.
    fault_records: u32,
    /// Offset of the translation-buffer invalidation register.
    iotlb_offset: u64,
    /// How many domain ids the unit has.
    ///
    /// A `u32` and not a `u16`, because the largest value the field can name is
    /// 65 536 — one more than a `u16` holds. Counting it in the narrower type
    /// would report 65 535 on the machine with the most domains, which is a
    /// number that is wrong in the direction nobody would question.
    domains: u32,
    /// The next domain id to hand out. Starts at one: zero is reserved by the
    /// architecture when the unit is in caching mode, and a build that used it
    /// on units that are not would have one machine where it is wrong.
    next_domain: u32,
    /// How many levels a second-level walk has on this unit.
    levels: u8,
    /// The width those levels reach. Unit: bits.
    width: u8,
    /// Whether the unit sees this kernel's table writes without help, and what
    /// to do when it does not.
    coherency: Coherency,
    /// This kernel's own copy of the persistent command bits.
    ///
    /// Kept rather than read back out of the status register, and the reason is
    /// that the two registers are not mirrors: several command bits are
    /// one-shot and their status counterparts report a completion rather than a
    /// setting. Writing back what the status register said would re-issue a
    /// command that had already completed.
    gcmd: u32,
    /// Physical address of the root table.
    root: u64,
    /// Every frame the unit itself is built from: the root table and one
    /// context table per bus that has been attached.
    tables: [Frame; MAX_BUS_TABLES],
    table_count: usize,
    /// Which bus each context table belongs to, in the same order.
    buses: [u8; MAX_BUS_TABLES],
    bus_count: usize,
}

/// How many buses may have a context table.
///
/// Four. One per bus that carries a device this kernel has attached to a
/// domain, and every machine this boots on puts its devices on bus zero. The
/// bound is a cost of the same kind as [`MAX_DOMAIN_TABLES`]: a fifth bus is a
/// refusal rather than a leak. The root table itself takes the first slot, which
/// is why this is one more than the number of buses.
const MAX_BUS_TABLES: usize = 5;

impl Unit {
    /// Bring up the unit `DMAR` described.
    ///
    /// # Errors
    ///
    /// [`Refuse`], every variant of which leaves the unit exactly as firmware
    /// left it: nothing here writes a register before every check has passed,
    /// which is what makes a refusal safe to carry on from.
    ///
    /// # Which unit a device is behind
    ///
    /// A `DMAR` table with several units divides the segment between them by
    /// device scope, and this build does not read scopes — so it refuses,
    /// because a unit applied to devices it was not described for is worse than
    /// no unit at all.
    ///
    /// A table with **one** unit is accepted whether or not it carries the
    /// include-all flag, and that is a judgement rather than a reading of the
    /// specification. Strictly, a device outside a scoped unit's scope has no
    /// unit; the flag is how firmware says *and everything else is mine*. QEMU
    /// describes its unit with an IOAPIC scope and no flag, and translates for
    /// every device behind it regardless, so a build that refused on the flag
    /// alone would refuse on the only machine this project can boot.
    ///
    /// What makes that safe rather than optimistic is that it is *checked*: a
    /// device this build attaches to a domain and then provokes must fault, and
    /// `dma=outside` fails the boot if it does not. An assumption a boot
    /// falsifies is a different kind of object from an assumption a comment
    /// asserts.
    ///
    /// *Reversal:* a machine with one scoped unit that genuinely does not cover
    /// the device this kernel attached, which shows up as `dma=outside`
    /// completing — at which point the scopes have to be read.
    ///
    /// # Safety
    ///
    /// `space` must be the address space currently in `CR3`, `frames` must be
    /// rebound onto its direct map, and `dmar` must have come from a `DMAR`
    /// table that validated — a register base read out of a table whose
    /// checksum was not checked is an arbitrary physical address about to be
    /// mapped as a device.
    pub unsafe fn open(
        frames: &mut FrameAllocator,
        space: &AddressSpace,
        features: Features,
        dmar: &Dmar,
    ) -> Result<Self, Refuse> {
        let drhd = &dmar.unit;
        if !drhd.include_all && dmar.units != 1 {
            return Err(Refuse::Scoped);
        }

        // The first page, so that the capability registers can be read. How
        // much more of the window is needed is a function of what they say.
        // SAFETY: the caller's guarantee, and `drhd.register_base` came from a
        // checksummed table describing device registers rather than memory.
        let regs = unsafe { paging::map_device(frames, space, drhd.register_base, features) }
            .map_err(Refuse::from)?;

        // SAFETY: `regs` is the window just mapped and `REG_VER` is a defined
        // register within its first page.
        let version = unsafe { read32(regs, REG_VER) };
        // Zero is a unit that is not there; all-ones is a mapping that reads as
        // a bus with nothing on it. Neither is a version, and believing either
        // would mean reading a capability register out of the same nothing.
        if version == 0 || version == u32::MAX {
            return Err(Refuse::NotResponding);
        }

        // SAFETY: as above; both are defined 64-bit registers in the first page.
        let cap = unsafe { read64(regs, REG_CAP) };
        // SAFETY: as above.
        let ecap = unsafe { read64(regs, REG_ECAP) };

        // Refuse before writing anything. A unit firmware has already enabled
        // is walking tables this kernel does not own, and there is no correct
        // way to take that over that does not begin with knowing what the
        // previous owner had mapped.
        // SAFETY: as above.
        let status = unsafe { read32(regs, REG_GSTS) };
        if status & (GSTS_TES | GSTS_QIES | GSTS_IRES) != 0 {
            return Err(Refuse::AlreadyEnabled);
        }

        // Required write-buffer flushing. Implemented below and refused here:
        // the emulator does not set this bit, so a build that accepted it would
        // ship a flush that has never run, on the one path where not running it
        // is a table the unit reads stale.
        if cap & (1 << 4) != 0 {
            return Err(Refuse::RequiresWriteBuffer);
        }

        // Page-walk coherency. Not a refusal, and it is the one property on
        // this list that the machine RFC 0031 pins to asserts *false* — see the
        // module comment and [`Coherency`]. It is decided here, beside the
        // refusals, because a reader of this list would otherwise conclude the
        // question was never asked.
        //
        // *Reversal:* a measurement showing the flushes dominating a map, which
        // is E1-B14's unmap-under-churn workload again. The alternative it
        // would buy is mapping the tables uncacheable instead, which trades a
        // flush per write for every read of a table being a memory reference.
        let coherency = Coherency::of(ecap);

        // The supported address widths, as a bitmap where bit `n` means
        // 30 + 9n bits and n + 2 levels. Three levels are preferred over four
        // where both are offered: a level is a memory reference on every
        // translation the unit does not have cached, and 39 bits reaches five
        // hundred and twelve gibibytes, which is past every machine this kernel
        // has a memory map for.
        let sagaw = (cap >> 8) & 0x1F;
        let (levels, width) = if sagaw & (1 << 1) != 0 {
            (3u8, 39u8)
        } else if sagaw & (1 << 2) != 0 {
            (4u8, 48u8)
        } else {
            return Err(Refuse::NoUsableWidth);
        };

        // The maximum address the unit will accept, which is a *different*
        // number from the width its page tables reach and is allowed to be
        // smaller. A build that bounded device addresses by the table depth
        // alone would install translations the unit refuses at walk time —
        // turning a grant the frame believes it made into a fault at first use,
        // which is the one failure mode in this file indistinguishable from the
        // failure it exists to detect. The narrower of the two is the bound.
        let mgaw = ((cap >> 16) & 0x3F).saturating_add(1);
        let width = u8::try_from(mgaw).unwrap_or(width).min(width);
        if width < 12 {
            return Err(Refuse::NoUsableWidth);
        }

        // Number of domains: the field is an exponent, and the smallest legal
        // value is sixteen. Saturating into a `u16` rather than widening,
        // because a unit reporting the largest field value has 65 536 domains
        // and this build's counter is the thing that would overflow.
        // The field is an exponent and the smallest legal value is sixteen.
        // Seven is reserved and is treated here as *the most this build will
        // hand out* rather than as an error, because a unit reporting it has
        // more domains than the sixteen-bit id in a context entry can name — so
        // what binds is the id space rather than the field.
        let nd = u32::from(cap as u8 & 0x7);
        let domains = 1u64.checked_shl(4 + 2 * nd).unwrap_or(0);
        let domains = u32::try_from(domains.min(u64::from(u16::MAX) + 1)).unwrap_or(0);

        let fault_offset = ((cap >> 24) & 0x3FF).wrapping_mul(16);
        let fault_records = (((cap >> 40) & 0xFF) as u32).saturating_add(1);
        let iotlb_offset = ((ecap >> 8) & 0x3FF).wrapping_mul(16);

        // How much of the register window has to be readable. The fault records
        // are the far end of it on every unit, and this build maps whole pages
        // — so the question is how many pages, and the answer being larger than
        // a small bound is a unit laid out unlike any this was written against.
        let fault_end = fault_offset.saturating_add(u64::from(fault_records).saturating_mul(16));
        let iotlb_end = iotlb_offset.saturating_add(16);
        let span = fault_end.max(iotlb_end).max(0x100);
        let pages = span.div_ceil(4096);
        if pages > MAX_REGISTER_PAGES {
            return Err(Refuse::FaultsOutOfWindow);
        }
        for page in 1..pages {
            let at = drhd.register_base.wrapping_add(page.wrapping_mul(4096));
            // SAFETY: as the first page: the caller's guarantee, and an address
            // inside the register window firmware described.
            unsafe { paging::map_device(frames, space, at, features) }.map_err(Refuse::from)?;
        }

        // SAFETY: the caller's guarantee that frames are addressable through
        // the direct map.
        let root = unsafe { paging::fresh_table(frames) }.map_err(Refuse::from)?;
        // SAFETY: `root` was just allocated by this module and is addressable
        // through the direct map. A unit that does not snoop would otherwise
        // read this table as whatever its last owner left.
        unsafe { coherency.table(frames, root) };

        let mut unit = Self {
            regs,
            cap,
            ecap,
            fault_offset,
            fault_records,
            iotlb_offset,
            domains,
            next_domain: 1,
            levels,
            width,
            coherency,
            gcmd: 0,
            root,
            tables: [Frame::from_addr(0); MAX_BUS_TABLES],
            table_count: 0,
            buses: [0; MAX_BUS_TABLES],
            bus_count: 0,
        };
        unit.record_table(Frame::from_addr(root))?;

        // The fault interrupt, masked, before anything can fault. See
        // [`FECTL_IM`].
        // SAFETY: `regs` is the mapped window and `REG_FECTL` is a defined
        // register in its first page.
        unsafe { write32(regs, REG_FECTL, FECTL_IM) };

        // Any fault firmware left recorded is not this kernel's, and leaving it
        // there would make the first poll after the first provocation report a
        // fault that predates the provocation.
        unit.clear_faults();

        Ok(unit)
    }

    /// The unit's capability register, for a caller that wants to print it.
    #[must_use]
    pub const fn capability(&self) -> u64 {
        self.cap
    }

    /// The unit's extended capability register.
    #[must_use]
    pub const fn extended(&self) -> u64 {
        self.ecap
    }

    /// How many address bits a translation covers on this unit. Unit: bits.
    #[must_use]
    pub const fn width(&self) -> u8 {
        self.width
    }

    /// How many levels a second-level walk has.
    #[must_use]
    pub const fn levels(&self) -> u8 {
        self.levels
    }

    /// How many domain ids the unit has.
    #[must_use]
    pub const fn domains(&self) -> u32 {
        self.domains
    }

    /// How many domain ids have been handed out.
    #[must_use]
    pub const fn domains_used(&self) -> u32 {
        self.next_domain.saturating_sub(1)
    }

    /// Whether the unit's page walks snoop the processor's caches.
    ///
    /// Reported so that the boot log says which of the two ways this kernel is
    /// keeping its tables visible to the unit. On a machine where this is
    /// false — which is the machine RFC 0031 pins to — every entry written and
    /// every table allocated is flushed by hand, and an emulator cannot tell
    /// the two builds apart. A log line can.
    #[must_use]
    pub const fn coherent(&self) -> bool {
        self.coherency.snooped
    }

    /// Whether the unit is in caching mode, where a not-present entry is cached
    /// and every change to one has to be invalidated.
    ///
    /// This build invalidates globally after every change either way, so the bit
    /// changes nothing about what it does. It is reported because a reader
    /// comparing this against a kernel that *does* optimise the invalidation
    /// away needs to know which mode the measurement was taken in.
    #[must_use]
    pub const fn caching_mode(&self) -> bool {
        self.cap & (1 << 7) != 0
    }

    /// Allocate a zeroed table the unit can be pointed at.
    ///
    /// [`paging::fresh_table`] plus the flush a unit that does not snoop needs.
    /// Every table in this file comes through here rather than through `paging`
    /// directly, because a table allocated on one path and flushed on the other
    /// is the bug [`Coherency`] exists to make impossible to write.
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the active space.
    unsafe fn new_table(&self, frames: &mut FrameAllocator) -> Result<u64, Refuse> {
        // SAFETY: the caller's guarantee, passed down.
        let at = unsafe { paging::fresh_table(frames) }.map_err(Refuse::from)?;
        // SAFETY: `at` was just allocated by this module.
        unsafe { self.coherency.table(frames, at) };
        Ok(at)
    }

    /// Write one entry of a table the unit walks.
    ///
    /// The one write path in this file, for the reason [`Unit::new_table`] is
    /// the one allocation path.
    ///
    /// # Safety
    ///
    /// As [`paging::entry_write`]: `table` must be a frame this module
    /// allocated and `slot` must be below 512.
    unsafe fn put(&self, frames: &FrameAllocator, table: u64, slot: usize, entry: u64) {
        // SAFETY: the caller's guarantee, passed down.
        unsafe { paging::entry_write(frames, table, slot, entry) };
        // SAFETY: as above, over the entry just written.
        unsafe { self.coherency.entry(frames, table, slot) };
    }

    fn record_table(&mut self, frame: Frame) -> Result<(), Refuse> {
        let slot = self.tables.get_mut(self.table_count).ok_or(Refuse::TooManyTables)?;
        *slot = frame;
        self.table_count = self.table_count.saturating_add(1);
        Ok(())
    }

    /// Every frame the unit's own tables are built from.
    #[must_use]
    pub fn tables(&self) -> &[Frame] {
        self.tables.get(..self.table_count).unwrap_or(&[])
    }

    /// Take a domain id out of the space, or refuse.
    ///
    /// # Errors
    ///
    /// [`Refuse::NoDomains`] when the space is exhausted, [`Refuse::NoFrames`]
    /// when there is no frame for the root table.
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the active space.
    pub unsafe fn domain(&mut self, frames: &mut FrameAllocator) -> Result<Domain, Refuse> {
        // A structured refusal rather than a wrap. A domain id handed out twice
        // is two components sharing one set of second-level tables, which is
        // exactly the isolation this whole module exists to provide — and it
        // would be invisible, because both components would work.
        //
        // Monotonic, and never reclaimed. So what this bounds is the number of
        // domains *created* over a boot rather than the number alive at once,
        // and a component restarted into a fresh epoch spends an id each time.
        // Nothing here reclaims one because reclaiming means a free list, and a
        // free list is the structure that has to agree with the context tables
        // about which ids are live — a second record of one fact, which this
        // file refuses everywhere else. At 65 536 ids the cost is a boot long
        // enough to restart a driver that many times.
        //
        // *Reversal:* a supervisor that restarts components in a loop, which is
        // E1-B05's; the answer there is a free list paid for out of the same
        // `Untyped` the domain's tables come from, so that the record and the
        // payment are one object.
        if self.next_domain >= self.domains || self.next_domain > u32::from(u16::MAX) {
            return Err(Refuse::NoDomains);
        }
        // SAFETY: the caller's guarantee that frames are addressable.
        let root = unsafe { self.new_table(frames) }?;

        let id = u16::try_from(self.next_domain).map_err(|_| Refuse::NoDomains)?;
        self.next_domain = self.next_domain.saturating_add(1);

        let mut domain = Domain {
            id,
            root,
            levels: self.levels,
            tables: [Frame::from_addr(0); MAX_DOMAIN_TABLES],
            count: 0,
            pages: 0,
        };
        domain.record(Frame::from_addr(root))?;
        Ok(domain)
    }

    /// Give a domain's frames back.
    ///
    /// Nothing is invalidated here and nothing is detached: the caller must
    /// have detached every device first, or the unit would be walking freed
    /// memory. That ordering is the caller's because it is the same ordering
    /// RFC 0008 already imposes on ending a component — the control ring stops
    /// before the memory goes back — and duplicating it here would be a second
    /// place it could be got wrong.
    ///
    /// # Safety
    ///
    /// No device may be attached to `domain`, and `frames` must be the
    /// allocator the domain's tables came from.
    pub unsafe fn release(&mut self, frames: &mut FrameAllocator, domain: Domain) {
        for frame in domain.tables() {
            // SAFETY: every frame here was allocated by this module for this
            // domain alone, and the caller has guaranteed no device is still
            // walking it.
            unsafe { frames.free(*frame) };
        }
    }

    /// Give one function's requests to one domain.
    ///
    /// # Errors
    ///
    /// [`Refuse`].
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the active space.
    pub unsafe fn attach(
        &mut self,
        frames: &mut FrameAllocator,
        bdf: Bdf,
        domain: &Domain,
    ) -> Result<(), Refuse> {
        let bus = usize::from(bdf.bus);
        // The root table is 256 entries of sixteen bytes, which is two 512-entry
        // rows of the eight-byte slots `paging` addresses. So a bus's entry is
        // at slot `2 * bus`, and the odd slot beside it is the upper half the
        // architecture reserves.
        let slot = bus.checked_mul(2).ok_or(Refuse::Overlap)?;

        // SAFETY: `self.root` is a table this module allocated and `slot` is
        // below 512 because a bus number is below 256.
        let existing = unsafe { paging::entry_read(frames, self.root, slot) };
        let context = if existing & 1 != 0 {
            existing & paging::ENTRY_ADDRESS
        } else {
            // SAFETY: the caller's guarantee that frames are addressable.
            let fresh = unsafe { self.new_table(frames) }?;
            self.record_table(Frame::from_addr(fresh))?;
            let note = self.buses.get_mut(self.bus_count).ok_or(Refuse::TooManyTables)?;
            *note = bdf.bus;
            self.bus_count = self.bus_count.saturating_add(1);
            // Present, and the address. There is nothing else in a root entry.
            // SAFETY: as above, into an entry that was not present.
            unsafe { self.put(frames, self.root, slot, fresh | 1) };
            fresh
        };

        // A context entry is also sixteen bytes, indexed by device and function
        // together — which is the low eight bits of the requester id.
        let devfn = usize::from(bdf.source_id() & 0xFF);
        let low_slot = devfn.checked_mul(2).ok_or(Refuse::Overlap)?;
        let high_slot = low_slot.saturating_add(1);

        // Translation type zero: an untranslated request walks the second-level
        // tables. The other two types are pass-through, which is the absence of
        // the protection this module exists for, and device-translation, which
        // needs a device that does its own walking.
        let low = (domain.root & paging::ENTRY_ADDRESS) | 1;
        // The address-width field counts levels the same way the supported-width
        // bitmap does: two levels is zero, so three levels is one and four is
        // two.
        let aw = u64::from(self.levels.saturating_sub(2));
        let high = aw | (u64::from(domain.id) << 8);

        // The high half first. A context entry becomes live the instant its
        // present bit is set, and a unit that read a present entry whose domain
        // id and width had not been written yet would walk with a width of zero
        // — two levels — into a table this build made three levels deep.
        // SAFETY: `context` is a table this module allocated, and both slots are
        // below 512 because `devfn` is below 256.
        unsafe { self.put(frames, context, high_slot, high) };
        // SAFETY: as above.
        unsafe { self.put(frames, context, low_slot, low) };

        self.invalidate()
    }

    /// Take one function's requests away from whatever domain they were in.
    ///
    /// The function goes back to having no context entry, which under an
    /// enabled unit means it can address nothing: a detached device is a device
    /// that faults, and that asymmetry is deliberate. The alternative — leaving
    /// the entry and freeing the tables under it — is the one bug in this file
    /// that a teardown could introduce and that nothing would report, because it
    /// is a device walking memory somebody else now owns.
    ///
    /// The root entry stays. A bus's context table is the unit's, not a
    /// component's, and freeing it would mean proving no other function on that
    /// bus is attached — a scan of 256 entries per detach, to reclaim one frame
    /// per bus for the life of the machine.
    ///
    /// # Errors
    ///
    /// [`Refuse::NotMapped`] for a function that was not attached, which is a
    /// bug in the caller's own bookkeeping rather than something a component can
    /// provoke, and [`Refuse::Stuck`] if the unit does not acknowledge the
    /// invalidation.
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the active space.
    pub unsafe fn detach(&mut self, frames: &mut FrameAllocator, bdf: Bdf) -> Result<(), Refuse> {
        let slot = usize::from(bdf.bus).checked_mul(2).ok_or(Refuse::Overlap)?;
        // SAFETY: `self.root` is a table this module allocated and `slot` is
        // below 512 because a bus number is below 256.
        let existing = unsafe { paging::entry_read(frames, self.root, slot) };
        if existing & 1 == 0 {
            return Err(Refuse::NotMapped);
        }
        let context = existing & paging::ENTRY_ADDRESS;

        let devfn = usize::from(bdf.source_id() & 0xFF);
        let low_slot = devfn.checked_mul(2).ok_or(Refuse::Overlap)?;
        let high_slot = low_slot.saturating_add(1);

        // The present bit first, and the rest after. The mirror of `attach`'s
        // order and for the mirror of its reason: an entry whose domain id was
        // cleared while it was still present is an entry the unit may walk into
        // domain zero.
        // SAFETY: `context` is a table this module allocated and both slots are
        // below 512 because `devfn` is below 256.
        unsafe { self.put(frames, context, low_slot, 0) };
        // SAFETY: as above.
        unsafe { self.put(frames, context, high_slot, 0) };

        self.invalidate()
    }

    /// Add one page of translation to a domain.
    ///
    /// `device` is the address the device will use and `phys` is where the
    /// memory actually is. Both are page-aligned or this refuses: a grant that
    /// silently rounded would translate bytes on either side of what was asked
    /// for, which is the whole failure this module prevents, made by arithmetic.
    ///
    /// # Errors
    ///
    /// [`Refuse::Overlap`] where something is already translated,
    /// [`Refuse::NoFrames`] or [`Refuse::TooManyTables`] where a table cannot
    /// be made or recorded.
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the active space, and
    /// `phys` must name memory the caller is entitled to hand to a device.
    pub unsafe fn map(
        &mut self,
        frames: &mut FrameAllocator,
        domain: &mut Domain,
        device: u64,
        phys: u64,
        writable: bool,
    ) -> Result<(), Refuse> {
        if !device.is_multiple_of(4096) || !phys.is_multiple_of(4096) {
            return Err(Refuse::Overlap);
        }
        if device >= (1u64 << self.width) {
            return Err(Refuse::Overlap);
        }

        let mut at = domain.root;
        let shifts = domain.shifts();
        let last = shifts.len().saturating_sub(1);
        for (index, shift) in shifts.iter().enumerate() {
            let slot = paging::level_slot(device, *shift);
            if index == last {
                // SAFETY: `at` is a table this module allocated and `slot` is
                // below 512 by construction of `level_slot`.
                let existing = unsafe { paging::entry_read(frames, at, slot) };
                if existing & (SL_READ | SL_WRITE) != 0 {
                    return Err(Refuse::Overlap);
                }
                let write = if writable { SL_WRITE } else { 0 };
                // SAFETY: as above, into an entry that was not present.
                unsafe { self.put(frames, at, slot, phys | SL_READ | write) };
                domain.pages = domain.pages.saturating_add(1);
                break;
            }

            // SAFETY: as above.
            let existing = unsafe { paging::entry_read(frames, at, slot) };
            if existing & (SL_READ | SL_WRITE) != 0 {
                if existing & SL_LARGE != 0 {
                    return Err(Refuse::Overlap);
                }
                at = existing & paging::ENTRY_ADDRESS;
                continue;
            }
            // SAFETY: the caller's guarantee that frames are addressable.
            let fresh = unsafe { self.new_table(frames) }?;
            domain.record(Frame::from_addr(fresh))?;
            // SAFETY: as above, into an entry that was not present.
            unsafe { self.put(frames, at, slot, fresh | SL_TABLE) };
            at = fresh;
        }

        self.invalidate()
    }

    /// Take one page of translation away again.
    ///
    /// # Errors
    ///
    /// [`Refuse::NotMapped`] where nothing is translated, [`Refuse::Overlap`]
    /// where a larger mapping covers the address.
    ///
    /// # Safety
    ///
    /// As [`Unit::map`].
    pub unsafe fn unmap(
        &mut self,
        frames: &mut FrameAllocator,
        domain: &mut Domain,
        device: u64,
    ) -> Result<(), Refuse> {
        let mut at = domain.root;
        let shifts = domain.shifts();
        let last = shifts.len().saturating_sub(1);
        for (index, shift) in shifts.iter().enumerate() {
            let slot = paging::level_slot(device, *shift);
            // SAFETY: `at` is a table this module allocated and `slot` is below
            // 512.
            let existing = unsafe { paging::entry_read(frames, at, slot) };
            if index == last {
                if existing & (SL_READ | SL_WRITE) == 0 {
                    return Err(Refuse::NotMapped);
                }
                // SAFETY: as above, over an entry that was present.
                unsafe { self.put(frames, at, slot, 0) };
                domain.pages = domain.pages.saturating_sub(1);
                break;
            }
            if existing & (SL_READ | SL_WRITE) == 0 {
                return Err(Refuse::NotMapped);
            }
            if existing & SL_LARGE != 0 {
                return Err(Refuse::Overlap);
            }
            at = existing & paging::ENTRY_ADDRESS;
        }

        // The tables above the leaf are left in place. A pass that freed an
        // empty table would have to prove it is empty, which is a scan of 512
        // entries per unmap — the same trade `mem::coalesce` makes and refuses
        // to make on the hot path. They are freed when the domain is released,
        // which is the point at which the whole answer is known.
        self.invalidate()
    }

    /// Does this domain translate every byte of `len` at `device`?
    ///
    /// The question [`f_ring::registry::PageWalk`] asks, answered by walking
    /// the same tables the unit walks. Deliberately a *read* of the tables
    /// rather than a record kept beside them: a second structure recording what
    /// is mapped is a second structure that can disagree with the first, and
    /// this one would disagree in the direction that says a device reaches
    /// memory it does not.
    ///
    /// # Safety
    ///
    /// `frames` must be rebound onto the direct map of the active space.
    #[must_use]
    pub unsafe fn reaches(
        &self,
        frames: &FrameAllocator,
        domain: &Domain,
        device: u64,
        len: u64,
    ) -> bool {
        // Zero bytes at any address is reached vacuously, which is the same
        // answer RFC 0024 gives for a zero-length operation: valid, and distinct
        // from an absent one.
        if len == 0 {
            return true;
        }
        let Some(end) = device.checked_add(len) else { return false };
        let mut page = device & !0xFFFu64;
        while page < end {
            // SAFETY: the caller's guarantee, passed down.
            if !unsafe { self.translates(frames, domain, page) } {
                return false;
            }
            let Some(next) = page.checked_add(4096) else { return false };
            page = next;
        }
        true
    }

    /// Is one page translated in this domain?
    ///
    /// # Safety
    ///
    /// As [`Unit::reaches`].
    unsafe fn translates(&self, frames: &FrameAllocator, domain: &Domain, page: u64) -> bool {
        if page >= (1u64 << self.width) {
            return false;
        }
        let mut at = domain.root;
        let shifts = domain.shifts();
        let last = shifts.len().saturating_sub(1);
        for (index, shift) in shifts.iter().enumerate() {
            let slot = paging::level_slot(page, *shift);
            // SAFETY: `at` is a table this module allocated and `slot` is below
            // 512.
            let existing = unsafe { paging::entry_read(frames, at, slot) };
            if existing & (SL_READ | SL_WRITE) == 0 {
                return false;
            }
            if index == last {
                return true;
            }
            if existing & SL_LARGE != 0 {
                // A larger mapping covers it. True, and this build never makes
                // one — so reaching here means firmware left something in a
                // table this kernel then used, which is worth not treating as
                // an ordinary success.
                return false;
            }
            at = existing & paging::ENTRY_ADDRESS;
        }
        false
    }

    /// Point the unit at the root table and turn translation on.
    ///
    /// # What changes at the instant this returns
    ///
    /// Every device behind this unit that has no context entry stops being able
    /// to address memory. That is the intended effect and it is also the
    /// sharpest edge in this file: a device that was mid-transfer takes a fault
    /// rather than finishing. Nothing this kernel drives is mid-transfer at
    /// boot, and the reason that is true is that nothing this kernel drives
    /// does DMA at all — the serial port, the interrupt controller and the
    /// timer are all registers.
    ///
    /// *Reversal:* firmware that leaves a device transferring, which is a
    /// machine with a boot-time storage controller. E5's problem, and the
    /// answer there is an identity domain installed before translation is
    /// enabled rather than a later enable.
    ///
    /// # Errors
    ///
    /// [`Refuse::Stuck`] if the unit does not acknowledge.
    ///
    /// # Safety
    ///
    /// Every device that must keep working has to have been attached first.
    pub unsafe fn enable(&mut self) -> Result<(), Refuse> {
        // The root table and every context table under it were flushed as they
        // were written; this is what orders those flushes ahead of the register
        // write that tells the unit to go and read them.
        self.coherency.settle();
        // Legacy root table: the mode field in the low bits is zero.
        // SAFETY: `self.regs` is the mapped window; the root-table address
        // register is a defined 64-bit register in its first page.
        unsafe { write64(self.regs, REG_RTADDR, self.root) };

        self.command(GCMD_SRTP, GSTS_RTPS, true)?;
        self.invalidate()?;
        self.command(GCMD_TE, GSTS_TES, true)?;
        Ok(())
    }

    /// Turn translation off again.
    ///
    /// # Errors
    ///
    /// [`Refuse::Stuck`].
    pub fn disable(&mut self) -> Result<(), Refuse> {
        self.command(GCMD_TE, GSTS_TES, false)
    }

    /// Is translation on?
    #[must_use]
    pub fn enabled(&self) -> bool {
        // SAFETY: `self.regs` is the mapped window and the status register is a
        // defined register in its first page.
        let status = unsafe { read32(self.regs, REG_GSTS) };
        status & GSTS_TES != 0
    }

    /// Issue a write-buffer flush.
    ///
    /// Only units that say they require it need it, and [`Unit::open`] refuses
    /// those — so this is written, unexercised, and named in the refusal that
    /// makes it unexercised. That is deliberate: a reversal that says *accept
    /// the unit* is then one line rather than a new function written under
    /// pressure on the machine that first needed it.
    ///
    /// # Errors
    ///
    /// [`Refuse::Stuck`].
    pub fn flush_write_buffer(&mut self) -> Result<(), Refuse> {
        // SAFETY: `self.regs` is the mapped window.
        unsafe { write32(self.regs, REG_GCMD, self.gcmd | GCMD_WBF) };
        self.spin_until(REG_GSTS, GSTS_WBFS, false)
    }

    /// Set or clear one persistent command bit and wait for the unit to agree.
    fn command(&mut self, bit: u32, status: u32, set: bool) -> Result<(), Refuse> {
        // One-shot bits — the root-table pointer set among them — are not kept
        // in the shadow, so issuing one does not leave it to be re-issued by the
        // next command.
        let one_shot = bit == GCMD_SRTP || bit == GCMD_WBF;
        let value = if set { self.gcmd | bit } else { self.gcmd & !bit };
        // SAFETY: `self.regs` is the mapped window and the command register is
        // a defined register in its first page.
        unsafe { write32(self.regs, REG_GCMD, value) };
        if !one_shot {
            self.gcmd = value;
        }
        self.spin_until(REG_GSTS, status, set)
    }

    /// Read a register until one bit reaches a value, or give up.
    fn spin_until(&self, offset: u64, bit: u32, want: bool) -> Result<(), Refuse> {
        let mut left = SPIN_LIMIT;
        while left > 0 {
            // SAFETY: `self.regs` is the mapped window and `offset` is a
            // defined register in its first page.
            let value = unsafe { read32(self.regs, offset) };
            if (value & bit != 0) == want {
                return Ok(());
            }
            left = left.saturating_sub(1);
            core::hint::spin_loop();
        }
        Err(Refuse::Stuck)
    }

    /// Throw away everything the unit has cached about translation.
    ///
    /// Global, on both caches, after every change. See the module comment for
    /// why this build does not do better and what number would make it.
    fn invalidate(&mut self) -> Result<(), Refuse> {
        // Every flush this change issued, made visible before the unit is told
        // to go and re-read what was flushed. See [`Coherency::settle`]; on a
        // unit that snoops this is not even a fence.
        self.coherency.settle();
        // The context cache first. A translation-buffer entry is reached
        // through a context entry, so invalidating the buffer before the
        // context would leave the unit able to repopulate it from the stale
        // context on the way past.
        // SAFETY: `self.regs` is the mapped window and the context command
        // register is a defined 64-bit register in its first page.
        unsafe { write64(self.regs, REG_CCMD, CCMD_ICC | CCMD_CIRG_GLOBAL) };
        self.spin_until64(REG_CCMD, CCMD_ICC, false)?;

        let at = self.iotlb_offset.saturating_add(8);
        // SAFETY: as above; the offset was computed from the extended
        // capability register and checked against the mapped window at
        // bring-up.
        unsafe { write64(self.regs, at, IOTLB_IVT | IOTLB_IIRG_GLOBAL) };
        self.spin_until64(at, IOTLB_IVT, false)
    }

    /// [`Unit::spin_until`], on a 64-bit register.
    fn spin_until64(&self, offset: u64, bit: u64, want: bool) -> Result<(), Refuse> {
        let mut left = SPIN_LIMIT;
        while left > 0 {
            // SAFETY: `self.regs` is the mapped window and `offset` is a
            // defined 64-bit register within it.
            let value = unsafe { read64(self.regs, offset) };
            if (value & bit != 0) == want {
                return Ok(());
            }
            left = left.saturating_sub(1);
            core::hint::spin_loop();
        }
        Err(Refuse::Stuck)
    }

    /// Read every fault the unit has recorded, and clear them.
    ///
    /// This is the mechanism the exit criterion rests on. A device that
    /// addresses memory outside its domain does not corrupt that memory and
    /// does not silently fail: the unit refuses the transaction and writes down
    /// who asked, for what address, and why. Reading it is what turns *the
    /// transfer did not happen* into *the transfer was refused*, and those are
    /// different claims — the first is consistent with a device that was never
    /// started.
    #[must_use]
    pub fn faults(&self) -> Faults {
        // SAFETY: `self.regs` is the mapped window and the fault status
        // register is a defined register in its first page.
        let status = unsafe { read32(self.regs, REG_FSTS) };
        let mut found = Faults { first: None, records: 0, overflowed: status & FSTS_PFO != 0 };
        if status & (FSTS_PPF | FSTS_PFO) == 0 {
            return found;
        }

        for index in 0..self.fault_records {
            let at = self.fault_offset.saturating_add(u64::from(index).saturating_mul(16));
            // SAFETY: the offset was computed from the capability register at
            // bring-up and the window was mapped to cover it.
            let high = unsafe { read64(self.regs, at.saturating_add(8)) };
            // Bit 63 of the upper half is the record's own valid bit.
            if high & (1 << 63) == 0 {
                continue;
            }
            // SAFETY: as above.
            let low = unsafe { read64(self.regs, at) };
            found.records = found.records.saturating_add(1);
            if found.first.is_none() {
                found.first = Some(Fault {
                    source: (high & 0xFFFF) as u16,
                    address: low & !0xFFFu64,
                    reason: ((high >> 32) & 0xFF) as u8,
                    // Bit 62 of the upper half: set for a read.
                    read: high & (1 << 62) != 0,
                });
            }
            // Write-one-to-clear, on the valid bit alone. Every other bit is
            // written as zero, which for this register means *leave it*.
            // SAFETY: as above.
            unsafe { write64(self.regs, at.saturating_add(8), 1 << 63) };
        }

        // The status bits are write-one-to-clear too, and they are cleared last:
        // a status cleared before the records it summarises would be a window in
        // which a reader sees records with nothing saying there are any.
        // SAFETY: as above.
        unsafe { write32(self.regs, REG_FSTS, FSTS_PPF | FSTS_PFO) };
        found
    }

    /// Throw away any fault the unit had recorded before this kernel arrived.
    fn clear_faults(&self) {
        let _ = self.faults();
    }
}

/// How many pages of the register window this build will map.
///
/// Four. Every unit this was written against puts its fault records inside the
/// first, and a unit that puts them past the fourth is a unit whose register
/// layout this build should be read again before being trusted with — which is
/// what [`Refuse::FaultsOutOfWindow`] says.
const MAX_REGISTER_PAGES: u64 = 4;

/// Read one 32-bit register.
///
/// # Safety
///
/// `regs` must be a mapped remapping-unit register window and `offset` a
/// defined register within what was mapped. These registers must be accessed at
/// their own width: a narrower or wider access is undefined rather than merely
/// wrong, which is the same rule the local APIC's registers carry.
unsafe fn read32(regs: u64, offset: u64) -> u32 {
    let at = regs.wrapping_add(offset) as *const u32;
    // SAFETY: the caller's guarantee. Volatile because this is a device: the
    // value changes without the compiler being told.
    unsafe { at.read_volatile() }
}

/// Write one 32-bit register.
///
/// # Safety
///
/// As [`read32`], and the value must be one the register accepts.
unsafe fn write32(regs: u64, offset: u64, value: u32) {
    let at = regs.wrapping_add(offset) as *mut u32;
    // SAFETY: the caller's guarantee.
    unsafe { at.write_volatile(value) };
}

/// Read one 64-bit register.
///
/// # Safety
///
/// As [`read32`], and `offset` must name a register that is sixty-four bits
/// wide and eight-byte aligned.
unsafe fn read64(regs: u64, offset: u64) -> u64 {
    let at = regs.wrapping_add(offset) as *const u64;
    // SAFETY: the caller's guarantee.
    unsafe { at.read_volatile() }
}

/// Write one 64-bit register.
///
/// # Safety
///
/// As [`read64`], and the value must be one the register accepts.
unsafe fn write64(regs: u64, offset: u64, value: u64) {
    let at = regs.wrapping_add(offset) as *mut u64;
    // SAFETY: the caller's guarantee.
    unsafe { at.write_volatile(value) };
}
