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
use super::{cpuid, pit, read_msr, read_tsc, write_msr};
use crate::jitter::Histogram;
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

/// End of interrupt. Written to acknowledge one; the value is ignored.
const REG_EOI: u32 = 0xB0;

/// The local vector table entry for the APIC's own timer.
const REG_LVT_TIMER: u32 = 0x320;

/// Loading this starts the APIC timer counting down. Loading zero stops it.
const REG_TIMER_INITIAL: u32 = 0x380;

/// What the APIC timer has left to count. Read-only, and the whole reason the
/// APIC's own clock can be calibrated at all.
const REG_TIMER_CURRENT: u32 = 0x390;

/// How far the APIC timer divides its input clock.
const REG_TIMER_DIVIDE: u32 = 0x3E0;

/// Divide by one. The bit pattern is not the number: the field's three bits are
/// split around a reserved one, which is why this is written out rather than
/// computed.
const DIVIDE_BY_1: u32 = 0b1011;

/// The deadline the processor compares the timestamp counter against.
///
/// Writing it arms the timer; writing zero disarms it. Reading it back gives
/// zero once the interrupt has been delivered, which is how the hardware says
/// "that one has already happened".
const IA32_TSC_DEADLINE: u32 = 0x6E0;

/// The APIC is enabled in software. Distinct from the hardware enable in
/// `IA32_APIC_BASE`, and both are required.
const SPURIOUS_ENABLE: u32 = 1 << 8;

/// The timer's local vector table entry counts down once and stops.
const LVT_TIMER_ONE_SHOT: u32 = 0b00 << 17;

/// The timer's local vector table entry fires when the timestamp counter
/// reaches [`IA32_TSC_DEADLINE`].
const LVT_TIMER_DEADLINE: u32 = 0b10 << 17;

/// An entry in the local vector table is masked and will not be delivered.
pub const LVT_MASKED: u32 = 1 << 16;

/// Where the timer's interrupt goes.
///
/// The first vector above the thirty-two the processor reserves for its own
/// exceptions, which is the conventional place and — since `pic.rs` has moved
/// the legacy controllers to 0x30 — a vector nothing else on the machine can
/// deliver to.
pub const TIMER_VECTOR: u8 = 32;

/// Where a request to forget a page goes.
///
/// The vector immediately after the timer's, and the second one in this kernel
/// that exists because something is wanted rather than because something might
/// go wrong. It is delivered core to core: see [`crate::smp::shootdown`].
pub const SHOOTDOWN_VECTOR: u8 = 33;

/// Where a doorbell goes.
///
/// The third vector in this kernel that exists because something is wanted, and
/// the one that carries the least: the shootdown next door says *which page*
/// and needs two shared words to say it, and a doorbell says only *stop
/// halting*. The entry is already in the ring and the cursor that publishes it
/// is already visible, so the signal has no content — which is why
/// `crate::doorbell` needs no shared state and adds no fifth address two cores
/// reach.
pub const DOORBELL_VECTOR: u8 = 34;

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
    /// The timestamp counter's rate, measured against the 8254. Zero until
    /// [`calibrate`] has run.
    tsc_khz: u64,
    /// The APIC timer's own rate, measured in the same pass.
    apic_khz: u64,
    /// Which mechanism arms the timer on this processor.
    backend: Backend,
}

/// Each core's local APIC, and the two clock rates measured through it.
///
/// Written at bring-up and calibration, read everywhere after — including from
/// the tick handler, which never writes it. That split is what makes it safe to
/// copy the whole thing out by value on the interrupt path: nothing can be
/// changing it while an interrupt is being handled, because the only code that
/// changes it runs with interrupts disabled and before any are armed.
static APIC: PerCpu<Apic> =
    PerCpu::new(Apic { regs: 0, tsc_khz: 0, apic_khz: 0, backend: Backend::OneShot });

/// How the timer is armed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Backend {
    /// `IA32_TSC_DEADLINE`: the deadline is handed to the processor as an
    /// absolute timestamp-counter value. The mechanism the milestone names, and
    /// the one worth having — there is no conversion, no divisor and no
    /// separate clock, so the only error left is the one in the schedule.
    Deadline,
    /// The APIC timer counting down once from a loaded value. Always present;
    /// used where TSC-deadline is not.
    ///
    /// It is a fallback and it is not a lesser design: the schedule is still
    /// absolute and still in counter ticks. What differs is that the remaining
    /// interval has to be converted into APIC ticks with a measured ratio, so
    /// an error in the calibration becomes an error in the interval — which is
    /// exactly the error TSC-deadline does not have.
    OneShot,
}

impl Backend {
    /// A word for the boot log.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Deadline => "tsc-deadline",
            Self::OneShot => "apic one-shot",
        }
    }
}

/// The timer's schedule and what it has recorded, for this core.
///
/// Deliberately separate from [`APIC`], and deliberately without the tick
/// count in it. The handler takes a `&mut` to this and nothing else does while
/// a run is in progress, which is what makes that reference exclusive; the one
/// value the waiting loop has to see — [`TICKS`] — is kept outside it so that
/// no reference is ever live across a read of it.
#[derive(Clone, Copy)]
pub struct Timer {
    /// Counter ticks between deadlines.
    period: u64,
    /// The absolute deadline the next interrupt is for.
    deadline: u64,
    /// How many ticks the run asked for.
    target: u64,
    /// Ticks that arrived a whole period or more late — a deadline that was
    /// not merely missed by a margin but skipped.
    missed: u64,
    /// How late each tick was, in counter ticks.
    late: Histogram,
}

/// The schedule, per core.
static TIMER: PerCpu<Timer> =
    PerCpu::new(Timer { period: 0, deadline: 0, target: 0, missed: 0, late: Histogram::new() });

/// Ticks delivered, per core.
///
/// The one piece of timer state that two things touch: the handler writes it
/// and the waiting loop reads it. It lives on its own, outside [`Timer`], and
/// every access to it on both sides is volatile through the raw pointer — never
/// through a reference, because a reference here would be a claim that the
/// handler and the code it interrupted are not both looking at it, and they
/// are. `percpu.rs` says this is the case no per-CPU abstraction can see; this
/// is what discharging that looks like.
static TICKS: PerCpu<u64> = PerCpu::new(0);

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
    unsafe { slot.write(Apic { regs, tsc_khz: 0, apic_khz: 0, backend: Backend::OneShot }) };

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

/// Where this core's local APIC registers are, or zero before bring-up.
///
/// Handed out for two callers that both need the address and neither of which
/// should be reading another core's shard to get it: the inter-processor
/// interrupts in [`super::ap`], and the boot processor telling a core it is
/// about to start where the window is.
#[must_use]
pub fn window() -> u64 {
    // SAFETY: this core's own slot, read by value. The handler never writes it
    // and the code that does runs with interrupts disabled.
    unsafe { APIC.mine().read() }.regs
}

/// What this core's timestamp counter was measured to run at, in kilohertz.
///
/// Zero before [`calibrate`] on the boot processor or [`adopt`] on any other,
/// which is the only value a caller has to treat specially: a bound computed
/// from zero is a bound of zero, and every caller here saturates rather than
/// divides.
#[must_use]
pub fn tsc_khz() -> u64 {
    // SAFETY: as [`window`].
    unsafe { APIC.mine().read() }.tsc_khz
}

/// Acknowledge the interrupt this core is handling.
///
/// Until this is written the local APIC will not deliver another interrupt at
/// this priority, which is what makes a handler single-threaded with respect to
/// itself. [`on_tick`] does its own; this is for the handlers that live outside
/// this module.
///
/// # Safety
///
/// Call once, from an interrupt handler, on the core the interrupt was
/// delivered to. Writing it anywhere else acknowledges somebody else's
/// interrupt — including, on the spurious vector, an interrupt that the
/// architecture is explicit must not be acknowledged at all.
pub unsafe fn end_of_interrupt() {
    // SAFETY: this core's own slot, read by value; the handler never writes it.
    let apic = unsafe { APIC.mine().read() };
    // SAFETY: this core's window, and the value written to this register is
    // ignored by the hardware.
    unsafe { write_reg(apic.regs, REG_EOI, 0) };
}

/// Take ownership of a started core's local APIC.
///
/// The difference from [`init`] is everything that is not per core. The
/// register window is not mapped again — every core's local APIC answers at the
/// same physical address, through its own hardware, so one mapping serves all
/// of them and a second would be a second name for the same page. The legacy
/// controllers are not remapped again either: there is one pair on the machine
/// and the boot processor has already silenced them.
///
/// What is per core is done here, in the same order [`init`] does it: accept
/// every priority, mask the timer before anything can deliver to a vector this
/// core has not installed, and software-enable last.
///
/// `clocks` are the boot processor's measurement. They are adopted rather than
/// re-measured because measuring them means owning the 8254, which is one chip
/// for the machine — two cores calibrating against it at once would each
/// measure the other's interference. The assumption underneath is that the
/// timestamp counter runs at the same rate on every core, which is what
/// `cpuid`'s invariant-TSC bit promises and what every machine this kernel
/// boots on provides.
///
/// *Reversal:* a machine where the counters are not invariant across cores, at
/// which point calibration becomes per core and needs a clock that is not the
/// 8254 to calibrate against.
///
/// # Errors
///
/// As [`init`], less the mapping.
///
/// # Safety
///
/// Call once, on the core being brought up, with interrupts disabled, after
/// [`super::idt::init`] on the same core, and with `regs` the window the boot
/// processor mapped — which is [`window`] read on that core.
pub unsafe fn adopt(regs: u64, clocks: Clocks) -> Result<(), InitError> {
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
    if base & APIC_BASE_ENABLE == 0 {
        // SAFETY: setting the enable bit of a register that exists, preserving
        // every other bit — including the base address and the flag saying this
        // is not the boot processor.
        unsafe { write_msr(IA32_APIC_BASE, base | APIC_BASE_ENABLE) };
    }
    // The window the boot processor mapped has to be this core's registers too.
    // It is the same physical address on every core, and checking that rather
    // than assuming it is what turns a firmware surprise into a refusal.
    if base & APIC_BASE_ADDRESS != regs_phys(regs) {
        return Err(InitError::IdentityMismatch);
    }

    // SAFETY: `regs` is a mapped window and `REG_ID` is a defined register in
    // its first page.
    let id = unsafe { read_reg(regs, REG_ID) } >> 24;
    if id as usize != super::current_cpu() {
        return Err(InitError::IdentityMismatch);
    }

    // SAFETY: as above.
    unsafe { write_reg(regs, REG_TASK_PRIORITY, 0) };
    // SAFETY: as above. Masked before anything can be delivered to a vector.
    unsafe { write_reg(regs, REG_LVT_TIMER, LVT_MASKED) };
    // SAFETY: as above. Last, because it is the step that starts delivery.
    unsafe { write_reg(regs, REG_SPURIOUS, SPURIOUS_ENABLE | u32::from(SPURIOUS_VECTOR)) };

    let slot = APIC.mine();
    // SAFETY: this core's own slot, on its boot path, with interrupts disabled
    // — so no handler can be holding it — and nothing else names this shard.
    unsafe {
        slot.write(Apic {
            regs,
            tsc_khz: clocks.tsc_khz,
            apic_khz: clocks.apic_khz,
            backend: clocks.backend,
        });
    }

    // SAFETY: this core's slot, written immediately above, with no handler able
    // to have touched it because interrupts are still disabled.
    let stored = unsafe { (*slot).regs };
    // SAFETY: `stored` is the window above and `REG_SPURIOUS` is defined in it.
    let echo = unsafe { read_reg(stored, REG_SPURIOUS) };
    if echo != SPURIOUS_ENABLE | u32::from(SPURIOUS_VECTOR) {
        return Err(InitError::NotResponding);
    }

    Ok(())
}

/// The physical address behind a device-window address.
///
/// The device window is a straight offset — [`paging::map_device`] returns
/// `DEVICE_OFFSET + phys` — so this is the inverse, and it exists so that
/// [`adopt`] can check the window it was handed against what this core's own
/// `IA32_APIC_BASE` says, rather than trusting the caller about hardware.
const fn regs_phys(regs: u64) -> u64 {
    (regs - paging::DEVICE_OFFSET) & !(0xFFF)
}

/// What the two clocks turned out to run at.
#[derive(Clone, Copy)]
pub struct Clocks {
    /// The timestamp counter, in kilohertz.
    pub tsc_khz: u64,
    /// The APIC timer's input, in kilohertz, at a divisor of one.
    pub apic_khz: u64,
    /// Which mechanism will arm the timer.
    pub backend: Backend,
}

/// Why the timer could not be set up or run.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TimerError {
    /// [`init`] has not run on this core.
    NotBroughtUp,
    /// The reference clock did not answer. See [`pit::CalibrateError`].
    Reference(pit::CalibrateError),
    /// A measured frequency is outside any band a working machine produces.
    /// Reported rather than used: every interval in the system would be
    /// computed from it, so a wrong number here is a wrong number everywhere,
    /// and silently continuing would make the timer's own output the evidence
    /// for the frequency that produced it.
    ImplausibleClock,
    /// A run was asked for before [`calibrate`].
    NotCalibrated,
    /// The requested frequency is faster than the measured clocks can express —
    /// a period of less than one tick of the counter the schedule is kept in.
    PeriodTooShort,
}

impl TimerError {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NotBroughtUp => "the local APIC has not been brought up on this core",
            Self::Reference(e) => e.message(),
            Self::ImplausibleClock => "a calibrated frequency is outside any plausible band",
            Self::NotCalibrated => "the clocks have not been calibrated",
            Self::PeriodTooShort => "the requested tick rate is faster than the clock",
        }
    }
}

/// The slowest clock worth believing, in kilohertz. One megahertz.
const PLAUSIBLE_MIN_KHZ: u64 = 1_000;

/// The fastest, in kilohertz. A hundred gigahertz.
///
/// Both bounds are deliberately absurd. They are not a quality check — they
/// catch a measurement that did not happen at all: a counter that did not
/// advance, a gate that rose immediately, a subtraction the wrong way round.
/// A band tight enough to judge a real machine would fail on the next one.
const PLAUSIBLE_MAX_KHZ: u64 = 100_000_000;

/// Measure the timestamp counter and the APIC timer against the 8254.
///
/// One pass over one interval, sampling both clocks at each end, because two
/// passes would measure two different intervals and the ratio between the two
/// clocks is the thing the one-shot path depends on.
///
/// # Errors
///
/// [`TimerError`]. Fatal: everything after this is computed from these numbers.
///
/// # Safety
///
/// Call once per core, on that core, after [`init`] on the same core, with
/// interrupts disabled — an interrupt landing inside the interval would be
/// counted as part of it and would inflate both frequencies together, which is
/// the failure mode a ratio does not reveal.
pub unsafe fn calibrate() -> Result<Clocks, TimerError> {
    let slot = APIC.mine();
    // SAFETY: this core's own slot, interrupts disabled, no handler armed.
    let mut apic = unsafe { slot.read() };
    if apic.regs == 0 {
        return Err(TimerError::NotBroughtUp);
    }

    // The APIC timer, counting down from as far as it can, undivided and
    // masked. Masked because nothing is to be delivered — the count register is
    // being read directly — and undivided because a divisor would throw away
    // resolution in the one measurement everything else is derived from.
    // SAFETY: `apic.regs` is the window `init` mapped and checked.
    unsafe { write_reg(apic.regs, REG_TIMER_DIVIDE, DIVIDE_BY_1) };
    // SAFETY: as above.
    unsafe { write_reg(apic.regs, REG_LVT_TIMER, LVT_MASKED | LVT_TIMER_ONE_SHOT) };
    // SAFETY: as above. This is the write that starts it.
    unsafe { write_reg(apic.regs, REG_TIMER_INITIAL, u32::MAX) };

    // The closure is built outside the `unsafe` block below, so that its own
    // obligation — reading a device register — is discharged where it is taken
    // rather than swallowed by the block around the call.
    let regs = apic.regs;
    let probe = || {
        // SAFETY: `regs` is the window `init` mapped and checked, and the
        // current-count register is read-only with no side effect.
        (read_tsc(), unsafe { read_reg(regs, REG_TIMER_CURRENT) })
    };

    // SAFETY: the 8254 is present on this platform, and interrupts are off per
    // this function's own contract.
    let sampled = unsafe { pit::calibrate_micros(probe) };

    // Stop it, whatever happened. A one-shot timer left running is a timer that
    // reaches zero somewhere unrelated to anything.
    // SAFETY: as above; zero is the defined way to stop it.
    unsafe { write_reg(apic.regs, REG_TIMER_INITIAL, 0) };

    let ((tsc_before, apic_before), (tsc_after, apic_after)) =
        sampled.map_err(TimerError::Reference)?;

    // The timestamp counter counts up and the APIC timer counts down, so the
    // two subtractions go opposite ways. Getting this backwards is the classic
    // calibration bug and it does not announce itself: it produces a number,
    // and the number is wrong by a factor nobody can guess afterwards.
    let tsc_delta = tsc_after.saturating_sub(tsc_before);
    let apic_delta = u64::from(apic_before.saturating_sub(apic_after));

    let micros = u64::from(pit::CALIBRATE_MICROS);
    let tsc_khz = tsc_delta.saturating_mul(1_000) / micros;
    let apic_khz = apic_delta.saturating_mul(1_000) / micros;

    if !(PLAUSIBLE_MIN_KHZ..=PLAUSIBLE_MAX_KHZ).contains(&tsc_khz)
        || !(PLAUSIBLE_MIN_KHZ..=PLAUSIBLE_MAX_KHZ).contains(&apic_khz)
    {
        return Err(TimerError::ImplausibleClock);
    }

    // SAFETY: `cpuid` is unprivileged and has no memory effect.
    let (_, ecx, _) = unsafe { cpuid(1) };
    let backend = if ecx & (1 << 24) == 0 { Backend::OneShot } else { Backend::Deadline };

    apic.tsc_khz = tsc_khz;
    apic.apic_khz = apic_khz;
    apic.backend = backend;
    // SAFETY: this core's own slot, interrupts still disabled, nothing armed.
    unsafe { slot.write(apic) };

    Ok(Clocks { tsc_khz, apic_khz, backend })
}

/// What a run produced.
#[derive(Clone, Copy)]
pub struct Summary {
    /// Ticks actually delivered. Equal to what was asked for unless the run
    /// gave up, which is the only reason to check it.
    pub ticks: u64,
    /// What was asked for.
    pub target: u64,
    /// The rate the schedule was built at.
    pub hz: u32,
    /// The counter the histogram is denominated in.
    pub tsc_khz: u64,
    /// Which mechanism armed it.
    pub backend: Backend,
    /// Ticks that arrived a whole period or more late.
    pub missed: u64,
    /// How late every tick was.
    pub late: Histogram,
}

/// A timer run that has been armed and not yet stopped.
///
/// The schedule lives in [`TIMER`], per core, and this is the handle the caller
/// keeps: what was asked for, what the numbers are denominated in, and the one
/// value nothing else knows — the point past which waiting for a tick that is
/// not coming stops being patience.
///
/// # Why a run is three calls rather than one
///
/// It was one until M3. The reason it is three is that the interesting thing to
/// do between arming a timer and stopping it stopped being *waiting*: a process
/// runs there now, at ring 3, and the whole claim the milestone has to support
/// is that the schedule survives it. A `run(hz, target)` that owns the interval
/// can only be given a callback, and a callback that enters ring 3 is not a
/// callback — it is the rest of the kernel.
#[derive(Clone, Copy)]
pub struct Window {
    hz: u32,
    target: u64,
    tsc_khz: u64,
    giveup: u64,
}

impl Window {
    /// The counter value past which a tick that has not arrived is not coming.
    ///
    /// Handed out because the give-up is not only [`wait`]'s: a process that is
    /// waiting on ticks needs the same bound, for the same reason, and two
    /// bounds computed from two guesses would be two ways to hang.
    #[must_use]
    pub const fn giveup(&self) -> u64 {
        self.giveup
    }

    /// How many ticks the run asked for.
    #[must_use]
    pub const fn target(&self) -> u64 {
        self.target
    }
}

/// Arm the timer at `hz` for `target` ticks and enable delivery.
///
/// Returns with interrupts *enabled*, which is the whole difference between
/// this and every other function in this module: from here until [`stop`] the
/// core is taking ticks, and whatever the caller does in between is what the
/// histogram is a distribution of.
///
/// # Errors
///
/// [`TimerError`] if the core is not calibrated or the rate is impossible.
///
/// # Safety
///
/// Call on the core that was brought up and calibrated, with interrupts
/// disabled on entry, after [`super::idt::init`] has installed
/// [`TIMER_VECTOR`]. Pair every call with [`stop`] on the same core.
pub unsafe fn start(hz: u32, target: u64) -> Result<Window, TimerError> {
    // SAFETY: this core's own slot; nothing is armed, so no handler can be
    // holding it.
    let apic = unsafe { APIC.mine().read() };
    if apic.regs == 0 {
        return Err(TimerError::NotBroughtUp);
    }
    if apic.tsc_khz == 0 {
        return Err(TimerError::NotCalibrated);
    }

    let period = apic.tsc_khz.saturating_mul(1_000) / u64::from(hz.max(1));
    if period == 0 {
        return Err(TimerError::PeriodTooShort);
    }

    let now = read_tsc();
    let first = now.saturating_add(period);

    let timer = TIMER.mine();
    // SAFETY: this core's slot, interrupts disabled, nothing armed — so no
    // handler exists that could be holding a reference to it.
    unsafe {
        timer.write(Timer { period, deadline: first, target, missed: 0, late: Histogram::new() });
    }
    let ticks_at = TICKS.mine();
    // SAFETY: this core's counter, before any handler can write it.
    unsafe { ticks_at.write_volatile(0) };

    // SAFETY: the caller has established that the vector is installed and the
    // window is this core's.
    unsafe { arm_first(&apic, first, now) };

    // The whole schedule, plus a quarter of it, plus a second. Generous on
    // purpose: this bound is not a timeout on a slow tick, it is the answer to
    // a timer that never fires at all, and a bound tight enough to be a timeout
    // would turn a slow machine into a failure.
    let budget = period.saturating_mul(target).saturating_mul(5) / 4;
    let giveup = now.saturating_add(budget).saturating_add(apic.tsc_khz.saturating_mul(1_000));

    // SAFETY: every vector the APIC can deliver to now has a gate — the timer's
    // and the spurious one — and the legacy controllers were masked at bring-up.
    unsafe { core::arch::asm!("sti", options(nostack)) };

    Ok(Window { hz, target, tsc_khz: apic.tsc_khz, giveup })
}

/// Ticks delivered on this core since the run was armed.
#[must_use]
pub fn ticks() -> u64 {
    // SAFETY: a volatile read of this core's counter, which the handler writes
    // volatilely. No reference to it is taken on either side.
    unsafe { TICKS.mine().read_volatile() }
}

/// Spin until the run has had every tick it asked for, or until it is clear
/// none is coming.
///
/// Returns how many were delivered.
///
/// # Why this spins rather than halting
///
/// Two reasons, and the second is the one that decided it. Halting between
/// ticks would put the idle-exit path inside every sample, and how deep a core
/// is allowed to idle is computed from the reservation table — RFC 0006 — and
/// there is neither a reservation table nor an implementation of the
/// computation before E5-B07. So a halting measurement would be measuring a
/// policy that is written and not yet in effect.
///
/// And a halt with no interrupt to wake it is a machine that stops with no
/// output. The give-up bound is why this cannot hang; a halt would make the
/// bound unreachable, because there would be no code running to check it.
///
/// This does mean the number is the best case for wake-up latency. Said here
/// rather than discovered later.
///
/// # Safety
///
/// Call on the core [`start`] was called on, while its run is still armed.
pub unsafe fn wait(window: &Window) -> u64 {
    loop {
        let n = ticks();
        if n >= window.target {
            return n;
        }
        if read_tsc() > window.giveup {
            return n;
        }
        core::hint::spin_loop();
    }
}

/// Disarm the timer and report what the run recorded.
///
/// Returns with interrupts disabled, whether the run completed or gave up. A
/// run that ended early is not an error: the [`Summary`] says how many ticks it
/// actually got, because the histogram it collected up to that point is worth
/// more than an error code.
///
/// # Safety
///
/// Call on the core [`start`] was called on, once per [`start`].
pub unsafe fn stop(window: &Window) -> Summary {
    // SAFETY: this core's own slot; read by value, and the handler never writes
    // it.
    let apic = unsafe { APIC.mine().read() };

    // SAFETY: disabling delivery on this core. Everything below runs with no
    // handler able to interleave, which is what makes reading the state back by
    // value sound.
    unsafe { core::arch::asm!("cli", options(nostack)) };

    // Unconditionally, because the handler only disarms on the path where it
    // reached the target — and the other path is exactly the one where a timer
    // is still armed and nobody is waiting for it.
    // SAFETY: this core's window, interrupts now disabled.
    unsafe { disarm(&apic) };

    // SAFETY: interrupts are disabled and the timer is disarmed, so no handler
    // can be running or about to run: this is the one moment the whole struct
    // can be read out by value.
    let state = unsafe { TIMER.mine().read() };
    // SAFETY: as above; nothing can be advancing the counter now.
    let delivered = unsafe { TICKS.mine().read_volatile() };

    Summary {
        ticks: delivered,
        target: window.target,
        hz: window.hz,
        tsc_khz: window.tsc_khz,
        backend: apic.backend,
        missed: state.missed,
        late: state.late,
    }
}

/// Point the timer's local vector table entry at [`TIMER_VECTOR`] and arm it.
///
/// # Safety
///
/// `apic` must describe this core's mapped window, and [`TIMER_VECTOR`] must
/// have a gate installed.
unsafe fn arm_first(apic: &Apic, deadline: u64, now: u64) {
    let mode = match apic.backend {
        Backend::Deadline => LVT_TIMER_DEADLINE,
        Backend::OneShot => LVT_TIMER_ONE_SHOT,
    };
    // Unmasked, so this is the write that makes delivery possible.
    // SAFETY: the caller's guarantee.
    unsafe { write_reg(apic.regs, REG_LVT_TIMER, mode | u32::from(TIMER_VECTOR)) };

    // Between changing the timer's mode and writing the deadline, because the
    // two are separate stores to separate places and the processor is entitled
    // to reorder them. The manual requires the fence here specifically; without
    // it a deadline can be armed against the mode the entry used to have.
    // SAFETY: a fence has no operands and no failure mode.
    unsafe { core::arch::asm!("mfence", options(nostack, preserves_flags)) };

    // SAFETY: as above.
    unsafe { arm(apic, deadline, now) };
}

/// Arm for one absolute deadline.
///
/// The deadline is absolute in both paths, which is the property that matters:
/// re-arming with "now plus a period" would let a late tick push the next one
/// later, and the resulting histogram would look *better* than the truth
/// because every deadline had moved to accommodate the tick that missed it.
///
/// # Safety
///
/// As [`arm_first`], and the timer's local vector table entry must already
/// name the right mode.
unsafe fn arm(apic: &Apic, deadline: u64, now: u64) {
    match apic.backend {
        Backend::Deadline => {
            // A deadline already in the past is delivered immediately, which is
            // the behaviour wanted: a tick that is late is still owed.
            // SAFETY: the register exists — `calibrate` chose this backend only
            // after `cpuid` reported it.
            unsafe { write_msr(IA32_TSC_DEADLINE, deadline) };
        }
        Backend::OneShot => {
            // Counter ticks to APIC ticks. Both rates were measured over the
            // same interval, so this ratio is the one thing calibration is
            // really for.
            let remaining = deadline.saturating_sub(now);
            let count = remaining.saturating_mul(apic.apic_khz) / apic.tsc_khz.max(1);
            // Never zero: loading zero stops the timer rather than firing it at
            // once, so a deadline already past would arm nothing and the run
            // would stall. One is the soonest this mechanism can say "now".
            let count = count.clamp(1, u64::from(u32::MAX)) as u32;
            // SAFETY: the caller's guarantee.
            unsafe { write_reg(apic.regs, REG_TIMER_INITIAL, count) };
        }
    }
}

/// Stop the timer and mask its vector.
///
/// # Safety
///
/// As [`arm_first`].
unsafe fn disarm(apic: &Apic) {
    match apic.backend {
        // SAFETY: zero is the defined disarm for this register.
        Backend::Deadline => unsafe { write_msr(IA32_TSC_DEADLINE, 0) },
        // SAFETY: and for this one.
        Backend::OneShot => unsafe { write_reg(apic.regs, REG_TIMER_INITIAL, 0) },
    }
    // Masked as well as stopped. Two mechanisms, one of which is a countdown
    // that could already be in flight — stopping it is not the same as
    // promising nothing is on its way.
    // SAFETY: the caller's guarantee.
    unsafe { write_reg(apic.regs, REG_LVT_TIMER, LVT_MASKED) };
}

/// Where a timer interrupt arrives.
///
/// # Safety
///
/// Called from the interrupt dispatcher on the core the timer was armed on,
/// with interrupts disabled by the gate. Not to be called from anywhere else:
/// it acknowledges an interrupt that would then not have happened, and it
/// advances a schedule nobody is keeping.
pub(super) unsafe fn on_tick() {
    // First, before anything else this function does — including finding out
    // which core it is on. Everything after this point is measured out of the
    // *next* interval rather than this one, and the schedule is absolute, so a
    // cost here does not accumulate.
    let now = read_tsc();

    // SAFETY: read by value. The handler never writes this shard and the only
    // code that does runs with interrupts disabled and nothing armed, so no
    // write can be in progress.
    let apic = unsafe { APIC.mine().read() };

    let ticks_at = TICKS.mine();
    // SAFETY: volatile, through the raw pointer, because the waiting loop is
    // reading the same location. See [`TICKS`].
    let ticks = unsafe { ticks_at.read_volatile() } + 1;

    let slot = TIMER.mine();
    // SAFETY: this reference is exclusive for as long as it is live. Two
    // claims, and both are needed: no other core can reach this slot, which is
    // what `PerCpu` is; and no other code *on this core* can, because the gate
    // is an interrupt gate so this handler cannot interrupt itself, and the
    // only other reader — the loop in `run` — touches `TICKS` and never this.
    // That separation is the whole reason the tick count lives outside `Timer`.
    let timer = unsafe { &mut *slot };

    let late = now.saturating_sub(timer.deadline);
    timer.late.record(late);
    if late >= timer.period {
        timer.missed += 1;
    }

    if ticks < timer.target {
        timer.deadline = timer.deadline.saturating_add(timer.period);
        // SAFETY: this core's window, and the mode was set by `arm_first`.
        unsafe { arm(&apic, timer.deadline, now) };
    } else {
        // SAFETY: as above.
        unsafe { disarm(&apic) };
    }

    // SAFETY: volatile, for the same reason as the read above. Written after
    // the timer is re-armed so that the loop cannot observe the target being
    // reached before the hardware has been told to stop.
    unsafe { ticks_at.write_volatile(ticks) };

    // Last. Until this write the APIC will not deliver another interrupt at
    // this priority, which is the guarantee that makes everything above
    // single-threaded with respect to itself.
    // SAFETY: this core's window; the value written to this register is ignored
    // by the hardware.
    unsafe { write_reg(apic.regs, REG_EOI, 0) };
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
