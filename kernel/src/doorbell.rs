// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The kernel's half of the doorbell: a vector, and something to count.
//!
//! `f_ring::doorbell` owns the protocol — when to ring, and the two counts that
//! make it measurable. This owns the one thing a ring cannot: an interrupt
//! actually arriving somewhere.
//!
//! # Why this is the third vector in the table, and not a mailbox
//!
//! The shootdown vector next door carries information — which page, and which
//! sequence number — so `smp` has two words per core for it, and `RFC 0016`
//! counts them. A doorbell carries nothing. The entry is already in the ring
//! and the cursor that publishes it is already visible; the whole content of
//! the signal is *stop halting*. So this needs no shared word, and the count
//! below is a per-core counter that only its own core writes.
//!
//! That is worth stating rather than assuming, because the obvious design —
//! a shared "doorbells pending" word per core — would have been a fifth address
//! two cores reach, and `CLAUDE.md` says a fifth needs an argument. It does not
//! need one because it does not exist.
//!
//! # What is proven at boot, and what is not
//!
//! Delivery is proven **to this core**: the boot self-test sends the vector to
//! the core it is running on and requires the count to advance. That exercises
//! the whole path — the interrupt command register, the delivery, the gate, the
//! handler, the acknowledgement — with one thing left out, which is the second
//! core.
//!
//! Cross-core delivery is not proven here and the reason is not shyness: the
//! only way to observe another core's count is to read another core's slot,
//! which is the fifth cross-core address the paragraph above is glad not to
//! need. Proving it costs either that word or a rendezvous through the mailbox
//! `smp` already has, and both belong with the component that will actually
//! sleep on a doorbell rather than with the vector.

use core::sync::atomic::{Ordering, compiler_fence};

use f_ring::Ringer;

use crate::arch::x86_64::{ap, apic, current_cpu};
use crate::percpu::PerCpu;

/// Doorbells delivered to each core.
///
/// Written by the handler on the core it was delivered to and read by that same
/// core, always volatilely and never through a reference — the handler and the
/// code it interrupted are both looking at it, which is the case `percpu.rs`
/// says no per-CPU abstraction can see. Unit: doorbells.
static DELIVERED: PerCpu<u64> = PerCpu::new(0);

/// Answer a doorbell: count it, and say it is over.
///
/// There is nothing else to do. The signal's entire content is that an
/// interrupt arrived, and its effect is that a halted core is no longer halted
/// — which has already happened by the time this runs.
///
/// # Safety
///
/// Call from the doorbell vector's own gate, on the core it was delivered to,
/// with interrupts disabled by that gate.
pub(crate) unsafe fn answer() {
    let slot = DELIVERED.mine();
    // SAFETY: this core's counter. The interrupted code may be reading it, and
    // is doing so volatilely for the same reason, which is why neither side
    // takes a reference.
    let seen = unsafe { slot.read_volatile() };
    // SAFETY: as above. This core is inside the handler, so nothing else on it
    // is writing.
    unsafe { slot.write_volatile(seen.wrapping_add(1)) };

    // SAFETY: this core, inside the handler for the interrupt being
    // acknowledged. Last, so that the count is written before the local APIC
    // will deliver another.
    unsafe { apic::end_of_interrupt() };
}

/// Doorbells this core has been delivered. Unit: doorbells.
#[must_use]
pub fn delivered() -> u64 {
    // SAFETY: a volatile read of this core's counter, which the handler writes
    // volatilely. No reference is taken on either side.
    unsafe { DELIVERED.mine().read_volatile() }
}

/// Ring `cpu`'s doorbell.
///
/// # Safety
///
/// The local APIC must be mapped and `cpu` must be a core that is running, or
/// the interrupt is delivered to nobody and the interrupt command register is
/// left holding a command for a destination that does not answer.
pub unsafe fn ring(cpu: usize) {
    // The compiler may not move the ring above whatever published the work it
    // is announcing. There is no *hardware* ordering question here — the
    // interrupt is a message and the store buffer is drained by the write to
    // the command register — but a reordered call would announce work that has
    // not been written, and that is a compiler question.
    compiler_fence(Ordering::Release);

    // SAFETY: the caller's guarantee, and a vector `idt::init` installs on
    // every core.
    unsafe { ap::send(apic::window(), cpu, apic::DOORBELL_VECTOR) };
}

/// The doorbell as `f_ring` sees it: something with a `ring`.
///
/// Carries the core it rings, which is why the trait takes an implementor
/// rather than a function — `f_ring` has no idea what an APIC identifier is and
/// should not acquire one.
pub struct Ipi {
    target: usize,
}

impl Ipi {
    /// A doorbell that wakes `cpu`.
    #[must_use]
    pub fn to(cpu: usize) -> Self {
        Self { target: cpu }
    }

    /// A doorbell that wakes the core it is rung on.
    ///
    /// What the boot self-test uses. A self-directed inter-processor interrupt
    /// is a real delivery and not a shortcut — the command register, the local
    /// APIC's own routing, the gate and the acknowledgement are all the ones a
    /// cross-core ring would use — and it needs no second core to be running
    /// and no way to read that core's counters.
    #[must_use]
    pub fn to_self() -> Self {
        Self { target: current_cpu() }
    }
}

impl Ringer for Ipi {
    fn ring(&mut self) {
        // SAFETY: the local APIC is mapped before anything builds one of these
        // — `apic::window` is what would fault otherwise — and `target` is a
        // core this kernel started or is this one.
        unsafe { ring(self.target) };
    }
}
