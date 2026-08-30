// SPDX-License-Identifier: Apache-2.0 OR MIT
//! More than one core, and the two things that requires: a way to tell a core
//! something, and a way to take a mapping back from one.
//!
//! # The rule this file bends, on purpose
//!
//! `percpu` says two cores never reach the same slot, and that is what makes
//! every other shard in this kernel lock-free by construction rather than by
//! discipline. A handshake between cores cannot obey it: somebody has to write
//! where somebody else reads, or there is no handshake.
//!
//! So the rule becomes narrower rather than weaker, and RFC 0016 is the
//! argument. Everything in this file is `PerCpu<u64>` — a machine word, in the
//! slot of the core it is *about*, written by another core and read by its
//! owner or the other way round, and every access on both sides is an atomic
//! with an ordering named at the access. Four words, and they are the entire
//! set of addresses in this kernel that two cores reach. Nothing else changes:
//! there is still no lock, and a shard holding anything larger than a word is
//! still a shard one core owns.
//!
//! # The ordering, and why it is not `Relaxed`
//!
//! Both protocols here publish something that is *not* the word being written.
//!
//! Bringing a core up publishes its handoff — the address space to run in, the
//! register window to use, the clocks to believe — and the word that says
//! "ready" is the last write. A core that read the word `Relaxed` could see the
//! flag and then read a handoff that had not arrived, which on x86 never
//! happens and on AArch64 happens. The store is `Release` and the load is
//! `Acquire`, and that pair is the whole of why the handoff is safe to read.
//!
//! A shootdown publishes a *page table edit*. The initiator has already written
//! a not-present entry; the sequence number it stores afterwards is what makes
//! that write visible to the core it is asking to invalidate. Getting this
//! wrong is a core that invalidates its buffer and then reloads the entry it
//! was told to forget — which is a stale translation with a fresh timestamp,
//! and it looks exactly like the bug it was supposed to fix.
//!
//! # What a shootdown is for here
//!
//! Revocation. A frame capability that has been mapped is two things — a name
//! and a translation — and until this file existed, revoking it withdrew the
//! name and left the translation. `arch::x86_64::paging` said so at length and
//! said the second core was what would fix it, which was right in a way worth
//! being precise about: the fix is not that another core is *available*, it is
//! that another core must be *told*, and a kernel with one core has nobody to
//! tell and so no reason to build the mechanism. See `process::withdraw`.

use core::sync::atomic::{AtomicU64, Ordering};

use crate::arch::x86_64::apic::Clocks;
use crate::arch::x86_64::paging::AddressSpace;
use crate::arch::x86_64::{ap, apic, current_cpu, gdt, idt, paging, read_tsc, ring3};
use crate::percpu::{MAX_CPUS, PerCpu};

/// What one core has been asked for, and what it has answered.
///
/// The slot belongs to the core it names. The boot processor writes another
/// core's slot to ask for something; that core writes its own to answer.
static MAILBOX: PerCpu<u64> = PerCpu::new(NOT_STARTED);

/// The core has never run kernel code. Also every slot before bring-up, which
/// is what makes "which cores exist" answerable without a second structure.
const NOT_STARTED: u64 = 0;

/// The core is up, its descriptor tables are its own, and it is waiting.
const READY: u64 = 1;

/// The core is to run the process prepared for it.
const RUN: u64 = 2;

/// It has, and everything it observed is in its own shards.
const DONE: u64 = 3;

/// The core got as far as kernel code and could not finish bringing itself up.
const FAILED: u64 = 4;

/// The page a core is being asked to forget.
static SHOOT_PAGE: PerCpu<u64> = PerCpu::new(0);

/// Which request that is. Counting rather than flagging, so that two
/// shootdowns of the same page are two events: a flag would let the second be
/// answered by the acknowledgement of the first.
static SHOOT_SEQ: PerCpu<u64> = PerCpu::new(0);

/// The last request this core has finished. Written by its owner, read by
/// whoever asked.
static SHOOT_ACK: PerCpu<u64> = PerCpu::new(0);

/// Everything a core needs to be told before it can find anything out for
/// itself.
///
/// Filled by the boot processor in the arriving core's slot — which is the case
/// [`PerCpu::at`] exists for and says so — and read once, by that core, before
/// it has any other way to reach any of it.
#[derive(Clone, Copy)]
struct Handoff {
    /// The address space to run in. Already in `CR3` when the core arrives, so
    /// this is what it switches *back* to after a process.
    kernel_root: u64,
    /// The local APIC register window the boot processor mapped.
    apic: u64,
    /// The clocks it measured, which this core adopts rather than re-measures.
    clocks: Clocks,
}

/// One per core, and only ever read by the core it belongs to.
static HANDOFF: PerCpu<Handoff> = PerCpu::new(Handoff {
    kernel_root: 0,
    apic: 0,
    clocks: Clocks { tsc_khz: 0, apic_khz: 0, backend: apic::Backend::OneShot },
});

/// How long the boot processor waits for a core it has started, in
/// microseconds.
///
/// A hundred milliseconds. It is not a timeout on a slow core — a core that has
/// arrived does so in microseconds — it is the answer to a core that is not
/// there at all, and under an emulator every one of those numbers is two orders
/// of magnitude out. Generous on purpose, for the reason `apic::start`'s
/// give-up bound is.
const ARRIVAL_MICROS: u64 = 100_000;

/// How long a core waits for another to acknowledge a shootdown.
///
/// Ten milliseconds, and this one *is* tight, because the thing being waited
/// for is an interrupt handler that does two instructions. A core that has not
/// answered in ten milliseconds with interrupts enabled is a core that is not
/// going to.
const SHOOTDOWN_MICROS: u64 = 10_000;

/// Why bringing the machine's other cores up did not finish.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StartError {
    /// The trampoline and the linker script disagree with this kernel about
    /// something the boot cannot proceed without.
    Geometry(&'static str),
    /// The on-ramp page could not be mapped.
    OnRamp(paging::BuildError),
    /// A core was sent the startup sequence and never reached kernel code.
    NeverArrived(usize),
    /// A core reached kernel code and could not finish bringing itself up.
    /// What went wrong is on that core, and it could not print it.
    ArrivedBroken(usize),
}

impl StartError {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Geometry(why) => why,
            Self::OnRamp(inner) => inner.message(),
            Self::NeverArrived(_) => "a core was started and never reached kernel code",
            Self::ArrivedBroken(_) => "a core reached kernel code and could not bring itself up",
        }
    }

    /// Which core, where the answer is a core.
    #[must_use]
    pub const fn core(self) -> Option<usize> {
        match self {
            Self::NeverArrived(cpu) | Self::ArrivedBroken(cpu) => Some(cpu),
            _ => None,
        }
    }
}

/// How many cores this kernel is running on.
///
/// Counted rather than assumed, and reported so the boot log says it.
#[must_use]
pub fn started() -> usize {
    (0..MAX_CPUS).filter(|&cpu| peek(cpu) != NOT_STARTED && peek(cpu) != FAILED).count()
}

/// The lowest-numbered running core that is not this one.
///
/// Where a process goes. It is a policy of one line because there is no
/// scheduler to have a better one: with one process and one core to spare,
/// "the other core" is the whole of the placement decision.
///
/// *Reversal:* E1's scheduler, at which point this is replaced by a run queue
/// and the question stops being which core is free and becomes which core the
/// reservation says.
#[must_use]
pub fn first_worker() -> usize {
    let me = current_cpu();
    (0..MAX_CPUS).find(|&cpu| cpu != me && peek(cpu) == READY).unwrap_or(me)
}

/// How many logical processors this package has, as the processor reports it.
///
/// Leaf 0x0B is the extended topology enumeration, and its second level counts
/// every logical processor in the package. It is asked rather than an ACPI
/// table parsed, and that is a deliberately smaller answer than the right one:
/// it is one package's count, so a two-socket machine is undercounted, and it
/// assumes the APIC ids are dense and small — which is the assumption
/// [`current_cpu`] already makes by using the initial APIC id as a shard index.
/// Making one of those two better without the other buys nothing.
///
/// *Reversal:* the multiprocessor table this really wants is the ACPI MADT,
/// which lists every local APIC on the machine with its id and whether firmware
/// has enabled it. It arrives with E5 naming a real machine, together with
/// finding the root pointer — multiboot 1 does not hand one over, so it has to
/// be searched for in the BIOS area — and it retires this function and the
/// dense-id assumption in the same change.
#[must_use]
pub fn logical_processors() -> usize {
    // SAFETY: `cpuid` is unprivileged and has no memory effect. Leaf zero
    // reports the highest leaf this processor answers, which is what makes
    // asking for 0x0B a question rather than a guess.
    let (highest, _, _, _) = unsafe { crate::arch::x86_64::cpuid_subleaf(0, 0) };
    if highest >= 0x0B {
        // Subleaf 1 is the core level; its `ebx` is the number of logical
        // processors at and below it, which for one package is all of them.
        // SAFETY: as above, and the leaf is one this processor answers.
        let (_, ebx, _, _) = unsafe { crate::arch::x86_64::cpuid_subleaf(0x0B, 1) };
        if ebx != 0 {
            return ebx as usize;
        }
    }

    // The older answer: the maximum addressable ids per package, which is a
    // power of two that is at least the count and may be more. A core that is
    // not really there simply never arrives, which is what the give-up bound in
    // `start` is for.
    // SAFETY: leaf 1 exists on every processor that has `cpuid` at all.
    let (_, ebx, _, _) = unsafe { crate::arch::x86_64::cpuid_subleaf(1, 0) };
    (((ebx >> 16) & 0xFF) as usize).max(1)
}

/// What bringing the machine up found.
#[derive(Clone, Copy)]
pub struct Started {
    /// How many cores are running kernel code, this one included.
    pub cores: usize,
    /// How many the processor says the package has.
    ///
    /// Reported separately from `cores` because they can differ, and the
    /// difference is the one thing about this that would otherwise be silent: a
    /// machine with more cores than [`MAX_CPUS`] has the rest left asleep. That
    /// is a real limitation and not a failure — every started core has a shard,
    /// which is what makes the kernel correct on it — but a boot log that said
    /// only "2 cores" on a sixteen-core machine would be hiding it.
    pub present: usize,
}

/// Bring up every core this machine has, and leave each of them waiting.
///
/// Serial on purpose: one core is started and waited for before the next is
/// touched. The trampoline is one page with one stack pointer in it, so two
/// cores starting at once would be two cores on one stack — and a bring-up that
/// is a hundred microseconds slower is a trade nobody has to think about again.
///
/// Returns how many cores are running, including this one.
///
/// # Errors
///
/// [`StartError`]. Fatal: a kernel that cannot start the cores it can see is
/// running on a machine it has misread, and the next thing it would do is hand
/// a process to a core that is not there.
///
/// # Safety
///
/// Call once, on the boot processor, with the kernel's address space active and
/// `frames` rebound onto its direct map, after [`apic::init`] and
/// [`apic::calibrate`] on this core, and with interrupts disabled.
pub unsafe fn start(
    frames: &mut crate::mem::FrameAllocator,
    space: &AddressSpace,
    clocks: Clocks,
) -> Result<Started, StartError> {
    ap::self_test().map_err(StartError::Geometry)?;

    // This core, in the same terms as the others, so that "which cores exist"
    // has one answer and the shootdown loop does not need a special case for
    // the core it happens to be running on.
    post(current_cpu(), READY);

    let present = logical_processors();
    // Every core past the last shard is left asleep rather than started. It is
    // not a refusal, because a kernel that ran on eight of a machine's sixteen
    // cores is correct on that machine — `current_cpu` uses the initial APIC id
    // as a shard index, so a core past the end has nowhere to keep its state and
    // starting it would panic in `PerCpu::at`. The number is carried out of here
    // so the boot log can say what was left.
    let count = present.min(MAX_CPUS);
    if count <= 1 {
        return Ok(Started { cores: 1, present });
    }

    // SAFETY: the caller's guarantee that the space is active and `frames` is
    // rebound. The page is `ap::TRAMPOLINE_PHYS`, which nothing else uses.
    let ramp = unsafe { paging::map_on_ramp(frames, space, ap::TRAMPOLINE_PHYS) }
        .map_err(StartError::OnRamp)?;

    // SAFETY: the direct map is live, the page is the one just mapped, and no
    // core is executing it — none has been started.
    unsafe { ap::install(space.root(), arrive as *const () as u64) };

    let window = apic::window();
    for cpu in 1..count {
        let Some(stack_top) = ap::stack_top(cpu) else {
            // Unreachable: `count` is clamped to `MAX_CPUS` above and
            // `self_test` has already checked that the linker reserved a block
            // for every core below it. Refused rather than skipped, because a
            // core started onto a stack that does not exist is not something to
            // carry on past.
            return Err(StartError::Geometry(
                "a core was to be started with no stack reserved for it",
            ));
        };

        let slot = HANDOFF.at(cpu);
        // SAFETY: the slot of a core that has not started, so nothing can be
        // reading it. The `Release` store below is what publishes this write to
        // the core that will.
        unsafe { slot.write(Handoff { kernel_root: space.root(), apic: window, clocks }) };

        // SAFETY: `window` is this core's mapped register window, the rate was
        // measured on this machine, the trampoline is installed, and `cpu` has
        // not been started.
        unsafe { ap::wake(window, clocks.tsc_khz, cpu, stack_top) };

        match wait_for(cpu, clocks.tsc_khz, ARRIVAL_MICROS) {
            READY => {}
            FAILED => return Err(StartError::ArrivedBroken(cpu)),
            _ => return Err(StartError::NeverArrived(cpu)),
        }
    }

    // The on-ramp has done its job, and every core that used it is executing in
    // the higher half. Withdrawing it is the first shootdown this kernel ever
    // performs, and it is a real one: the arriving cores walked those tables, so
    // the entries are in their translation buffers.
    // SAFETY: as `map_on_ramp`, `ramp` came from it against this same space, and
    // every core that used it has reported `READY` — which it does only after
    // it is executing kernel code at a kernel address.
    unsafe { paging::unmap_on_ramp(frames, space, &ramp) };
    // The error is dropped on purpose and only here: a core that cannot
    // acknowledge the withdrawal of a page it is no longer executing is a
    // problem, and it is a smaller problem than refusing to boot over it. Every
    // other caller of `shootdown` treats a failure as fatal.
    // SAFETY: the entry has been cleared and this core's translation
    // invalidated by `unmap_on_ramp`, and every core being told has interrupts
    // enabled — `arrive` turns them on before it reports ready.
    let _ = unsafe { shootdown(ramp.page()) };

    Ok(Started { cores: started(), present })
}

/// Where a started core arrives.
///
/// Reached by an absolute jump out of the trampoline, so nothing calls it and
/// it never returns. Everything it does is per core and in the order the rest of
/// the kernel established at boot: descriptor tables before anything that could
/// fault, the local APIC before anything that could be delivered, the
/// system-call entry before anything could make one.
///
/// It prints nothing. Two cores writing to one serial port produce interleaved
/// bytes, and the boot log is a fixture — so what this core finds out, it
/// records in its own shards, and the boot processor says it afterwards.
extern "C" fn arrive() -> ! {
    let me = current_cpu();
    if me >= MAX_CPUS {
        // Nothing can be recorded — this core has no slot to record it in.
        crate::arch::x86_64::halt_forever();
    }

    // SAFETY: this core's slot, written by the boot processor before it started
    // this core, and published by the `Release` store it is waiting on.
    let handoff = unsafe { HANDOFF.mine().read() };

    // SAFETY: once on this core, before interrupts are enabled on it, and the
    // descriptors it installs describe the flat address space it is already
    // running in.
    unsafe { gdt::init() };
    // SAFETY: as above, and after the code selector its gates name exists.
    unsafe { idt::init() };

    // SAFETY: once on this core, interrupts disabled, after `idt::init` on it,
    // and `handoff.apic` is the window the boot processor mapped — which is the
    // whole list `adopt` asks for.
    if unsafe { apic::adopt(handoff.apic, handoff.clocks) }.is_err() {
        post(me, FAILED);
        crate::arch::x86_64::halt_forever();
    }

    // SAFETY: once on this core, after `gdt::init` on it, and before anything
    // enters ring 3 on it.
    unsafe { ring3::init() };

    // Last, and the ordering is the point: everything above has to have
    // happened before another core is allowed to believe it has. The `Release`
    // is what makes that true rather than merely likely.
    post(me, READY);

    park(me, handoff.kernel_root)
}

/// Wait for work, do it, and wait again.
///
/// Interrupts are enabled here and only here on a started core, because a core
/// that cannot take an interrupt cannot answer a shootdown — and a shootdown
/// that is not answered is a stale translation nobody will ever notice.
fn park(me: usize, kernel_root: u64) -> ! {
    // SAFETY: every vector the local APIC can deliver to this core has a gate:
    // the thirty-two exceptions, the timer, the shootdown and the spurious one.
    // The legacy controllers were masked by the boot processor at bring-up.
    unsafe { core::arch::asm!("sti", options(nostack)) };

    loop {
        if peek(me) == RUN {
            // The timer this core is about to arm wants interrupts disabled on
            // entry, and turns them back on itself.
            // SAFETY: disabling delivery on this core, which nothing else is
            // depending on between here and `execute` enabling it again.
            unsafe { core::arch::asm!("cli", options(nostack)) };
            // SAFETY: this core is up, its capability table and process state
            // were filled by the boot processor before the job was posted, and
            // `kernel_root` is the address space to return to.
            unsafe { crate::process::execute(kernel_root) };
            // SAFETY: as above.
            unsafe { core::arch::asm!("sti", options(nostack)) };
            post(me, DONE);
        }
        core::hint::spin_loop();
    }
}

/// Ask a core to run the process prepared for it, and wait until it has.
///
/// # Errors
///
/// The core it was asked of, if it never answered.
///
/// # Safety
///
/// `cpu` must be a core reporting [`READY`], everything
/// [`crate::process::execute`] depends on must already be in that core's
/// shards, and this core must have interrupts enabled — the running core may
/// need a shootdown answered, and a core with interrupts disabled cannot answer
/// one.
pub unsafe fn run_on(cpu: usize, kernel_root: u64, tsc_khz: u64, micros: u64) -> Result<(), usize> {
    if cpu == current_cpu() {
        // A machine with one core, and it is this one. There is no mailbox
        // exchange to make, because the core that would post the job is the
        // core that would run it — so it runs it, and the difference between
        // the two shapes is confined to this branch instead of spreading into
        // the caller.
        //
        // The visible consequence is in the boot log rather than in the code:
        // the process's timer window and the milestone's own no longer overlap,
        // because one core cannot hold two. `main::timed_window` says so where
        // it opens the second one.
        // SAFETY: the caller's guarantee that this core was prepared, plus
        // interrupts disabled — which `execute` requires and which the caller
        // owes in this branch specifically, because there is no `park` here to
        // have turned them off.
        unsafe { crate::process::execute(kernel_root) };
        return Ok(());
    }

    post(cpu, RUN);
    match wait_for(cpu, tsc_khz, micros) {
        DONE => {
            // Back to waiting, so a second job is a second `RUN` rather than a
            // word that already says what the next answer would be.
            post(cpu, READY);
            Ok(())
        }
        _ => Err(cpu),
    }
}

/// Tell every other running core to forget one page.
///
/// Returns once each of them has acknowledged, which is what makes the caller
/// entitled to believe the translation is gone everywhere rather than only
/// here.
///
/// # One initiator at a time
///
/// This assumes it. The request words in a target's slot are written by whoever
/// is asking, and two cores asking the same core at once would overwrite each
/// other's page and could both be satisfied by one acknowledgement. Nothing can
/// do that today — one process runs at a time, on one core, and it is the only
/// thing that revokes — but the assumption is here rather than implied, because
/// the day a second core starts revoking is the day this becomes a lost
/// invalidation with no symptom.
///
/// The fix when that day comes is not a lock. It is a queue of pending
/// invalidations per core, which is a structure rather than a word, and RFC
/// 0016 names it as the thing that would reverse the rule this file bends.
///
/// # Errors
///
/// The core that did not answer. A caller cannot recover from this: an
/// unacknowledged shootdown is a core still reading memory through a mapping
/// that has been taken away, and there is no smaller thing to report.
///
/// # Safety
///
/// The page table entry must already have been cleared and the calling core's
/// own translation invalidated — [`paging::unmap_user_live`] does both — and
/// every core being told must have interrupts enabled.
pub unsafe fn shootdown(page: u64) -> Result<(), usize> {
    let me = current_cpu();
    // Read here rather than passed in, because this is reached from the
    // system-call path, where the only thing the caller has is a handle. Every
    // core has the same measurement — one core measures and the rest adopt it —
    // so asking this core is asking the machine.
    let tsc_khz = apic::tsc_khz();
    for cpu in 0..MAX_CPUS {
        if cpu == me || peek(cpu) == NOT_STARTED || peek(cpu) == FAILED {
            continue;
        }

        let seq = load(&SHOOT_SEQ, cpu, Ordering::Relaxed) + 1;
        store(&SHOOT_PAGE, cpu, page, Ordering::Relaxed);
        // The one store that matters. It publishes two things the target has to
        // see: the page above, and the not-present entry the caller wrote
        // before calling. Everything before this store, in program order, is
        // visible to a core that reads this word with `Acquire` — which is what
        // `answer` does, and why `Relaxed` here would be a core invalidating a
        // translation and then walking a table that still has the old entry in
        // it.
        store(&SHOOT_SEQ, cpu, seq, Ordering::Release);

        // SAFETY: this core's mapped register window, a core that is running,
        // and a vector `idt::init` installs on every core.
        unsafe { ap::send(apic::window(), cpu, apic::SHOOTDOWN_VECTOR) };

        let deadline = read_tsc().saturating_add(tsc_khz.saturating_mul(SHOOTDOWN_MICROS) / 1_000);
        loop {
            if load(&SHOOT_ACK, cpu, Ordering::Acquire) >= seq {
                break;
            }
            if read_tsc() > deadline {
                return Err(cpu);
            }
            core::hint::spin_loop();
        }
    }
    Ok(())
}

/// Answer a shootdown: forget the page, then say so.
///
/// # Safety
///
/// Call from the shootdown vector's own gate, on the core it was delivered to,
/// with interrupts disabled by that gate.
pub(crate) unsafe fn answer() {
    let me = current_cpu();

    // `Acquire`, against the `Release` in `shootdown`. It is what makes the
    // page below — and the page table entry the initiator cleared before it —
    // visible to this core. The order of these two reads is the order the
    // ordering is about: the sequence number first, because it is the one that
    // publishes the other.
    let seq = load(&SHOOT_SEQ, me, Ordering::Acquire);
    let page = load(&SHOOT_PAGE, me, Ordering::Relaxed);

    // SAFETY: invalidating a page is architecturally valid at ring 0 for any
    // address, mapped or not — which matters here, because a core that never
    // had this translation is told about it anyway. Telling everybody is the
    // correct answer to "who might have cached this": the alternative is
    // tracking which cores have loaded which address space, and a tracker that
    // is wrong is a stale translation with an explanation attached.
    unsafe {
        core::arch::asm!("invlpg [{}]", in(reg) page, options(nostack, preserves_flags));
    }

    // `Release`, so that a core seeing this acknowledgement is entitled to
    // believe the invalidation above has happened rather than merely been
    // issued.
    store(&SHOOT_ACK, me, seq, Ordering::Release);

    // Last, after the acknowledgement rather than before it, because until this
    // write the local APIC will not deliver another shootdown to this core —
    // and a second request that arrived before the first was answered would be
    // answered by the same sequence number.
    // SAFETY: this core, inside the handler for the interrupt being
    // acknowledged.
    unsafe { apic::end_of_interrupt() };
}

/// Wait for a core's mailbox to say something other than what it says now.
///
/// Returns whatever it ended up saying, including the value it started at if
/// the bound was reached — the caller decides what that means.
fn wait_for(cpu: usize, tsc_khz: u64, micros: u64) -> u64 {
    let deadline = read_tsc().saturating_add(tsc_khz.saturating_mul(micros) / 1_000);
    loop {
        let state = peek(cpu);
        if state == READY || state == DONE || state == FAILED {
            return state;
        }
        if read_tsc() > deadline {
            return state;
        }
        core::hint::spin_loop();
    }
}

/// Put a word in a core's mailbox.
fn post(cpu: usize, value: u64) {
    store(&MAILBOX, cpu, value, Ordering::Release);
}

/// Read a core's mailbox.
fn peek(cpu: usize) -> u64 {
    load(&MAILBOX, cpu, Ordering::Acquire)
}

/// Read one core's word out of a shard, atomically.
///
/// The `PerCpu` is what keeps this legal at all — the slot is a word of its
/// own, so nothing else shares its cache line's meaning — and the atomic is what
/// makes it legal for the *other* core to be looking at it. A volatile read
/// would be the wrong tool: volatile says "do not elide this access" and says
/// nothing about ordering, and ordering is the entire content of both protocols
/// in this file.
fn load(shard: &'static PerCpu<u64>, cpu: usize, order: Ordering) -> u64 {
    let slot = shard.at(cpu);
    // SAFETY: `at` returns a pointer to one aligned `u64` inside a `'static`
    // shard, which is a valid `AtomicU64` for as long as every access to it goes
    // through one — and in this file every access does. The reference does not
    // outlive the call.
    unsafe { AtomicU64::from_ptr(slot) }.load(order)
}

/// Write one core's word into a shard, atomically. As [`load`].
fn store(shard: &'static PerCpu<u64>, cpu: usize, value: u64, order: Ordering) {
    let slot = shard.at(cpu);
    // SAFETY: as [`load`].
    unsafe { AtomicU64::from_ptr(slot) }.store(value, order);
}
