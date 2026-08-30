// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The hardware [`Env`]: the one place real time enters the system.
//!
//! # The wiring this file is
//!
//! [`f_env`] states the rule — nothing observes time, randomness or ordering
//! except through an `Env` — and until now the only implementation was the
//! seeded one. That is a substrate with a simulator on one side and nothing on
//! the other: every property of the interface was checked against an
//! environment that could be rewound, and none of it against a machine.
//!
//! This is the other side. `now()` reads the timestamp counter through
//! [`arch::x86_64::read_tsc`], the single allow-listed hardware time source in
//! the tree, and divides by the frequency `apic::calibrate` measured against
//! the 8254. `wall()` carries what the CMOS clock said at boot. Both are checked
//! against [`f_env::contract`] on every boot, which is the same check the seeded
//! environment passes in the host tests — the point being that a property is
//! only worth stating if both implementations are held to it.
//!
//! # What still runs on the seed, and why that is not a contradiction
//!
//! Everything. `kmain` builds a [`f_env::SeededEnv`] and the boot uses it,
//! because the boot log is a fixture: two runs of one commit have to match byte
//! for byte, and a log carrying a number derived from a real clock is a fixture
//! that fails at random. The hardware environment is constructed, checked, and
//! not yet consumed.
//!
//! That is deliberate rather than unfinished. The first thing with a genuine
//! claim on real time is the scheduler at M3, which has to decide when a
//! deadline has passed and cannot ask a virtual clock. Wiring it before then
//! would mean choosing consumers to make the type look used, which is how a
//! substrate acquires call sites nobody needed.

use f_env::{Env, Instant, Scheduler, WallSource, WallTime};

use crate::arch::x86_64::{read_tsc, rtc};

/// A wall-clock reading, and the monotonic instant it was taken at.
#[derive(Clone, Copy)]
struct Anchor {
    /// What the firmware said, with its provenance and its uncertainty.
    stamp: WallTime,
    /// Nanoseconds since this environment's origin, when it was asked.
    at: u64,
}

/// The production environment: a real clock, a real machine.
///
/// One per core. [`Instant`]s from two of these are not comparable — they have
/// different origins, and on a multi-socket machine they are counting different
/// counters — which is the caveat `f_env::Instant` already states and which
/// became load-bearing at E0-B10, where the second core arrives. It adopts this
/// core's measured rate rather than taking its own — `apic::adopt` says why, and
/// what would reverse it — so the *rates* agree; the origins do not, and nothing
/// in this kernel compares two cores' instants.
pub struct Hardware {
    /// Ticks per millisecond, as measured at boot.
    tsc_khz: u64,
    /// The counter reading this environment calls zero.
    origin: u64,
    /// The generator's state.
    state: u64,
    /// The wall clock, on a machine that has one worth reading.
    anchor: Option<Anchor>,
}

impl Hardware {
    /// Build the environment this core will run on.
    ///
    /// `tsc_khz` comes from `apic::calibrate`, which has already rejected a
    /// frequency outside any band a working machine produces — so the clamp
    /// below cannot trigger. It is there because the alternative to a clamp is a
    /// division that faults, and a divide error during boot is a machine that
    /// stops with no output.
    ///
    /// # Safety
    ///
    /// Call once per core, on that core, after `apic::calibrate` on the same
    /// core, with interrupts disabled. The wall clock is read through the CMOS
    /// index and data ports, which are two accesses to a device with one
    /// selector: an interrupt landing between them returns whichever register
    /// the handler selected. See [`rtc::read`].
    #[must_use]
    pub unsafe fn new(tsc_khz: u64) -> Self {
        let origin = read_tsc();
        let mut env =
            Self { tsc_khz: tsc_khz.max(1), origin, state: seed_from(origin), anchor: None };

        // SAFETY: the caller's contract — this core, interrupts off — passed
        // straight down to the port pair.
        if let Some(stamp) = unsafe { rtc::read() } {
            env.anchor = Some(Anchor { stamp, at: env.now().as_nanos() });
        }
        env
    }
}

/// Turn a count of timestamp-counter ticks into nanoseconds.
///
/// # Why this is not one multiply and one divide
///
/// Because `ticks * 1_000_000 / tsc_khz` overflows. At three and a half
/// gigahertz the product passes `u64::MAX` after about ninety minutes of
/// uptime, and what it does then is not fail — it wraps, and the monotonic
/// clock jumps backwards by nine minutes at an interval no test is long enough
/// to reach. A clock that is correct for the first hour of every boot is
/// exactly the bug that gets found in production.
///
/// So the division comes first: whole seconds exactly, then the remainder,
/// which is smaller than one second's worth of ticks and therefore has room to
/// be scaled. The result is exact to the nanosecond and runs for the 584 years
/// the type allows. [`self_test`] checks the case the naive form gets wrong.
pub(crate) const fn nanos_from_ticks(ticks: u64, tsc_khz: u64) -> u64 {
    let per_second = tsc_khz.saturating_mul(1_000);
    if per_second == 0 {
        return 0;
    }
    let seconds = ticks / per_second;
    let remainder = ticks % per_second;
    seconds
        .saturating_mul(1_000_000_000)
        .saturating_add(remainder.saturating_mul(1_000_000) / tsc_khz)
}

/// Carry a wall-clock reading forward to now.
///
/// # Why the clock is read once and not per call
///
/// Two reasons, and the second is the one that matters. Reading the CMOS costs
/// a dozen port accesses and a wait for the update flag, which is microseconds
/// for a value nobody is measuring. And the CMOS is *set by people*: a firmware
/// menu, a hypervisor resynchronising its guest, an administrator correcting a
/// drift. A stamp taken twice in one function would then be able to move
/// backwards between two lines that both look correct — which is the family of
/// bugs RFC 0009 exists to make unavailable, arriving through the one clock the
/// RFC allows to jump.
///
/// So the reading is anchored to a monotonic instant and carried forward by
/// monotonic elapsed time. The uncertainty does not grow with the interval:
/// stating a drift rate would be a claim about an oscillator this system has
/// not measured, and the constant it starts from is already dominated by that
/// same unmeasured drift.
const fn projected(anchor: Anchor, now: Instant) -> WallTime {
    WallTime {
        // Saturating both ways. `now` is always at or after the anchor, so the
        // subtraction cannot go negative — and if a future change makes it able
        // to, the answer is the anchor rather than an interval of six hundred
        // years.
        tai_nanos: anchor.stamp.tai_nanos.saturating_add(now.as_nanos().saturating_sub(anchor.at)),
        uncertainty_nanos: anchor.stamp.uncertainty_nanos,
        source: anchor.stamp.source,
    }
}

/// Turn a counter reading into a generator state worth using.
///
/// One round of the splitmix64 finaliser. The timestamp counter's low bits move
/// and its high bits do not, and xorshift is a linear map over that state: seed
/// it with a number whose top half is effectively constant and the top half of
/// the stream moves slowly for a long time. The mixer costs four instructions
/// once, at boot.
const fn seed_from(ticks: u64) -> u64 {
    let mut z = ticks.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    // All-zero is xorshift's fixed point. Reachable here only from one exact
    // counter value, which is not a reason to leave a state that produces
    // nothing but zeroes forever.
    if z == 0 { 0x9E37_79B9_7F4A_7C15 } else { z }
}

impl Scheduler for Hardware {
    /// The first ready alternative.
    ///
    /// Not a random one, deliberately. Under simulation `choose` is where the
    /// seed explores interleavings adversarially; in production it is where the
    /// real scheduler expresses its policy, and there is no scheduler until M3.
    /// Until then the honest production answer is the one a machine with a
    /// single runnable thing gives — and putting a random choice here instead
    /// would be simulator behaviour leaking into the machine, making a
    /// production run unreproducible for no benefit anybody asked for.
    ///
    /// *Reversal:* E0-B09 and M3. This becomes a call into the run queue, and
    /// the `Env` method stops being where the policy lives.
    fn choose(&mut self, _n: u32) -> u32 {
        0
    }
}

impl Env for Hardware {
    fn now(&self) -> Instant {
        // The one legitimate hardware time source in the tree, reached through
        // the one function permitted to contain it. RFC 0004.
        Instant(nanos_from_ticks(read_tsc().wrapping_sub(self.origin), self.tsc_khz))
    }

    /// # What this generator is, and what it is not
    ///
    /// xorshift64, seeded once from the counter. It is not cryptographic, and
    /// the one consumer it has today does not need it to be: the frame
    /// allocator masks its free-list links, which is a defence against
    /// following a corrupted pointer, not against an adversary who can already
    /// read kernel memory.
    ///
    /// *Reversal:* capability tokens and address-space layout at M4 do need
    /// unpredictability against an adversary. That is where a hardware source
    /// and a real construction go — behind this same method, so that no call
    /// site changes when they do.
    fn next_u64(&mut self) -> u64 {
        self.state ^= self.state << 13;
        self.state ^= self.state >> 7;
        self.state ^= self.state << 17;
        self.state
    }

    fn wall(&self) -> Option<WallTime> {
        self.anchor.map(|anchor| projected(anchor, self.now()))
    }

    fn scheduler(&mut self) -> &mut dyn Scheduler {
        self
    }
}

/// Check the arithmetic this environment is built on.
///
/// The device half of it announces its own failures: a counter that does not
/// tick is a boot that never finishes, a CMOS that does not answer is a `None`.
/// The conversions are the half that can be wrong quietly — an overflow that
/// only appears after an hour of uptime, a projection that loses the source it
/// was carrying — so they are checked on every boot with numbers whose answers
/// are known.
///
/// # Errors
///
/// A sentence naming the case that failed.
pub fn self_test() -> Result<(), &'static str> {
    // A round gigahertz, where a tick is a nanosecond and every answer can be
    // read off by eye.
    const GHZ: u64 = 1_000_000;

    if nanos_from_ticks(0, GHZ) != 0 {
        return Err("no ticks is not no time");
    }
    if nanos_from_ticks(1, GHZ) != 1 {
        return Err("one tick at a gigahertz is not a nanosecond");
    }
    if nanos_from_ticks(1_000_000_000, GHZ) != 1_000_000_000 {
        return Err("a gigatick at a gigahertz is not a second");
    }
    // A frequency that does not divide evenly, which is every real one.
    if nanos_from_ticks(3_400_000, 3_400_000) != 1_000_000 {
        return Err("a millisecond of ticks at 3.4 GHz is not a millisecond");
    }

    // The case the naive form gets wrong. Five hours at 3.4 GHz: the product of
    // the ticks and a million passes u64, and the split form is still exact.
    const HOURS: u64 = 5;
    let ticks = 3_400_000 * 1_000 * 3_600 * HOURS;
    if ticks.checked_mul(1_000_000).is_some() {
        return Err("the overflow this test exists for no longer overflows");
    }
    if nanos_from_ticks(ticks, 3_400_000) != HOURS * 3_600 * 1_000_000_000 {
        return Err("five hours of ticks is not five hours");
    }

    // A frequency of zero cannot reach here — calibration rejects it and the
    // constructor clamps it — and must not divide by zero if it ever does.
    if nanos_from_ticks(1_000, 0) != 0 {
        return Err("a stopped counter produced a duration");
    }

    // The projection carries the value forward and the provenance unchanged. A
    // stamp that loses its source on the way through is a stamp nobody can
    // reason about, which is the whole reason the source is part of the type.
    let anchor = Anchor {
        stamp: WallTime {
            tai_nanos: 1_788_006_896_000_000_000,
            uncertainty_nanos: 3_600 * 1_000_000_000,
            source: WallSource::Firmware,
        },
        at: 500,
    };
    let later = projected(anchor, Instant(500 + 2_000_000_000));
    if later.tai_nanos != 1_788_006_898_000_000_000 {
        return Err("two seconds of elapsed time did not move the wall clock two seconds");
    }
    if later.source != WallSource::Firmware {
        return Err("the projection lost the source it was carrying");
    }
    if later.uncertainty_nanos != anchor.stamp.uncertainty_nanos {
        return Err("the projection lost the uncertainty it was carrying");
    }
    // Before the anchor: impossible from a monotonic clock, and it saturates
    // rather than wrapping to six hundred years in the future.
    if projected(anchor, Instant(0)).tai_nanos != anchor.stamp.tai_nanos {
        return Err("a projection backwards did not saturate");
    }

    // A degenerate seed must not leave the generator at its fixed point.
    if seed_from(0) == 0 || seed_from(u64::MAX) == 0 {
        return Err("the generator was seeded with xorshift's fixed point");
    }

    Ok(())
}
