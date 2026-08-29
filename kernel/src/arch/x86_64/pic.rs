// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The 8259 pair, moved out of the way and switched off.
//!
//! # Why a driver for a chip this kernel will never use
//!
//! Because it is already running. The two programmable interrupt controllers
//! are wired up by the firmware before the kernel sees the machine, the 8254's
//! channel 0 is free-running on IRQ 0 underneath them, and their default vector
//! assignment — IRQs 0 through 7 at vectors 8 through 15 — sits exactly on top
//! of the processor's own exception vectors.
//!
//! So the first time this kernel enables interrupts, a periodic IRQ 0 arrives
//! as vector 8, and vector 8 is the double fault. The exception report would be
//! entirely correct and entirely wrong: it would name a double fault that never
//! happened, with an error code that is really a timer tick, on a machine that
//! was working. That is a bad afternoon, and it is avoidable in twenty lines.
//!
//! # What this does about it
//!
//! Both halves are remapped to vectors 0x30–0x3F and then masked completely.
//! Masking alone would nearly be enough, and nearly is the problem: a masked
//! 8259 can still deliver a *spurious* interrupt on its lowest-priority line
//! when an IRQ is withdrawn between assertion and acknowledgement. Remapped,
//! that arrives at vector 0x37 — a gate this kernel does not install — and is
//! reported as the general protection fault it is. Left where the firmware put
//! it, it arrives at vector 15 and is reported as an exception nobody caused.
//!
//! Deliberately not disabled by way of `IMCR` or the MP tables: interrupt
//! *routing* is the I/O APIC's job and belongs to E0-B15, where there is
//! something to route. This module only ensures that the legacy path cannot
//! deliver anything that would be misread.

use super::port::outb;

/// Command port of the first controller.
const PRIMARY_COMMAND: u16 = 0x20;

/// Data port of the first controller, which is also its interrupt mask.
const PRIMARY_DATA: u16 = 0x21;

/// Command port of the second controller.
const SECONDARY_COMMAND: u16 = 0xA0;

/// Data port of the second controller.
const SECONDARY_DATA: u16 = 0xA1;

/// Begin initialisation: expect three more bytes on the data port.
const INIT: u8 = 0x11;

/// Operate in 8086 mode rather than the 8080 mode the chip predates.
const MODE_8086: u8 = 0x01;

/// Where the first controller's eight lines are sent.
///
/// Above the vectors this kernel uses for real — the exceptions end at 31 and
/// the timer takes 32 — so that nothing the legacy path emits can be confused
/// for something the kernel meant.
pub const PRIMARY_VECTOR_BASE: u8 = 0x30;

/// Where the second controller's eight lines are sent.
pub const SECONDARY_VECTOR_BASE: u8 = 0x38;

/// Remap both controllers out of the exception range, then mask every line.
///
/// # Safety
///
/// Call once, on the boot processor, with interrupts disabled and before they
/// are ever enabled. Ports 0x20, 0x21, 0xA0 and 0xA1 must be the PC-class 8259
/// pair; every machine that can boot a multiboot image on this architecture has
/// them, and a platform that does not is a platform this kernel does not yet
/// claim to run on.
pub unsafe fn remap_and_mask() {
    // The initialisation sequence, in the order the chip requires: a command
    // byte to each controller, then three data bytes to each. Written as data
    // rather than as ten calls for the same reason the serial port's sequence
    // is — one SAFETY comment covering ten operations covers none of them.
    const SEQUENCE: [(u16, u8); 10] = [
        (PRIMARY_COMMAND, INIT),
        (SECONDARY_COMMAND, INIT),
        // Where each controller's lines land.
        (PRIMARY_DATA, PRIMARY_VECTOR_BASE),
        (SECONDARY_DATA, SECONDARY_VECTOR_BASE),
        // How they are wired to each other: the second controller hangs off
        // line 2 of the first, which is the standard cascade and the reason
        // there are fifteen usable lines rather than sixteen.
        (PRIMARY_DATA, 1 << 2),
        (SECONDARY_DATA, 2),
        (PRIMARY_DATA, MODE_8086),
        (SECONDARY_DATA, MODE_8086),
        // Every line masked. The data port means the interrupt mask once
        // initialisation is over, which is why the same port appears twice
        // here meaning two different things.
        (PRIMARY_DATA, 0xFF),
        (SECONDARY_DATA, 0xFF),
    ];

    for (port, value) in SEQUENCE {
        // SAFETY: `port` is one of the four architectural 8259 registers named
        // above, and `value` is the byte the initialisation sequence calls for
        // at that position. The controllers are being taken over, not shared:
        // nothing else in this kernel touches them.
        unsafe { outb(port, value) };
    }
}
