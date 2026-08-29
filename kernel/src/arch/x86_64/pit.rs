// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The 8254, used once, as the thing every other clock is measured against.
//!
//! # Why a chip from 1982 is the reference
//!
//! Because its frequency is a constant and everything else's is not. The
//! timestamp counter runs at a rate the processor does not have to tell you,
//! the local APIC's timer runs at a rate derived from a bus clock it also does
//! not have to tell you, and both of those are the clocks this kernel wants to
//! use. Calibration is the operation that turns counts into seconds, and it
//! needs one clock whose rate is known without being asked.
//!
//! The 8254 is that clock. It is driven at 1.193182 MHz on every PC-class
//! machine — a third of the original colour-burst frequency, kept for
//! compatibility long after the reason for it evaporated — and the number has
//! not moved in forty years.
//!
//! # Channel 2, and why it is the only channel usable here
//!
//! Channel 0 is wired to the interrupt controller and channel 1 to memory
//! refresh on machines old enough to need it. Channel 2 is wired to the
//! speaker, which sounds like the least useful of the three and is in fact the
//! only one that works for this: its gate is *software controlled* through port
//! 0x61, and its output is *readable* through the same port. So it can be
//! started at a moment this code chooses and polled to completion without an
//! interrupt controller, without an interrupt handler, and without the timer
//! this calibration exists to bring up.
//!
//! Nothing here makes a sound. The speaker enable is the next bit along and is
//! explicitly cleared.
//!
//! # After this
//!
//! The 8254 is used at boot and never again. It is not a time source for the
//! system; it is a ruler, applied once.

use super::port::{inb, outb};

/// The input frequency, in hertz. Fixed by the platform, not by this chip.
pub const HZ: u64 = 1_193_182;

/// Mode and command register.
const COMMAND: u16 = 0x43;

/// Channel 2's data register.
const CHANNEL2: u16 = 0x42;

/// The control port carrying channel 2's gate, the speaker enable, and a
/// readable copy of channel 2's output.
const CONTROL: u16 = 0x61;

/// Channel 2's gate. High means counting.
const GATE: u8 = 1 << 0;

/// The speaker. Kept low throughout, which is the difference between a
/// calibration and a noise.
const SPEAKER: u8 = 1 << 1;

/// Channel 2's output, as read back. Goes high at terminal count in mode 0.
const OUTPUT: u8 = 1 << 5;

/// Select channel 2, access low byte then high byte, mode 0, binary.
///
/// Mode 0 — interrupt on terminal count — is the one that counts down once and
/// stops, raising its output and leaving it raised. That is what makes it an
/// interval rather than a wave, and the raised output is the edge this module
/// polls for.
///
/// The mode field is bits 3 to 1, which is one bit further left than it looks:
/// `0b1011_0010` is the byte every calibration example on the internet uses and
/// it selects mode *1*, not mode 0. Mode 1 is retriggered by the gate and holds
/// its output high until then, so a gate raised after the count is loaded reads
/// as already expired — a ten-millisecond interval that measures eighty
/// microseconds, and a calibration wrong by two orders of magnitude that still
/// produces a confident-looking number.
const MODE0_CHANNEL2: u8 = 0b1011_0000;

/// The counter value for an interval, or `None` if it will not fit.
///
/// The counter is sixteen bits at 1.193182 MHz, so the longest interval the
/// chip can express in one pass is a little under 55 ms.
#[must_use]
pub const fn counts_for_micros(micros: u32) -> Option<u16> {
    let counts = HZ * micros as u64 / 1_000_000;
    if counts == 0 || counts > u16::MAX as u64 { None } else { Some(counts as u16) }
}

/// How long a calibration interval is.
///
/// Ten milliseconds. The instinct is to make this as long as the counter
/// allows, on the grounds that a longer interval gives a more accurate
/// frequency — and it does, but accuracy is not what the frequency is for.
///
/// The timer schedule is computed in timestamp-counter ticks, so an error in
/// the measured frequency makes a 1 kHz timer run at 1.001 kHz. It does not
/// make any individual tick late, because lateness is measured against the same
/// counter the schedule is expressed in. The frequency only scales the
/// conversion to nanoseconds at the end. Ten milliseconds is good to well under
/// a tenth of a percent, which is far below anything that matters here, and it
/// costs ten milliseconds of every boot rather than fifty.
pub const CALIBRATE_MICROS: u32 = 10_000;

/// The calibration interval, as a counter value.
///
/// A `const` rather than a runtime conversion so that an interval that does not
/// fit the chip is a build failure, not a `None` somebody has to remember to
/// handle.
const CALIBRATE_COUNTS: u16 = match counts_for_micros(CALIBRATE_MICROS) {
    Some(counts) => counts,
    None => panic!("the calibration interval does not fit in the 8254's counter"),
};

/// Start channel 2 counting down from `counts`, with the gate open.
///
/// # The order of these four writes is the whole function
///
/// Gate first, then the mode, then the counter. In mode 0 the countdown begins
/// when the counter is loaded and the output drops at that moment, so the
/// counter has to be the *last* thing written — which means the gate has to be
/// open before it, not after.
///
/// The instinct is the other way round: hold the gate shut, set everything up,
/// then open it, so that the interval starts at a moment this code chose. That
/// is right for mode 1 and wrong here, and getting it wrong does not fail — it
/// returns an interval that has already elapsed.
///
/// # Safety
///
/// Ports 0x42, 0x43 and 0x61 must be the PC-class 8254 and its control
/// register. That is true of every machine which can boot a multiboot image on
/// this architecture, and stops being true on a platform that has removed the
/// legacy device — which is a platform this kernel does not yet claim to run
/// on.
unsafe fn start(counts: u16) {
    // SAFETY: reading 0x61 has no side effect; it latches what was written to
    // it, plus two read-only status bits.
    let control = unsafe { inb(CONTROL) };

    // Gate high, speaker low. Every other bit in the register belongs to
    // somebody else and is preserved: this is a compatibility port with
    // unrelated bits in it, and assigning rather than masking is how a
    // calibration turns into a reset on some chipsets.
    // SAFETY: as above, and both bits touched here are channel 2's own.
    unsafe { outb(CONTROL, (control & !SPEAKER) | GATE) };

    // SAFETY: the mode byte selects channel 2 and a legal access pattern.
    unsafe { outb(COMMAND, MODE0_CHANNEL2) };
    // SAFETY: low byte then high byte, which is the access pattern the mode
    // byte just declared. Writing one without the other leaves the chip
    // half-loaded and waiting for the rest.
    unsafe { outb(CHANNEL2, counts as u8) };
    // SAFETY: as above. This is the write that starts the countdown and drops
    // the output, so it is deliberately last.
    unsafe { outb(CHANNEL2, (counts >> 8) as u8) };
}

/// Whether channel 2 has reached terminal count.
///
/// # Safety
///
/// As [`start`].
unsafe fn expired() -> bool {
    // SAFETY: reading the control port is free of side effects.
    unsafe { inb(CONTROL) & OUTPUT != 0 }
}

/// Close the gate.
///
/// # Safety
///
/// As [`start`].
unsafe fn stop() {
    // SAFETY: reading the control port is free of side effects.
    let control = unsafe { inb(CONTROL) };
    // SAFETY: dropping channel 2's gate stops it counting. The speaker bit goes
    // with it, because leaving that set is how a machine ends up humming.
    unsafe { outb(CONTROL, control & !(GATE | SPEAKER)) };
}

/// Why a calibration produced nothing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CalibrateError {
    /// The gate never rose. Either channel 2 is not there, or its output is not
    /// readable at port 0x61 — both of which are true of some virtual
    /// platforms, and neither of which is worth hanging a boot over.
    ///
    /// This is the whole reason the spin is bounded. A calibration that hangs
    /// is a machine that stops with no output, which is the failure mode the
    /// exception report exists to have removed and a poor thing to reintroduce
    /// one milestone later.
    NoReference,
}

impl CalibrateError {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::NoReference => "the 8254 never reached terminal count; no reference clock",
        }
    }
}

/// Run one calibration interval, sampling `probe` at both ends of it.
///
/// The closure is whatever is being measured. It is called immediately after
/// the gate rises and again immediately after the output goes high, and the two
/// samples are returned in that order.
///
/// Interrupts are the caller's business and are expected to be off: one landing
/// between the gate and the first sample makes the measured interval longer
/// than the chip says it was, which inflates every frequency derived from it.
///
/// # Errors
///
/// [`CalibrateError::NoReference`] if the gate does not rise within a generous
/// bound. The bound counts port reads rather than time, because time is exactly
/// what is not yet known.
///
/// # Safety
///
/// As [`start`].
pub unsafe fn calibrate_micros<T>(mut probe: impl FnMut() -> T) -> Result<(T, T), CalibrateError> {
    // Port reads to allow before giving up. A read of an emulated port costs
    // hundreds of nanoseconds and a read of a real one about a microsecond, so
    // ten million is somewhere between ten seconds and a minute — far longer
    // than the ten-millisecond interval it is waiting on, and still bounded.
    const PATIENCE: u32 = 10_000_000;

    // SAFETY: the caller's platform guarantee, passed down.
    unsafe { start(CALIBRATE_COUNTS) };
    let before = probe();

    let mut spins = 0u32;
    loop {
        // SAFETY: as above.
        if unsafe { expired() } {
            break;
        }
        spins += 1;
        if spins == PATIENCE {
            // SAFETY: as above. The gate is closed on the failure path too —
            // leaving it open leaves the chip counting into a boot that has
            // decided not to look at it again.
            unsafe { stop() };
            return Err(CalibrateError::NoReference);
        }
        core::hint::spin_loop();
    }

    let after = probe();
    // SAFETY: as above.
    unsafe { stop() };

    Ok((before, after))
}
