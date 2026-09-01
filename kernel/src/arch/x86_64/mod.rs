// SPDX-License-Identifier: Apache-2.0 OR MIT
//! x86-64 support.
//!
//! The boot handoff, serial output, a QEMU exit channel, the single legitimate
//! hardware time source, the wall clock that may order nothing, and the core's
//! answer to which core it is. Real descriptor tables and paging owned by the
//! frame arrived at M1; the local APIC and the clock measured against the 8254
//! arrive here at M2 — see `docs/design/ring-scene-boot.html` section 15.
//!
//! M3 adds the other privilege level: `ring3` is the transition and the system
//! call entry, and `probe` is the program on the far side of it. What a process
//! *is* rather than how it is entered lives outside this module, in
//! `crate::process`, because none of it is particular to this architecture.

pub mod ap;
pub mod apic;
pub mod boot;
pub mod gdt;
pub mod idt;
pub mod multiboot;
pub mod paging;
pub mod pic;
pub mod pit;
pub mod port;
pub mod probe;
pub mod ring3;
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
/// # The reversal this comment predicted, and why it did not happen
///
/// It used to say that E0-B10 would move the index into `GS`, where reading it
/// is one `mov`. E0-B10 has arrived — there is a second core — and the index is
/// still read from `cpuid`, so the prediction is corrected rather than left
/// standing.
///
/// `GS` is already spoken for. `IA32_KERNEL_GS_BASE` names the ring-3 entry
/// block and `GS_BASE` is deliberately zero while a process runs, and the swap
/// between them happens on the system-call path and *only* there: the interrupt
/// stubs do not `swapgs`. So a core index in `GS` would be correct in a system
/// call and would read a process's base in the timer handler — which is the one
/// caller on the critical path and the one that must not be wrong. Making it
/// right means `swapgs` in every stub, conditional on the saved code selector,
/// which is a change to the interrupt entry path rather than to this function.
///
/// `cpuid` is correct on both paths today, and correct is the requirement.
///
/// *Reversal:* the interrupt stubs learning to swap `GS` — which is E1's, along
/// with the scheduler that makes this function hot enough for the difference to
/// be measurable — or a machine whose APIC ids are sparse, which is the same
/// change for a different reason.
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
    let ebx: u64;
    let ecx: u32;
    let edx: u32;
    // SAFETY: `rbx` cannot be named as an operand, so it is saved and restored
    // around the instruction. The exchange, not a `mov` before a `pop`, is
    // what makes that correct: the allocator may hand the output operand `rbx`
    // itself — under optimisation it prefers to, because that deletes the copy
    // — and then a restore after the capture would overwrite the result with
    // whatever the caller had in `rbx`. The `xchg` is right under both
    // allocations: a scratch register swaps result for saved value, and `rbx`
    // itself degenerates to three no-ops with the result already where the
    // output lives. Sixty-four-bit moves, because a thirty-two-bit save would
    // zero the caller's upper half. This was E0's one miscompiled-boot bug:
    // the release image's application processors computed their shard index
    // from the restored `rbx` — zero, fresh out of the trampoline — and
    // reported ready in the boot processor's slot.
    unsafe {
        core::arch::asm!(
            "mov {ebx:r}, rbx",
            "cpuid",
            "xchg {ebx:r}, rbx",
            ebx = out(reg) ebx,
            inout("eax") leaf => _,
            out("ecx") ecx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    (ebx as u32, ecx, edx)
}

/// One `cpuid` leaf and subleaf, as `(eax, ebx, ecx, edx)`.
///
/// The wider form, for the two leaves whose answer depends on `ecx` going in
/// and on `eax` coming out. [`cpuid`] stays as it is because every other caller
/// in this kernel wants three registers of one leaf, and a four-tuple with a
/// discarded element at every site would be noise.
///
/// # Safety
///
/// As [`cpuid`], and `leaf` must be one this processor implements: a leaf above
/// the maximum reported by leaf zero answers with the maximum leaf's contents
/// rather than with zeroes, so a caller that has not checked is reading a
/// different question's answer.
pub(crate) unsafe fn cpuid_subleaf(leaf: u32, subleaf: u32) -> (u32, u32, u32, u32) {
    let eax: u32;
    let ebx: u64;
    let ecx: u32;
    let edx: u32;
    // SAFETY: as [`cpuid`], exchange and all — the save/capture/restore
    // sequence this replaced was wrong in the same way there as here.
    unsafe {
        core::arch::asm!(
            "mov {ebx:r}, rbx",
            "cpuid",
            "xchg {ebx:r}, rbx",
            ebx = out(reg) ebx,
            inout("eax") leaf => eax,
            inout("ecx") subleaf => ecx,
            out("edx") edx,
            options(nostack, preserves_flags),
        );
    }
    (eax, ebx as u32, ecx, edx)
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
    /// The frame panicked.
    ///
    /// Separate from [`Failure`](Exit::Failure) because the two are different
    /// events, and a harness that cannot tell them apart cannot say which one
    /// it got. `Failure` is the kernel deciding an assertion did not hold and
    /// stopping deliberately — a report, working as designed. A panic is the
    /// frame reaching a state it has no opinion about, which is a bug in the
    /// frame, and it can happen anywhere including inside the code that would
    /// have reported a `Failure`.
    ///
    /// The distinction has to be in the exit code rather than in the log,
    /// because the log is the thing a panic is most likely to have interrupted.
    Panic = 0x12,
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
