// SPDX-License-Identifier: Apache-2.0 OR MIT
//! x86-64 support.
//!
//! The boot handoff, serial output, a QEMU exit channel, the single legitimate
//! hardware time source, the wall clock that may order nothing, and the core's
//! answer to which core it is. Real descriptor tables and paging owned by the
//! frame arrived at M1; the local APIC and the clock measured against the 8254
//! arrive here at M2 — see `docs/design/ring-scene-boot.html` section 15.

pub mod apic;
pub mod boot;
pub mod gdt;
pub mod idt;
pub mod multiboot;
pub mod paging;
pub mod pic;
pub mod pit;
pub mod port;
pub mod rtc;
pub mod serial;

/// Which core is executing this.
///
/// # Why the processor is asked rather than told
///
/// The alternative is a counter handed out as cores are started, which is one
/// more piece of boot state to keep — and which reads correctly on the boot
/// processor whatever the code does, because the boot processor is the one that
/// initialises the counter. The initial APIC id is the machine's own answer,
/// available before the APIC is configured, before a second core exists, and
/// with no state behind it that could be wrong.
///
/// # What is true only for now
///
/// The initial APIC id is used directly as a slot index, which assumes the ids
/// are small and dense. QEMU numbers them from zero and a single-core machine
/// answers zero, so the assumption holds for every configuration this kernel
/// currently boots on. It is not true in general: a multi-socket machine
/// numbers by package and core, and the ids are sparse.
///
/// `cpuid` also serialises, which is a cost worth naming before it is paid in a
/// loop. Nothing calls this per interrupt yet.
///
/// *Reversal:* E0-B10 starts the application processors, and each core learns
/// its own dense index there. From that point the index lives in `GS` and this
/// function reads it with one `mov` — which is both the cheap answer and the
/// one that survives sparse ids. `PerCpu` does not change; this function does.
#[must_use]
pub fn current_cpu() -> usize {
    // SAFETY: `cpuid` is unprivileged and has no memory effect.
    let (ebx, _, _) = unsafe { cpuid(1) };
    (ebx >> 24) as usize
}

/// One `cpuid` leaf, as `(ebx, ecx, edx)`.
///
/// # Safety
///
/// None beyond the instruction itself, which is unprivileged and has no memory
/// effect. `unsafe` because it is `asm!`.
pub(crate) unsafe fn cpuid(leaf: u32) -> (u32, u32, u32) {
    let ebx: u32;
    let ecx: u32;
    let edx: u32;
    // SAFETY: `rbx` is reserved by the compiler, so it is saved and restored
    // around the instruction rather than named as an output. The target
    // disables the red zone, so using the stack here is sound.
    unsafe {
        core::arch::asm!(
            "push rbx",
            "cpuid",
            "mov {ebx:e}, ebx",
            "pop rbx",
            ebx = lateout(reg) ebx,
            inout("eax") leaf => _,
            out("ecx") ecx,
            out("edx") edx,
            options(preserves_flags),
        );
    }
    (ebx, ecx, edx)
}

/// Read a model-specific register.
///
/// # Safety
///
/// `msr` must be a register this processor implements. Reading one it does not
/// raises a general protection fault — which is now reported rather than fatal
/// to the machine, but is still a fault nobody asked for. Callers check
/// `cpuid` first.
pub(crate) unsafe fn read_msr(msr: u32) -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: the caller has promised the register exists. `rdmsr` reads it
    // into edx:eax and touches nothing else.
    unsafe {
        core::arch::asm!(
            "rdmsr",
            in("ecx") msr,
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}

/// Write a model-specific register.
///
/// # Safety
///
/// As [`read_msr`], and considerably more: a model-specific register is where
/// the processor keeps the switches that change what instructions mean. The
/// caller must know what this particular register does and must preserve every
/// bit of it that belongs to somebody else — these registers are read, modified
/// and written, never assigned, unless the whole register is one field.
pub(crate) unsafe fn write_msr(msr: u32, value: u64) {
    // SAFETY: the caller has promised the register exists and that this value
    // is a correct thing to put in it.
    unsafe {
        core::arch::asm!(
            "wrmsr",
            in("ecx") msr,
            in("eax") value as u32,
            in("edx") (value >> 32) as u32,
            options(nostack, preserves_flags),
        );
    }
}

/// QEMU `isa-debug-exit` port. Writing to it terminates the machine with an
/// exit status derived from the value, which is what lets an integration test
/// assert on a kernel run through ordinary `cargo test`.
const QEMU_EXIT_PORT: u16 = 0xF4;

/// How a kernel run ended.
#[derive(Clone, Copy, Debug)]
#[repr(u32)]
pub enum Exit {
    /// Everything asserted held.
    Success = 0x10,
    /// Something failed. The serial log carries the detail.
    Failure = 0x11,
}

/// Terminate the machine.
///
/// QEMU reports `(value << 1) | 1`, so `Success` becomes process exit code 33
/// and `Failure` becomes 35. `xtask` maps those back.
pub fn exit_qemu(status: Exit) -> ! {
    // SAFETY: this port is the QEMU debug-exit device, present because the
    // launch configuration in xtask adds it. On real hardware the write lands
    // on an unused port and the halt loop below runs instead.
    unsafe {
        core::arch::asm!(
            "out dx, eax",
            in("dx") QEMU_EXIT_PORT,
            in("eax") status as u32,
            options(nomem, nostack, preserves_flags),
        );
    }
    halt_forever()
}

/// Park this core with interrupts disabled.
pub fn halt_forever() -> ! {
    loop {
        // SAFETY: `cli` and `hlt` are architecturally valid at any privilege
        // level the kernel runs at, and parking is always a sound response to
        // having nothing left to do.
        unsafe {
            core::arch::asm!("cli; hlt", options(nomem, nostack));
        }
    }
}

/// Read the timestamp counter.
///
/// # This is the only `rdtsc` in the tree
///
/// Every other consumer of time goes through [`f_env::Env`]. That is what makes
/// a run reproducible from a seed, and `cargo xtask lint-determinism` fails the
/// build if a second call site appears. See `docs/rfc/0004-determinism-substrate.md`.
#[must_use]
pub fn read_tsc() -> u64 {
    let low: u32;
    let high: u32;
    // SAFETY: `rdtsc` is unprivileged, has no memory effects, and writes only
    // the two output registers named here.
    unsafe {
        core::arch::asm!(
            "rdtsc",
            out("eax") low,
            out("edx") high,
            options(nomem, nostack, preserves_flags),
        );
    }
    (u64::from(high) << 32) | u64::from(low)
}
