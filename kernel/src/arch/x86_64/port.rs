// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The two instructions that reach a legacy device.
//!
//! # Why this is a module rather than two functions in the driver that needs
//! them
//!
//! It was, until there were two drivers. The serial port and the 8254 both
//! speak port I/O, and the interesting part of `inb` and `outb` is not the
//! instruction — it is the safety obligation, which is identical for both and
//! is not the sort of argument that improves by being written twice.
//!
//! # What the obligation actually is
//!
//! A port write is a message to whatever device the platform has decided lives
//! at that address, and neither the compiler nor the processor knows which
//! device that is. `nomem` is honest — the instruction touches no memory the
//! compiler can see — and is also exactly what makes this dangerous: the
//! effect is entirely outside the model. So the obligation is on the caller
//! and it is about the *platform*, not about memory.

use core::arch::asm;

/// Write a byte to an I/O port.
///
/// # Safety
///
/// The caller must ensure `port` names a device that tolerates this write.
/// Writing to an arbitrary port can put the platform into an undefined state,
/// and there is no class of value that is safe independently of the port.
pub unsafe fn outb(port: u16, value: u8) {
    // SAFETY: the caller has promised `port` is a valid target for `value`.
    unsafe {
        asm!(
            "out dx, al",
            in("dx") port,
            in("al") value,
            options(nomem, nostack, preserves_flags),
        );
    }
}

/// Read a byte from an I/O port.
///
/// # Safety
///
/// The caller must ensure `port` names a device where a read has no side
/// effect the caller is unprepared for. Reads are not automatically benign:
/// several PC-class registers clear themselves when read.
pub unsafe fn inb(port: u16) -> u8 {
    let value: u8;
    // SAFETY: the caller has promised reading `port` is benign.
    unsafe {
        asm!(
            "in al, dx",
            out("al") value,
            in("dx") port,
            options(nomem, nostack, preserves_flags),
        );
    }
    value
}
