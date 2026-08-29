// SPDX-License-Identifier: Apache-2.0 OR MIT
//! x86-64 support.
//!
//! At M0 this is the boot handoff, serial output, a QEMU exit channel, and the
//! single legitimate hardware time source. Real descriptor tables, paging owned
//! by the frame, and the local APIC arrive at M1 and M2 — see
//! `docs/design/ring-scene-boot.html` section 15. The page tables the boot stub
//! builds exist only to make the jump to long mode legal, and M1 replaces them.

pub mod boot;
pub mod gdt;
pub mod idt;
pub mod multiboot;
pub mod paging;
pub mod serial;

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
