// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Minimal 16550 UART driver on COM1.
//!
//! This exists so that milestone M0 can report, and so that the panic handler
//! has somewhere to write. It is deliberately the only device the kernel knows
//! about at M0 — virtio and everything else arrives in phase 01, on rings.

use core::fmt::{self, Write};

use super::port::{inb, outb};

const COM1: u16 = 0x3F8;

/// COM1, in the state the firmware left it.
pub struct Serial;

impl Serial {
    /// Configure COM1 for 38400 baud, 8N1, FIFOs enabled.
    ///
    /// Safe to call more than once.
    pub fn init(&self) {
        // The standard 16550 initialisation sequence, in order. Written as
        // data rather than as seven calls because the workspace denies
        // multiple unsafe operations per block — on the grounds that one
        // SAFETY comment covering seven operations covers none of them.
        const SEQUENCE: [(u16, u8); 7] = [
            (COM1 + 1, 0x00), // interrupts off while configuring
            (COM1 + 3, 0x80), // enable divisor latch
            (COM1, 0x03),     // divisor low: 38400 baud
            (COM1 + 1, 0x00), // divisor high
            (COM1 + 3, 0x03), // 8 bits, no parity, one stop bit
            (COM1 + 2, 0xC7), // enable and clear FIFOs, 14-byte threshold
            (COM1 + 4, 0x0B), // data terminal ready, request to send
        ];

        for (port, value) in SEQUENCE {
            // SAFETY: `port` is one of the architectural COM1 register offsets
            // listed immediately above, on a PC-class platform. A byte write
            // to a UART register has no memory effect and cannot alias.
            unsafe { outb(port, value) };
        }
    }

    /// Write bytes exactly as given, with no line-ending translation.
    ///
    /// [`Write::write_str`] below turns a newline into a carriage return and a
    /// newline, because that is what a terminal wants from a `println`. This
    /// does not, and the difference is the point: these bytes come out of a
    /// channel's inline arena on behalf of an opcode, and a device that
    /// rewrote its payload would be a device that cannot carry one. What
    /// arrives is what was asked for.
    pub fn write_bytes(&self, bytes: &[u8]) {
        for byte in bytes {
            self.write_byte(*byte);
        }
    }

    /// One byte from COM1, if the port has one waiting.
    ///
    /// # Why a kernel that only ever printed now reads
    ///
    /// Because `cargo xtask gpu` has to tell the machine when it has looked at
    /// the screen. That check's whole subject is a picture on a display, which
    /// is on the far side of the emulator and which nothing inside this machine
    /// can observe — so the harness captures the framebuffer from outside and
    /// the boot has to still be running when it does. A byte on this port is how
    /// the harness says *I have looked*, and `kernel/src/main.rs` bounds the wait
    /// for it so that a harness that never answers is a count rather than a
    /// hang. RFC 0054.
    ///
    /// It is a **poll and not an interrupt**, which is R05 rather than
    /// convenience: nothing in this system is delivered asynchronously, and a
    /// byte on a serial port is not going to be the exception.
    ///
    /// What it is not is a console. There is no line discipline, no buffer and
    /// no reader anywhere else in the tree; the one caller wants to know whether
    /// *anything* arrived. A build that grows a second caller should ask whether
    /// it wants a device rather than this function.
    #[must_use]
    pub fn received(&self) -> Option<u8> {
        // Bit zero of the line status register is *data ready*. Reading it has
        // no side effect; reading the receive buffer below consumes the byte,
        // which is why the two are in this order.
        // SAFETY: reading the line status register of COM1 has no side effect.
        if unsafe { inb(COM1 + 5) } & 0x01 == 0 {
            return None;
        }
        // SAFETY: the line status register says a byte is waiting, so reading
        // the receive buffer takes that byte and nothing else.
        Some(unsafe { inb(COM1) })
    }

    fn write_byte(&self, byte: u8) {
        // Spin until the transmit holding register is empty. Bounded in
        // practice by the UART, and this path exists only for M0 and panics.
        // SAFETY: reading the line status register of COM1 has no side effect.
        while unsafe { inb(COM1 + 5) } & 0x20 == 0 {
            core::hint::spin_loop();
        }
        // SAFETY: the transmit register is ready, per the poll above.
        unsafe { outb(COM1, byte) }
    }
}

impl Write for Serial {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        for byte in s.bytes() {
            if byte == b'\n' {
                self.write_byte(b'\r');
            }
            self.write_byte(byte);
        }
        Ok(())
    }
}

/// Print to COM1. Available before anything else in the system works.
#[macro_export]
macro_rules! kprint {
    ($($arg:tt)*) => {{
        use core::fmt::Write as _;
        let mut serial = $crate::arch::x86_64::serial::Serial;
        let _ = write!(serial, $($arg)*);
    }};
}

/// Print a line to COM1.
#[macro_export]
macro_rules! kprintln {
    () => { $crate::kprint!("\n") };
    ($($arg:tt)*) => {{
        $crate::kprint!($($arg)*);
        $crate::kprint!("\n");
    }};
}
