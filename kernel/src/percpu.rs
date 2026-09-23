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
/// # What it costs, measured
///
/// *Resident* here is the sum of the image's allocated sections — what the
/// linker reserves, loaded or not. The cost is linear in this constant, and
/// exactly so, because every term is an array indexed by it plus `linker.ld`'s
/// `AP_CORES * AP_STACK_STRIDE`, which is why the fit is not a fit:
///
/// ```text
/// resident(N) = 1 909 150 + 64 328 * N bytes      62.8 KiB per core
///
/// **Re-measured on 2026-09-23**, when `E3-B01g` gave every core four
/// doorbell counters and a wakeup latch: the slope moved by 32 bytes and
/// `cargo xtask cores` refused the build until this line, the table in
/// `docs/booting-on-hardware.md` and `xtask`'s `CORE_COST_BYTES` agreed
/// again. Only the slope is gated; the intercept above is not, and it had
/// drifted by more than half a mebibyte before this measurement caught it,
/// which is the check's own point about which of the two is stale more
/// often.
/// ```
///
/// Built at 2, 8, 16, 32 and 64, on 2026-09-17 at `a8e5f31`: the model is taken
/// from the two ends and predicts the middle three to the byte — 1 866 462,
/// 2 380 830 and 3 409 566. The per-core term decomposes exactly, and most of
/// it is not `PerCpu` at all:
///
/// ```text
/// 57 344   one guarded application-processor stack block, AP_STACK_STRIDE
///  4 504   .bss, of which the interrupt descriptor table is 4 096
///  2 448   .data
/// ```
///
/// The stack block is reserved in the image rather than allocated at bring-up
/// because a guard page needs the mapper that builds the kernel window, and
/// that runs long before any core starts.
///
/// **The intercept is this kernel's code and it rots. The slope is this
/// constant and does not.** The intercept read 438 566 here until 2026-09-17,
/// against a `.text` a third of today's — wrong by more than fourteen cores'
/// worth of the per-core term it is the intercept of, while nothing about
/// sharding had moved. So quote the slope, and re-read the intercept before
/// quoting it:
///
/// ```text
/// cargo build -p f-kernel --target x86_64-unknown-none -Zbuild-std=core,compiler_builtins
/// size -A target/x86_64-unknown-none/debug/f-kernel   # every section with an address
/// ```
///
/// A second point for the slope means moving this constant and `AP_CORES`
/// together and building again, and that is now `cargo xtask cores`: two
/// builds, at 2 and at 64, with the **slope** above required to be the
/// difference over the span. It was this comment's own reversal condition and
/// it is discharged — for the slope, and only for the slope. The intercept is
/// deliberately left out, for the reason the condition gave: it is a different
/// number in every commit that changes code, and a check on it would fire in
/// all of them. So the recipe above is still the recipe and still the reader's
/// job to run — and a green `cargo xtask cores` prints the intercept it
/// measured on the way past, which is the number to paste here.
///
/// What the machine withholds is not this figure but the page-rounded span
/// `__kernel_phys_start .. __kernel_end` — 1 871 872 bytes, 457 frames, at
/// eight, and not quite linear because section padding is not. That one is
/// checkable from a boot log rather than from an image: on the 128 MiB fixture
/// the allocator reports `frames 31978 free of 31978`, which is the 32 479
/// frames of usable memory above 1 MiB, less those 457 and the 44 the boot
/// modules take.
///
/// Bring-up costs a further ~10.4 ms per core, and that one is a hardcoded
/// sequential spin: `ap::wake` waits 10 ms after `INIT` and 200 µs after each
/// `STARTUP`, whatever the core actually does.
///
/// **The two costs have different shapes, which is the part worth keeping.**
/// Memory tracks *this constant* and is paid on every machine, including a
/// single-core one that touches none of it. Boot time tracks
/// `present.min(MAX_CPUS)` — the cores that actually exist — so a high ceiling
/// on a small machine costs memory and no time at all.
///
/// # Why it is still eight
///
/// It was raised to 64 for a Threadripper 2990WX, measured, and put back. That
/// experiment is where the shape of the numbers above came from — they have
/// been re-measured since, and the sweep is cheap enough to repeat that the
/// decision need not rest on a remembered figure. The reasoning is that a
/// ceiling is not a speedup. Nothing here schedules work above two cores — `init` runs on one,
/// the timer on another, and every core past that is started, given tables and
/// a stack, and parked — so the cores a larger ceiling admits would have had
/// nothing to do, at 62.8 KiB and 10.4 ms each.
///
/// When it does pay, it will pay as *admission capacity* rather than as
/// throughput: RFC 0007 reserves a core whole, with its SMT sibling and a cache
/// partition, so more cores means more reserved workloads coexisting and not
/// any one of them running faster. Worth having the right word before the first
/// number is published.
///
/// *Reversal:* raise it when a scheduler can place work on the cores it admits,
/// or when E5 names a machine with more cores than this, or when the kernel
/// learns to size the shards from the core count the firmware reports — the
/// last being the same change made properly, and the only one of the three that
/// stops this being a straight line with no knee in it. It needs an allocator
/// that runs before the descriptor tables do.
///
/// `AP_CORES` in `kernel/linker.ld` must equal this less one, and
/// [`crate::arch::x86_64::ap::self_test`] checks that at boot against the
/// linker's own symbols rather than trusting either comment — so raising one
/// and forgetting the other is a refused boot naming the problem, not a
/// corrupted stack.
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
