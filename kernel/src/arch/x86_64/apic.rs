// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The local APIC: the first device the kernel maps, and the last piece of the
//! machine still being run by whoever ran it before.
//!
//! # What this takes over
//!
//! Every core has one of these and it is already switched on when the kernel
//! arrives. Until now that has not mattered, because nothing has enabled
//! interrupts and so nothing it might have delivered could arrive. From here it
//! matters a great deal: the local APIC is what a timer, an inter-processor
//! interrupt and every device interrupt will eventually come through, and all
//! three are the difference between a kernel and a program.
//!
//! # Two things have to be true before an interrupt is survivable
//!
//! **The legacy controllers must not be able to speak.** They are remapped and
//! masked in [`super::pic`], for reasons set out there — the short version is
//! that their default vectors sit on top of the processor's exceptions, so a
//! timer tick from a chip nobody is using would be reported as a double fault.
//!
//! **The APIC must have somewhere to put an interrupt it cannot explain.** The
//! spurious vector is not optional and is not an error path: the architecture
//! requires a vector to be nominated, the processor delivers to it when an
//! interrupt is withdrawn mid-delivery, and a machine with no gate there
//! answers that with a fault instead.
//!
//! # xAPIC, not x2APIC
//!
//! The register window is memory-mapped at the physical address
//! `IA32_APIC_BASE` names, and reached through the device window
//! [`paging::DEVICE_OFFSET`] opens. The other mode — x2APIC, where the same
//! registers are model-specific registers instead — is detected and refused.
//!
//! It buys two things: addressing beyond 255 cores, and register access without
//! a memory mapping. This kernel shards for eight cores, so the first is not a
//! problem it has; and the second is a mapping that exists either way, because
//! the next device along will not be an APIC. Supporting both modes would mean
//! two register paths where only one is ever exercised on a given machine,
//! which is how the untested one comes to be wrong.
//!
//! *Reversal:* a machine with more cores than an eight-bit APIC id can name, or
//! firmware that hands the kernel a processor already in x2APIC mode. The
//! second is why this refuses rather than assuming: a kernel that wrote xAPIC
//! offsets into a window that is not there would read zeroes and believe them.

use super::paging::{self, AddressSpace, BuildError, Features};
use super::{cpuid, read_msr, write_msr};
use crate::mem::FrameAllocator;
use crate::percpu::PerCpu;

/// The register holding the local APIC's base address and its two enable bits.
const IA32_APIC_BASE: u32 = 0x1B;

/// The processor is in x2APIC mode and the memory window does not exist.
const APIC_BASE_EXTD: u64 = 1 << 10;

/// The local APIC is on at all. Clearing it is a one-way trip on most parts.
const APIC_BASE_ENABLE: u64 = 1 << 11;

/// The bits of `IA32_APIC_BASE` that are the physical address.
const APIC_BASE_ADDRESS: u64 = 0x000F_FFFF_FFFF_F000;

/// Local APIC id. Read at bring-up only, to check the kernel agrees with the
/// processor about which core this is.
const REG_ID: u32 = 0x20;

/// Version, and the number of local-vector-table entries this APIC has.
const REG_VERSION: u32 = 0x30;

/// Task priority. Zero means accept everything.
const REG_TASK_PRIORITY: u32 = 0x80;

/// Spurious interrupt vector, and the software enable bit.
const REG_SPURIOUS: u32 = 0xF0;

/// The local vector table entry for the APIC's own timer.
const REG_LVT_TIMER: u32 = 0x320;

/// The APIC is enabled in software. Distinct from the hardware enable in
/// `IA32_APIC_BASE`, and both are required.
const SPURIOUS_ENABLE: u32 = 1 << 8;

/// An entry in the local vector table is masked and will not be delivered.
pub const LVT_MASKED: u32 = 1 << 16;

/// Where a spurious interrupt goes.
///
/// The top of the vector space, which is convention rather than requirement:
/// on parts predating the Pentium 4 the low four bits of this field were
/// hard-wired to one, so 0xFF is the one value that means the same thing on
/// every processor that has ever had a local APIC.
pub const SPURIOUS_VECTOR: u8 = 0xFF;

/// This core's local APIC.
///
/// Per core because every core has its own, at the same physical address seen
/// through its own hardware — which is the one case where a single mapping and
/// a per-core register set are the same thing, and where reading another core's
/// slot would give an address that is right for a set of registers that is
/// wrong.
#[derive(Clone, Copy)]
pub struct Apic {
    /// Where the register window is, or zero before bring-up. Zero is a usable
    /// sentinel here precisely because the device window never starts at zero.
    regs: u64,
}

/// Where each core's local APIC was found.
static APIC: PerCpu<Apic> = PerCpu::new(Apic { regs: 0 });

/// Why the local APIC could not be brought up.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum InitError {
    /// The processor does not report a local APIC. Every 64-bit x86 processor
    /// has one, so in practice this means firmware has disabled it — which is
    /// a machine this kernel cannot schedule on and should say so about.
    Absent,
    /// The processor is already in x2APIC mode. See the module comment.
    ExtendedMode,
    /// The register window could not be mapped.
    Mapping(BuildError),
    /// The APIC id the register reports is not the one `cpuid` reported at
    /// boot. That would mean the per-CPU shard is indexed by one number and the
    /// hardware is answering to another, which is worth catching here rather
    /// than as a mystery on the day a second core starts.
    IdentityMismatch,
    /// The spurious register did not read back what was written to it through
    /// this core's stored window. Either the mapping is not the device, or the
    /// shard is not holding the address that was mapped.
    NotResponding,
}

impl InitError {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Absent => "this processor reports no local APIC",
            Self::ExtendedMode => "the local APIC is in x2APIC mode, which this kernel refuses",
            Self::Mapping(e) => e.message(),
            Self::IdentityMismatch => "the local APIC id disagrees with the one cpuid reported",
            Self::NotResponding => "the local APIC did not read back what was written to it",
        }
    }
}

/// What bring-up found, for the boot log.
///
/// Every field here is a property of the machine rather than a measurement of
/// it, so printing all of them leaves the boot log byte-identical between runs
/// of the same commit — which it has to be, because that log is a fixture.
#[derive(Clone, Copy)]
pub struct Found {
    /// Physical address of the register window.
    pub phys: u64,
    /// The APIC's version byte. 0x14 and up is an integrated APIC.
    pub version: u8,
    /// How many local-vector-table entries it has, counted the way the register
    /// reports them: one less than the number present.
    pub max_lvt: u8,
}

/// Take ownership of this core's local APIC.
///
/// Maps its registers, silences the legacy controllers, nominates a spurious
/// vector and leaves every local-vector-table entry masked. Nothing is delivered
/// as a result of this call; it makes delivery possible and no more.
///
/// # Errors
///
/// [`InitError`]. Fatal at M2: a kernel that cannot reach its interrupt
/// controller has no path to a timer, and every task in the epoch after this
/// one waits on the timer.
///
/// # Safety
///
/// Call once per core, on that core, with interrupts disabled, after
/// [`super::idt::init`] on the same core and after the kernel's own address
/// space is active with `frames` rebound onto its direct map. `space` must be
/// the address space currently in `CR3`.
pub unsafe fn init(
    frames: &mut FrameAllocator,
    space: &AddressSpace,
    features: Features,
) -> Result<Found, InitError> {
    // SAFETY: `cpuid` is unprivileged and has no memory effect.
    let (_, _, edx) = unsafe { cpuid(1) };
    if edx & (1 << 9) == 0 {
        return Err(InitError::Absent);
    }

    // SAFETY: `IA32_APIC_BASE` exists on every processor reporting a local
    // APIC, which the check above has just established.
    let base = unsafe { read_msr(IA32_APIC_BASE) };
    if base & APIC_BASE_EXTD != 0 {
        return Err(InitError::ExtendedMode);
    }

    // The hardware enable, which firmware may have left clear. Read, modify,
    // write: every other bit in this register — the base address itself, and
    // the flag saying whether this is the boot processor — belongs to somebody
    // else.
    if base & APIC_BASE_ENABLE == 0 {
        // SAFETY: setting the enable bit of a register that exists, preserving
        // the rest of it. Setting it is architecturally defined; it is only
        // *clearing* it that cannot be undone without a reset.
        unsafe { write_msr(IA32_APIC_BASE, base | APIC_BASE_ENABLE) };
    }

    let phys = base & APIC_BASE_ADDRESS;
    // SAFETY: the caller has guaranteed `space` is active and `frames` rebound.
    // `phys` came from `IA32_APIC_BASE`, so it names device registers rather
    // than memory — which is the other half of what `map_device` asks for.
    let regs =
        unsafe { paging::map_device(frames, space, phys, features) }.map_err(InitError::Mapping)?;

    // The legacy controllers, before anything can enable interrupts. Their
    // default vectors are the processor's exception vectors, so this has to
    // happen before delivery is possible rather than merely before delivery is
    // wanted.
    // SAFETY: boot processor, interrupts disabled, and nothing else in this
    // kernel touches the 8259 pair.
    unsafe { super::pic::remap_and_mask() };

    // SAFETY: `regs` is the window just mapped, and `REG_ID` is a defined
    // register within the first page of it.
    let id = unsafe { read_reg(regs, REG_ID) } >> 24;
    if id as usize != super::current_cpu() {
        return Err(InitError::IdentityMismatch);
    }

    // SAFETY: as above.
    let version = unsafe { read_reg(regs, REG_VERSION) };

    // Accept interrupts of every priority. The task priority register is the
    // one piece of state that silently drops interrupts when it is wrong, and
    // firmware is under no obligation to have left it at zero.
    // SAFETY: as above.
    unsafe { write_reg(regs, REG_TASK_PRIORITY, 0) };

    // The timer, masked. It is masked here rather than later because the
    // firmware's value for this register is unknown, and an unmasked entry
    // pointing at a vector this kernel has not installed is a fault waiting for
    // the first `sti`.
    // SAFETY: as above.
    unsafe { write_reg(regs, REG_LVT_TIMER, LVT_MASKED) };

    // Software enable, and the vector for an interrupt that arrives without a
    // cause. Last, because it is the step that makes the APIC start delivering.
    // SAFETY: as above.
    unsafe { write_reg(regs, REG_SPURIOUS, SPURIOUS_ENABLE | u32::from(SPURIOUS_VECTOR)) };

    let slot = APIC.mine();
    // SAFETY: this core's own slot, on the boot path, with interrupts disabled
    // — so no handler can be holding it — and nothing else in the kernel names
    // this shard.
    unsafe { slot.write(Apic { regs }) };

    // Read the last register written back, through the shard rather than
    // through the local. Two things are being checked and neither is
    // hypothetical: that the window is a device and not a page of memory that
    // merely mapped successfully, and that the address the shard now holds is
    // the address that was mapped. A wrong window that happens to be readable
    // returns something; it does not return this.
    // SAFETY: this core's slot, written immediately above, with no handler able
    // to have touched it in between because interrupts are still disabled.
    let stored = unsafe { (*slot).regs };
    // SAFETY: `stored` is the window mapped above — that is what is being
    // confirmed — and `REG_SPURIOUS` is a defined register within it.
    let echo = unsafe { read_reg(stored, REG_SPURIOUS) };
    if echo != SPURIOUS_ENABLE | u32::from(SPURIOUS_VECTOR) {
        return Err(InitError::NotResponding);
    }

    Ok(Found { phys, version: version as u8, max_lvt: (version >> 16) as u8 })
}

/// Read one 32-bit register.
///
/// # Safety
///
/// `regs` must be a mapped local APIC register window and `offset` a defined
/// register within it. Every APIC register is four bytes and must be accessed
/// as four bytes: a narrower or wider access is undefined, not merely wrong.
unsafe fn read_reg(regs: u64, offset: u32) -> u32 {
    let at = (regs + u64::from(offset)) as *const u32;
    // SAFETY: the caller's guarantee. Volatile because this is a device: the
    // value changes without the compiler being told, and reading it twice is
    // not the same as reading it once.
    unsafe { at.read_volatile() }
}

/// Write one 32-bit register.
///
/// # Safety
///
/// As [`read_reg`], and the value must be one the register accepts — several
/// of them fault or wedge the APIC on a reserved bit.
unsafe fn write_reg(regs: u64, offset: u32, value: u32) {
    let at = (regs + u64::from(offset)) as *mut u32;
    // SAFETY: the caller's guarantee.
    unsafe { at.write_volatile(value) };
}
