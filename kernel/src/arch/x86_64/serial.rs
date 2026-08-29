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
