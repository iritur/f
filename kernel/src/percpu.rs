// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Kernel state, sharded by core.
//!
//! # Why this exists while one core is running
//!
//! `docs/design/ring-scene-boot.html` section 14 names two decisions worth
//! making before they are forced, and this is the first: make all kernel state
//! per-CPU from the very first allocation, behind a `PerCpu<T>`, even while
//! only one core is running.
//!
//! The reason is not symmetry. Retrofitting a shard onto state that is reached
//! as a global is a refactor that touches every call site, and it arrives at
//! the exact moment the kernel is also being debugged on more than one core for
//! the first time. Done now it is a type and three call sites; done at M3 it is
//! the miserable refactor the design document warns about, competing for
//! attention with an SMP bring-up bug.
//!
//! # What it does not do
//!
//! It does not lock. Two cores never reach the same slot, which is the whole
//! premise — a lock here would be a confession that the sharding is not
//! believed.
//!
//! Within one core a slot can still be reached twice: an interrupt handler that
//! touches the same shard as the code it interrupted races with itself, and no
//! per-CPU abstraction can see that. So [`PerCpu::mine`] hands out a raw
//! pointer rather than a reference. The obligation is discharged where the
//! write happens, in an `unsafe` block with a `SAFETY` comment naming why no
//! second reference to that slot is live — not laundered through a safe `&mut`
//! that would be unsound the first time an interrupt lands in the wrong place.
//!
//! # What may be a static in this kernel
//!
//! Exactly this. `cargo xtask lint-percpu` fails the build on a `static mut`,
//! or on a `static` carrying a cell, a lock or an atomic, anywhere under
//! `kernel/` except this file. The policy is enforceable precisely because
//! there is one type it has to make an exception for.

use core::cell::UnsafeCell;

use crate::arch::x86_64::current_cpu;

/// How many cores state is sharded for.
///
/// Every `PerCpu<T>` costs `MAX_CPUS * size_of::<T>()` whether or not the
/// machine has that many cores, so this is a memory number as much as a
/// topology one: at eight, the largest shard in the kernel — the interrupt
/// descriptor table — is 32 KiB of `.bss`.
///
/// *Reversal:* raise it when E5 names a machine with more cores than this, or
/// when the kernel learns to size the shards from the core count the firmware
/// reports, which is the same change made properly and needs an allocator that
/// runs before the descriptor tables do.
pub const MAX_CPUS: usize = 8;

/// One `T` per core, reachable only by the core it belongs to.
///
/// The array is one `UnsafeCell` rather than an array of them because a slot is
/// addressed by pointer arithmetic from the base, which is the operation the
/// hardware also performs when it reads a descriptor table out of one of these
/// slots behind the kernel's back.
pub struct PerCpu<T> {
    slots: UnsafeCell<[T; MAX_CPUS]>,
}

// SAFETY: `Sync` here is the claim that a shared reference can cross cores,
// which is true because the only thing a core can do with that reference is
// take a pointer to its *own* slot: `mine` indexes by the core it is called on,
// and `at` carries the obligation in its signature. No two cores can reach the
// same `T`, so there is nothing for two cores to race over.
unsafe impl<T: Send> Sync for PerCpu<T> {}

impl<T: Copy> PerCpu<T> {
    /// Every slot starts as a copy of `value`.
    ///
    /// `const` because these are statics: a shard built at run time would need
    /// somewhere to live before the allocator exists, which is the bootstrap
    /// problem the frame allocator already refuses to have.
    pub const fn new(value: T) -> Self {
        Self { slots: UnsafeCell::new([value; MAX_CPUS]) }
    }
}

impl<T> PerCpu<T> {
    /// A pointer to the calling core's slot.
    ///
    /// Safe to call and unsafe to dereference, which is the honest split: the
    /// address is arithmetic, and the aliasing question — is a second reference
    /// to this slot live on this core, in an interrupt handler or otherwise —
    /// can only be answered where the access happens.
    #[must_use]
    pub fn mine(&self) -> *mut T {
        self.at(current_cpu())
    }

    /// A pointer to `cpu`'s slot.
    ///
    /// The escape hatch, and the one that has to be justified at every use:
    /// naming another core's slot is exactly what the type exists to prevent.
    /// It is here because bringing a core up means preparing its state before
    /// it can prepare its own (E0-B10), and because [`self_test`] has to look
    /// at slots it does not own to prove they are distinct.
    ///
    /// # Panics
    ///
    /// If `cpu` is not a core this kernel shards for. That is a build
    /// misconfiguration — [`MAX_CPUS`] smaller than the machine — and a panic
    /// naming it is better than silently sharing slot zero between two cores,
    /// which is the failure this whole module exists to make impossible.
    #[must_use]
    pub fn at(&self, cpu: usize) -> *mut T {
        assert!(cpu < MAX_CPUS, "core index beyond MAX_CPUS: raise it in kernel::percpu");
        // One `T` per core, laid out as an array, so the slot is the base
        // pointer advanced by the core index. `wrapping_add` rather than `add`
        // because the bound above is what makes it in-range, and saying so once
        // is better than an `unsafe` block that says it again.
        self.slots.get().cast::<T>().wrapping_add(cpu)
    }
}

/// Prove the shard arithmetic before anything depends on it.
///
/// Three properties, none of which survives being assumed: this core's index is
/// one the kernel shards for, every slot is a distinct address, and a write
/// through one slot is invisible to the others. The third is the one that
/// catches a real mistake — a `PerCpu` that returns the same pointer for every
/// core looks perfectly correct on a machine with one core, and stops looking
/// correct on the day the second one boots, which is the worst day to find out.
///
/// Returns the calling core's index.
pub fn self_test() -> Result<usize, &'static str> {
    static PROBE: PerCpu<u64> = PerCpu::new(0);

    let me = current_cpu();
    if me >= MAX_CPUS {
        return Err("this core's index is beyond MAX_CPUS");
    }

    for cpu in 0..MAX_CPUS {
        // SAFETY: single core, boot path, and `PROBE` is reached from nowhere
        // else in the kernel — so no second reference to any slot is live.
        unsafe { PROBE.at(cpu).write(0xC0FF_EE00 + cpu as u64) };
    }

    for cpu in 0..MAX_CPUS {
        // SAFETY: as above.
        if unsafe { PROBE.at(cpu).read() } != 0xC0FF_EE00 + cpu as u64 {
            return Err("a write to one core's slot disturbed another's");
        }
    }

    if PROBE.mine() != PROBE.at(me) {
        return Err("this core's slot is not the one its index names");
    }

    Ok(me)
}
