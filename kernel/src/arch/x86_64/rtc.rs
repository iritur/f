// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The CMOS real-time clock: the only wall clock this machine has.
//!
//! # This is a datum, not a clock
//!
//! Nothing in F may order an event by what this returns. It is read once, at
//! boot, and turned into an [`f_env::WallTime`] — a value carrying its source
//! and its uncertainty, which may be stamped on things and shown to people and
//! may not drive a timer or express a deadline. The clock with ordering
//! authority is [`f_env::Instant`], and it comes from the timestamp counter.
//! RFC 0009.
//!
//! # Why the projection happens here
//!
//! RFC 0009 says civil time does not exist below the semantic layer: no zones,
//! no calendars, no leap seconds. This chip only speaks civil time — a year, a
//! month, a day and a wall-clock hour, in whatever encoding the firmware left
//! it in. So this module is the edge where an outside representation is
//! projected into the one the system uses, which is the place the RFC says that
//! work belongs. Above this file there are only TAI nanoseconds.
//!
//! # What the reading is worth
//!
//! Not much, and the value says so. The CMOS clock is a battery-backed
//! oscillator that nobody disciplines; its quantisation is a second and its
//! drift is not measurable from here. `WallSource::Firmware` is the honest
//! provenance — *the firmware said so* — and a caller who needs better has to
//! obtain it from something disciplined, which this system does not yet have.

use f_env::{WallSource, WallTime};

use super::port::{inb, outb};

/// The register selector. Writing an index here selects what the data port
/// reads.
const INDEX: u16 = 0x70;

/// The data port for whichever register [`INDEX`] last named.
const DATA: u16 = 0x71;

/// Register indices. The gaps are alarm registers, which nothing here uses.
const SECOND: u8 = 0x00;
/// Minutes.
const MINUTE: u8 = 0x02;
/// Hours, whose top bit means PM when the chip is in twelve-hour mode.
const HOUR: u8 = 0x04;
/// Day of the month.
const DAY: u8 = 0x07;
/// Month, one-based.
const MONTH: u8 = 0x08;
/// Year within the century.
const YEAR: u8 = 0x09;

/// The century, on the machines that implement it.
///
/// This index is not architectural. ACPI's fixed description table carries the
/// index of the century register — and says it is zero on a machine that has
/// none — which is the correct way to learn this and is not available until
/// there is an ACPI parser. 0x32 is what every implementation that has one
/// uses, and a value that does not look like a century is discarded below.
const CENTURY: u8 = 0x32;

/// Status register A. Its top bit says an update is in progress.
const STATUS_A: u8 = 0x0A;

/// Status register B. Carries the two bits that say how everything else is
/// encoded.
const STATUS_B: u8 = 0x0B;

/// Status A: the chip is mid-update and every other register is untrustworthy.
const UPDATE_IN_PROGRESS: u8 = 1 << 7;

/// Status B: the registers hold binary rather than binary-coded decimal.
const BINARY: u8 = 1 << 2;

/// Status B: hours run 0 to 23 rather than 1 to 12 with a PM flag.
const HOUR_24: u8 = 1 << 1;

/// The hour register's PM flag, in twelve-hour mode only.
const PM: u8 = 1 << 7;

/// TAI minus UTC, in seconds.
///
/// Thirty-seven since 2017-01-01, and the reason [`f_env::WallTime`] is TAI: a
/// leap second is a discontinuity in UTC, and a discontinuity in a number this
/// system stamps on objects is a bug waiting for a specific date. The General
/// Conference on Weights and Measures voted in 2022 to stop inserting them by
/// 2035.
///
/// *Reversal:* the IERS announces one. Then this constant changes, every stamp
/// taken before the change is wrong by a second, and that — a constant in a
/// kernel that a committee can invalidate — is the whole argument for keeping
/// civil time in the semantic layer.
const TAI_MINUS_UTC: u64 = 37;

/// How wrong this reading could be.
///
/// One hour, and the reasoning matters more than the number. The quantisation
/// is a second. The leap-second offset applied above is exact today. What
/// dominates is drift: an unattended oscillator with a coin cell behind it,
/// running for an unknown length of time since somebody last set it, and cheap
/// parts are specified in minutes per month. So a bound stated in seconds would
/// be a precision claim this system cannot support, and a fabricated precision
/// is exactly what RFC 0009 says is worse than no reading at all.
///
/// What this bound does *not* cover, because no bound of this shape could: a
/// machine whose firmware keeps local time rather than UTC reads a whole number
/// of hours wrong, and nothing in the CMOS says which of the two it is. F reads
/// it as UTC, which is what UEFI requires and what every hypervisor this kernel
/// boots on presents. *Reversal:* a machine that boots with a stamp wrong by
/// hours is one that keeps local time, and the answer is a boot parameter
/// naming the offset — not a wider number here, which would hide it.
const UNCERTAINTY_NANOS: u64 = 3_600 * 1_000_000_000;

/// One reading of the seven registers, in whatever encoding status B declares.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Reading {
    second: u8,
    minute: u8,
    hour: u8,
    day: u8,
    month: u8,
    year: u8,
    century: u8,
}

/// Read one CMOS register.
///
/// # The high bit of the index port
///
/// Bit 7 of [`INDEX`] is the non-maskable interrupt mask, which is a
/// spectacular thing to find in the same register as a clock selector. Writing
/// an index with that bit set disables NMI until somebody writes it clear
/// again, which is how a machine ends up silently ignoring machine checks. This
/// kernel never masks NMI, so every write here has the bit clear — and every
/// register index used here is below 0x80 anyway, which is what makes that
/// automatic rather than remembered.
///
/// # Safety
///
/// Ports 0x70 and 0x71 must be the PC-class CMOS pair, and interrupts must be
/// off. The index and the data are two separate accesses to a device with one
/// selector: anything that runs between them and touches the CMOS leaves this
/// read returning whichever register it selected instead.
unsafe fn register(index: u8) -> u8 {
    // SAFETY: the caller's platform guarantee. The index port takes any value;
    // this one has bit 7 clear, so NMI stays enabled.
    unsafe { outb(INDEX, index & 0x7F) };
    // SAFETY: as above. Reading the data port has no side effect — the CMOS
    // registers are not read-to-clear.
    unsafe { inb(DATA) }
}

/// Whether the chip is mid-update, when every other register is in flux.
///
/// # Safety
///
/// As [`register`].
unsafe fn updating() -> bool {
    // SAFETY: the caller's guarantee, passed down.
    unsafe { register(STATUS_A) & UPDATE_IN_PROGRESS != 0 }
}

/// Spin until the chip is not updating, or give up.
///
/// The bound counts port reads rather than time, because time is what this
/// function is trying to obtain. A machine with no CMOS reads 0xFF from every
/// port, which has the update bit set forever — so the timeout is not a
/// pathological case, it is how the absence of the device is detected.
///
/// # Safety
///
/// As [`register`].
unsafe fn settle() -> bool {
    // A CMOS update lasts about two milliseconds and happens once a second. A
    // port read costs about a microsecond on real hardware and less under
    // emulation, so a million of them is comfortably longer than any update and
    // still bounded.
    const PATIENCE: u32 = 1_000_000;

    for _ in 0..PATIENCE {
        // SAFETY: the caller's guarantee, passed down.
        let busy = unsafe { updating() };
        if !busy {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Read the seven registers once.
///
/// # Safety
///
/// As [`register`], and only meaningful when [`settle`] has just returned true.
unsafe fn sample() -> Reading {
    Reading {
        // SAFETY: the caller's guarantee, passed down, seven times.
        second: unsafe { register(SECOND) },
        // SAFETY: as above.
        minute: unsafe { register(MINUTE) },
        // SAFETY: as above.
        hour: unsafe { register(HOUR) },
        // SAFETY: as above.
        day: unsafe { register(DAY) },
        // SAFETY: as above.
        month: unsafe { register(MONTH) },
        // SAFETY: as above.
        year: unsafe { register(YEAR) },
        // SAFETY: as above.
        century: unsafe { register(CENTURY) },
    }
}

/// What time the firmware says it is, or `None` if this machine has nothing
/// worth believing.
///
/// # Why it is read twice
///
/// The seven registers are seven separate port reads and the chip updates them
/// while nobody is looking. A read that straddles an update gives a second from
/// before it and a minute from after — 10:59:59 becomes 10:00:59, once an hour,
/// on a boot that happens to land there. So the registers are read twice and
/// the reading is accepted only when the two agree, which is the standard
/// defence and the reason this is not four lines.
///
/// # Safety
///
/// As [`register`]: the CMOS pair must be present at 0x70 and 0x71, and
/// interrupts must be off for the whole call.
#[must_use]
pub unsafe fn read() -> Option<WallTime> {
    // Two disagreeing reads mean an update landed between them, which can
    // happen twice in a row on an unlucky boot and cannot happen eight times.
    const ATTEMPTS: u32 = 8;

    for _ in 0..ATTEMPTS {
        // SAFETY: the caller's guarantee, passed down to every access below.
        let idle = unsafe { settle() };
        if !idle {
            return None;
        }
        // SAFETY: as above, immediately after the chip said it was idle.
        let first = unsafe { sample() };
        // SAFETY: as above.
        let idle = unsafe { settle() };
        if !idle {
            return None;
        }
        // SAFETY: as above.
        let second = unsafe { sample() };

        if first != second {
            continue;
        }

        // Read after the values, deliberately: this register says how to
        // interpret them, and reading it first would leave a window where the
        // firmware changed the encoding underneath the reading. Nothing does
        // that, and it costs one line to not depend on it.
        // SAFETY: as above.
        let status = unsafe { register(STATUS_B) };
        return interpret(first, status);
    }
    None
}

/// Turn a raw reading into TAI nanoseconds, or reject it.
///
/// Split out from [`read`] because it is the half that can be tested: it takes
/// bytes and returns a number, with no device under it. [`self_test`] runs it
/// against dates whose answers are known.
fn interpret(reading: Reading, status_b: u8) -> Option<WallTime> {
    let binary = status_b & BINARY != 0;
    let hour24 = status_b & HOUR_24 != 0;

    let second = u64::from(decode(reading.second, binary));
    let minute = u64::from(decode(reading.minute, binary));

    // Twelve-hour mode, which is where the two values everybody gets wrong
    // live: noon is 12 PM and midnight is 12 AM, so twelve wraps to zero and
    // the PM flag adds twelve to *that*. The flag is stripped before decoding,
    // because in binary-coded decimal it would otherwise read as another eight
    // tens.
    let pm = !hour24 && reading.hour & PM != 0;
    let mut hour = u64::from(decode(reading.hour & !PM, binary));
    if !hour24 {
        hour = hour % 12 + if pm { 12 } else { 0 };
    }

    let day = decode(reading.day, binary);
    let month = decode(reading.month, binary);
    let year_in_century = u64::from(decode(reading.year, binary));
    let century = u64::from(decode(reading.century, binary));

    // A century that does not look like one means the register is not there —
    // it reads as zero, or as whatever the firmware left in that byte. Assuming
    // 20xx is not a guess about the machine, it is a guess about the calendar,
    // and it is wrong for the first time in 2100.
    let year = if (19..=21).contains(&century) {
        century * 100 + year_in_century
    } else {
        2000 + year_in_century
    };

    // Everything below rejects rather than corrects. A clock that says the
    // thirty-first of February is a clock whose battery has gone, and a
    // plausible number derived from it is worse than no number: RFC 0009 makes
    // this an Option precisely so this branch exists.
    if !(1970..=2200).contains(&year) || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let utc = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second;
    Some(WallTime {
        tai_nanos: (utc + TAI_MINUS_UTC) * 1_000_000_000,
        uncertainty_nanos: UNCERTAINTY_NANOS,
        source: WallSource::Firmware,
    })
}

/// One register, in whichever encoding status B declared.
///
/// Binary-coded decimal keeps each digit in its own nibble, which is why an
/// hour of 0x59 is fifty-nine minutes and not eighty-nine. A nibble above nine
/// is not a digit at all; this returns a number for it anyway and validation
/// upstream throws the reading out, which is cheaper than a second error path
/// for a case that means the battery is dead.
const fn decode(value: u8, binary: bool) -> u8 {
    if binary { value } else { (value >> 4) * 10 + (value & 0x0F) }
}

/// Whether this year has a twenty-ninth of February.
const fn is_leap(year: u64) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

/// How many days a month has.
const fn days_in_month(year: u64, month: u8) -> u8 {
    let february = if is_leap(year) { 29 } else { 28 };
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => february,
        // Unreachable: the caller checks the range first. Returning zero rather
        // than panicking, because a panic in a kernel is a dead machine and
        // this answer makes every day of a nonexistent month invalid.
        _ => 0,
    }
}

/// Days from 1970-01-01 to this date, in the proleptic Gregorian calendar.
///
/// Howard Hinnant's `days_from_civil`, which is the standard closed form: shift
/// the year to start in March so the leap day lands at the end of it, then count
/// whole four-hundred-year eras, whose length — 146 097 days — is exactly
/// divisible by seven and is what makes the arithmetic branchless.
///
/// The alternative is a loop over years adding 365 or 366, which is correct,
/// slower, and the version people write a leap-year bug into.
const fn days_from_civil(year: u64, month: u8, day: u8) -> u64 {
    // March-based year. The caller guarantees year >= 1970, so this stays
    // positive and the whole calculation fits in unsigned arithmetic.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y / 400;
    let year_of_era = y - era * 400;

    let m = month as u64;
    let shifted = if m > 2 { m - 3 } else { m + 9 };
    let day_of_year = (153 * shifted + 2) / 5 + day as u64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;

    // 719 468 is 1970-01-01 counted from the start of the era containing year
    // zero, which is what turns an era-relative day number into a Unix one.
    era * 146_097 + day_of_era - 719_468
}

/// Check the projection against dates whose answers are known.
///
/// This is the half of the module that can be wrong quietly. A device read that
/// fails is a boot that says so; a calendar that is wrong by a day produces a
/// timestamp that looks entirely reasonable and is not, and nothing downstream
/// can tell. So it is checked on every boot, against the cases that catch the
/// mistakes actually made here: the epoch itself, a century boundary, both
/// twelve o'clocks, and a February the calendar has to get right twice.
///
/// # Errors
///
/// A sentence naming the case that failed.
pub fn self_test() -> Result<(), &'static str> {
    /// Status B for a machine that keeps binary registers and a 24-hour clock.
    const BINARY_24: u8 = BINARY | HOUR_24;

    /// Decimal to binary-coded decimal, for building the cases that are.
    const fn bcd(value: u8) -> u8 {
        ((value / 10) << 4) | (value % 10)
    }

    fn utc_of(reading: Reading, status: u8) -> Option<u64> {
        interpret(reading, status).map(|w| w.tai_nanos / 1_000_000_000 - TAI_MINUS_UTC)
    }

    let epoch = Reading { second: 0, minute: 0, hour: 0, day: 1, month: 1, year: 70, century: 19 };
    if utc_of(epoch, BINARY_24) != Some(0) {
        return Err("1970-01-01 is not the epoch");
    }

    // The century boundary, in the encoding real firmware uses. 946 684 800 is
    // the number every Y2K post-mortem is denominated in.
    let y2k = Reading {
        second: 0,
        minute: 0,
        hour: 0,
        day: bcd(1),
        month: bcd(1),
        year: bcd(0),
        century: bcd(20),
    };
    if utc_of(y2k, HOUR_24) != Some(946_684_800) {
        return Err("2000-01-01 is not where the calendar puts it");
    }

    // An ordinary date, far enough from both boundaries to catch an error in
    // the era arithmetic rather than in a special case.
    let ordinary = Reading {
        second: bcd(56),
        minute: bcd(34),
        hour: bcd(12),
        day: bcd(29),
        month: bcd(8),
        year: bcd(26),
        century: bcd(20),
    };
    if utc_of(ordinary, HOUR_24) != Some(1_788_006_896) {
        return Err("2026-08-29 12:34:56 is not where the calendar puts it");
    }

    // Twelve-hour mode. Midnight is 12 AM and noon is 12 PM, which is the pair
    // that turns into a twelve-hour error in every implementation that treats
    // the flag as an addition.
    let midnight = Reading { second: 0, minute: 0, hour: bcd(12), ..ordinary };
    if utc_of(midnight, 0) != Some(1_787_961_600) {
        return Err("twelve AM is not midnight");
    }
    let noon = Reading { second: 0, minute: 0, hour: bcd(12) | PM, ..ordinary };
    if utc_of(noon, 0) != Some(1_788_004_800) {
        return Err("twelve PM is not noon");
    }
    let one_pm = Reading { second: 0, minute: 0, hour: bcd(1) | PM, ..ordinary };
    if utc_of(one_pm, 0) != Some(1_788_008_400) {
        return Err("one PM is not thirteen hundred");
    }

    // A leap day that exists, and the same date in a year where it does not.
    // 2100 is the case a divisible-by-four rule gets wrong, and it is inside
    // the range this code accepts.
    let leap = Reading {
        second: 0,
        minute: 0,
        hour: 0,
        day: bcd(29),
        month: bcd(2),
        year: bcd(24),
        century: bcd(20),
    };
    if utc_of(leap, HOUR_24) != Some(1_709_164_800) {
        return Err("2024-02-29 is not where the calendar puts it");
    }
    if utc_of(Reading { year: bcd(23), ..leap }, HOUR_24).is_some() {
        return Err("2023 was given a twenty-ninth of February");
    }
    if utc_of(Reading { year: bcd(0), century: bcd(21), ..leap }, HOUR_24).is_some() {
        return Err("2100 was given a twenty-ninth of February");
    }

    // Rejections. A dead battery reads as zeroes or as 0xFF, and neither may
    // become a plausible-looking timestamp.
    let zeroes = Reading { second: 0, minute: 0, hour: 0, day: 0, month: 0, year: 0, century: 0 };
    if interpret(zeroes, BINARY_24).is_some() {
        return Err("an all-zero reading became a timestamp");
    }
    let ones = Reading {
        second: 0xFF,
        minute: 0xFF,
        hour: 0xFF,
        day: 0xFF,
        month: 0xFF,
        year: 0xFF,
        century: 0xFF,
    };
    if interpret(ones, BINARY_24).is_some() {
        return Err("an all-ones reading became a timestamp");
    }
    if interpret(Reading { month: bcd(13), ..ordinary }, HOUR_24).is_some() {
        return Err("a thirteenth month became a timestamp");
    }

    // A machine with no century register: the byte reads as something that is
    // not a century, and the fallback puts the date in the current one rather
    // than in the year 26.
    if utc_of(Reading { century: 0, ..ordinary }, HOUR_24) != Some(1_788_006_896) {
        return Err("a missing century register did not fall back to 20xx");
    }

    // The provenance and the uncertainty are part of the value, not decoration:
    // a caller decides what to do with a reading by looking at them.
    let stamp = interpret(ordinary, HOUR_24).ok_or("an ordinary date produced no stamp")?;
    if stamp.source != WallSource::Firmware {
        return Err("a firmware reading claimed some other source");
    }
    if stamp.uncertainty_nanos != UNCERTAINTY_NANOS {
        return Err("the stated uncertainty is not the one this module claims");
    }

    Ok(())
}
