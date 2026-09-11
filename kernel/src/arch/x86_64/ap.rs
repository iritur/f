// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Starting a core that is not the one the firmware started.
//!
//! # What has to be true before a second core can run Rust
//!
//! An application processor comes out of reset in *real mode*. Not protected
//! mode, not long mode: sixteen bits, no paging, and an instruction pointer the
//! interrupt command register supplies as one byte — the page number of where
//! to begin. That is the whole of the interface, and everything in this file
//! exists to get from there to a core executing the same kernel the boot
//! processor is executing.
//!
//! So the sequence is the boot stub's, again, in miniature: real mode to
//! protected mode to long mode, and then a jump to the higher half. It is
//! written here rather than reused from `boot.rs` because the two differ in the
//! one way that matters — the stub builds its own transitional page tables and
//! this does not. By the time a core is started the kernel's real address space
//! exists, so the trampoline loads *that* into `CR3` and the arriving core is
//! in the finished address space from its first paged instruction.
//!
//! # The on-ramp, and why one page of the kernel's lower half is mapped
//!
//! Enabling paging does not change where the instruction pointer is. The
//! instruction after `mov %eax, %cr0` executes at the trampoline's own low
//! physical address, and if that address is not mapped in the table just
//! loaded, the core takes a page fault with no descriptor table, no handler and
//! nowhere to report it — which is a triple fault and a silent reset.
//!
//! So exactly one page — [`TRAMPOLINE_PHYS`], identity mapped, present,
//! readable and executable and neither writable nor reachable from ring 3 —
//! is added to the kernel's address space for the length of bring-up and
//! withdrawn afterwards. It is in the lower half, which is otherwise empty in
//! the kernel's own space and is copied into no process's: [`super::paging`]
//! copies the upper half and only the upper half.
//!
//! The descriptors in that page carry the accessed bit already set. That is not
//! decoration: the processor *writes* the accessed bit into a descriptor when a
//! selector naming it is loaded, and the on-ramp page is deliberately not
//! writable — so a descriptor that arrived with the bit clear would fault on
//! the far jump that uses it. Setting it here is what lets the page stay
//! read-only.
//!
//! # Where a started core's stacks come from
//!
//! The linker script, and `kernel/linker.ld` argues why at the point it
//! reserves them. The short version: a stack needs a guard page, a guard page
//! is a hole in the kernel window, and the only code that can leave a hole
//! there is the mapper that builds the kernel window — which has finished long
//! before a core is started.

use core::arch::global_asm;

use super::paging::PHYS_OFFSET;
use crate::mem::FRAME_SIZE;
use crate::percpu::MAX_CPUS;

/// Where the trampoline is placed, physically.
///
/// A startup interrupt carries a page number and nothing else, so this has to
/// be page-aligned and below one mebibyte. Thirty-two kibibytes is inside the
/// region every machine reports as usable, above the real-mode interrupt table
/// and the BIOS data area, and below everything the loader touches — and the
/// frame allocator never hands out anything under
/// [`LOW_MEMORY_LIMIT`](crate::mem::LOW_MEMORY_LIMIT), so nothing else in this
/// kernel can be given it.
pub const TRAMPOLINE_PHYS: u64 = 0x8000;

/// Where in the trampoline page the descriptor table sits.
const GDT_OFFSET: u64 = 0xE00;

/// Where in the trampoline page the pointer to that table sits.
const GDT_POINTER_OFFSET: u64 = 0xE20;

/// Where in the trampoline page the parameters the boot processor fills sit.
///
/// The offsets within the block are duplicated in the assembly below as literal
/// addresses, because sixteen-bit code has no way to be told an address. They
/// are checked against each other by [`self_test`].
const PARAMS_OFFSET: u64 = 0xF00;

/// `CR3` the arriving core loads: the kernel's own address space.
const PARAM_CR3: u64 = 0x00;
/// `CR4` the arriving core loads, copied from the boot processor's.
const PARAM_CR4: u64 = 0x08;
/// `IA32_EFER` the arriving core loads, copied from the boot processor's.
const PARAM_EFER: u64 = 0x10;
/// The stack pointer it starts on.
const PARAM_RSP: u64 = 0x18;
/// The sixty-four-bit address it jumps to once it is in the higher half.
const PARAM_ENTRY: u64 = 0x20;

/// How far the arriving core has got, written by the core itself.
///
/// # Why a byte in a page rather than a mailbox word
///
/// [`crate::smp`]'s mailbox is how a core reports that it *arrived*, and it is
/// written in Rust, from `arrive`, after the trampoline has finished. Nothing
/// reports the trampoline, and the trampoline is where a core that never
/// arrives has died — with no descriptor table, no handler and nothing to
/// report with, which is a triple fault and, on a hypervisor, the whole virtual
/// machine stopping. A post-mortem line printed by the boot processor never
/// runs on that machine, because the boot processor is gone too.
///
/// So the core writes a number here as it passes each point, and the boot
/// processor *watches* while it waits — see `smp::start`. The evidence reaches
/// the serial port before the fault rather than after it, which is the only
/// ordering that survives the failure it exists to describe.
///
/// One byte, and the values are [`STAGE_*`](STAGE_REAL_MODE). Written with a
/// plain store because the only two cores that touch it are the one writing its
/// own progress and the one reading it, a byte is atomic on this architecture,
/// and nothing is ordered against it — a stage read late is a stage printed
/// late, which costs a reader nothing.
const PARAM_STAGE: u64 = 0x28;

/// One past the last byte of the parameter block.
const PARAMS_END: u64 = PARAM_STAGE + 8;

/// The core is executing the trampoline with a zero segment base.
pub const STAGE_REAL_MODE: u8 = 1;
/// It has loaded the descriptor table the boot processor left for it.
pub const STAGE_DESCRIPTORS: u8 = 2;
/// It is in thirty-two-bit protected mode.
pub const STAGE_PROTECTED: u8 = 3;
/// It has loaded `CR4`.
pub const STAGE_CR4: u8 = 4;
/// It has loaded `CR3` — the kernel's address space, paging still off.
pub const STAGE_CR3: u8 = 5;
/// It has written `IA32_EFER`. **The last stage before paging**, and the one a
/// core that dies writing a read-only bit stops at. [`EFER_TO_ARRIVE_WITH`].
const STAGE_EFER: u8 = 6;
/// It is in sixty-four-bit mode, in the kernel's address space.
const STAGE_LONG_MODE: u8 = 7;
/// It is about to leave the on-ramp for `arrive`.
const STAGE_LEAVING: u8 = 8;
/// Reached `arrive`: its shard index is its own and within `MAX_CPUS`.
pub const STAGE_ARRIVED: u8 = 9;
/// Its descriptor tables are its own.
pub const STAGE_TABLES: u8 = 10;
/// Its local APIC is adopted.
pub const STAGE_APIC: u8 = 11;
/// Its system-call entry is installed. The next thing it does is report ready.
pub const STAGE_RING3: u8 = 12;

/// What each stage is called, for the boot processor's log.
#[must_use]
pub fn stage_name(stage: u8) -> &'static str {
    match stage {
        STAGE_REAL_MODE => "executing the trampoline",
        STAGE_DESCRIPTORS => "descriptor table loaded",
        STAGE_PROTECTED => "thirty-two-bit protected mode",
        STAGE_CR4 => "cr4 loaded",
        STAGE_CR3 => "cr3 loaded, paging still off",
        STAGE_EFER => "efer written",
        STAGE_LONG_MODE => "sixty-four-bit mode",
        STAGE_LEAVING => "leaving the on-ramp",
        STAGE_ARRIVED => "in the kernel, shard index taken",
        STAGE_TABLES => "its own descriptor tables",
        STAGE_APIC => "local apic adopted",
        STAGE_RING3 => "system-call entry installed",
        // Not "unknown": zero is what `wake` writes before it starts a core,
        // so a core still at zero is a core that never executed an instruction
        // of the trampoline. That is a different finding from dying inside it,
        // and it is the one a machine with fewer cores than it reports produces.
        0 => "nothing; it never executed",
        _ => "not a stage this build names",
    }
}

/// Where the arriving core writes [`PARAM_STAGE`], as this core can reach it.
///
/// The direct map, because the trampoline page is mapped read-only into the
/// kernel's own space — deliberately, and the module comment says why — so the
/// physical address is reachable for writing only through the map that covers
/// all of memory.
#[must_use]
pub const fn stage_address() -> u64 {
    PHYS_OFFSET + TRAMPOLINE_PHYS + PARAMS_OFFSET + PARAM_STAGE
}

/// How far the core being started has got. See [`PARAM_STAGE`].
///
/// # Safety
///
/// The direct map must be live.
#[must_use]
pub unsafe fn stage() -> u8 {
    let at = stage_address() as *const u8;
    // SAFETY: the caller's guarantee. One byte inside the trampoline page,
    // reachable through the direct map, written by the core being started.
    unsafe { at.read_volatile() }
}

/// Record that this core has reached `stage`. Called from `arrive`, which is
/// past the point where the trampoline can write it.
///
/// # Safety
///
/// The direct map must be live, and this must be the core being started.
pub unsafe fn record(stage: u8) {
    let at = stage_address() as *mut u8;
    // SAFETY: the caller's guarantee. As [`stage`], and the only writer.
    unsafe { at.write_volatile(stage) };
}

/// The four descriptors the trampoline needs, with the accessed bit set.
///
/// Null, thirty-two-bit code, data, sixty-four-bit code — selectors 0x00, 0x08,
/// 0x10 and 0x18, in the order the assembly below names them. The boot stub's
/// own table cannot be used: its code descriptor is sixty-four-bit, and the
/// middle step of this walk is thirty-two-bit protected mode.
const TRAMPOLINE_GDT: [u64; 4] = [
    0,
    // Present, ring 0, code, execute/read, accessed. 32-bit, granularity 4 KiB.
    0x00CF_9B00_0000_FFFF,
    // Present, ring 0, data, read/write, accessed.
    0x00CF_9300_0000_FFFF,
    // Present, ring 0, code, execute/read, accessed. Long mode.
    0x00AF_9B00_0000_FFFF,
];

unsafe extern "C" {
    /// First byte of the trampoline.
    static ap_trampoline_start: u8;
    /// One past its last byte.
    static ap_trampoline_end: u8;
    /// The bottom of the first application processor's stack block.
    static __ap_stacks_start: u8;
    /// One past the last byte of the last one.
    static __ap_stacks_end: u8;
}

/// One unmapped page below each stack.
///
/// These four numbers mirror `kernel/linker.ld`. They are not read from it,
/// because the values are small absolute symbols and the kernel code model
/// addresses a symbol relative to the instruction pointer — which cannot reach
/// the number seven. [`self_test`] closes the gap the other way: it checks that
/// the geometry these describe covers exactly the extent the linker reserved.
const AP_GUARD: u64 = 4 * 1024;

/// How much stack a started core gets.
const AP_STACK: u64 = 32 * 1024;

/// How much stack its double-fault handler switches to.
const AP_FAULT_STACK: u64 = 16 * 1024;

/// One core's whole block: guard, stack, guard, fault stack.
const AP_STRIDE: u64 = AP_GUARD + AP_STACK + AP_GUARD + AP_FAULT_STACK;

/// Where the stack block for `cpu` begins, or `None` for a core with none —
/// which is the boot processor, whose stacks are the linker script's other
/// pair, and any index this kernel does not shard for.
fn block(cpu: usize) -> Option<u64> {
    if cpu == 0 || cpu >= MAX_CPUS {
        return None;
    }
    let start = (&raw const __ap_stacks_start) as u64;
    Some(start + (cpu as u64 - 1) * AP_STRIDE)
}

/// The stack a started core runs on: the address one past its top.
#[must_use]
pub fn stack_top(cpu: usize) -> Option<u64> {
    Some(block(cpu)? + AP_GUARD + AP_STACK)
}

/// The stack its double-fault handler switches to.
#[must_use]
pub fn fault_stack_top(cpu: usize) -> Option<u64> {
    Some(block(cpu)? + AP_GUARD + AP_STACK + AP_GUARD + AP_FAULT_STACK)
}

/// Is this page one of the holes below an application processor's stacks?
///
/// Asked by [`super::paging::build`] for every page of the kernel window, which
/// is why it is a test rather than a list: the list would be fourteen entries
/// on a machine with one core.
#[must_use]
pub fn is_stack_guard(virt: u64) -> bool {
    let start = (&raw const __ap_stacks_start) as u64;
    let end = (&raw const __ap_stacks_end) as u64;
    if virt < start || virt >= end {
        return false;
    }
    let within = (virt - start) % AP_STRIDE;
    let second = AP_GUARD + AP_STACK;
    within < AP_GUARD || (second..second + AP_GUARD).contains(&within)
}

/// Check that this file and the linker script still agree.
///
/// One property, and it is the one that fails silently: the extent the linker
/// reserved has to be exactly the extent the geometry above describes. If it is
/// not, [`stack_top`] hands some core an address inside its neighbour's block,
/// or past the end of the section entirely — and the symptom is a core that
/// corrupts another core's stack, which is the least debuggable failure this
/// kernel has.
///
/// What it does not catch is [`AP_STACK`] and [`AP_FAULT_STACK`] being swapped,
/// because the total is the same. That would leave both stacks guarded and both
/// mapped, so it is a sizing mistake rather than a soundness one.
///
/// # Errors
///
/// A sentence for the boot log.
pub fn self_test() -> Result<(), &'static str> {
    let start = (&raw const __ap_stacks_start) as u64;
    let end = (&raw const __ap_stacks_end) as u64;
    if end.saturating_sub(start) != (MAX_CPUS as u64 - 1) * AP_STRIDE {
        return Err("the linker script and ap.rs disagree about stack geometry");
    }
    if AP_GUARD != FRAME_SIZE {
        return Err("a stack guard that is not one page is a guard paging::build cannot skip");
    }
    // The whole block the assembly addresses by literal must fit in the page a
    // startup interrupt can name.
    if PARAMS_OFFSET + PARAMS_END > FRAME_SIZE || GDT_POINTER_OFFSET + 10 > PARAMS_OFFSET {
        return Err("the trampoline's fixed offsets do not fit in one page");
    }
    if program().len() as u64 > GDT_OFFSET {
        return Err("the trampoline is longer than the space reserved before its descriptors");
    }
    // The two addresses the assembly writes its progress to are literals there —
    // it is copied to one address and assembled at another, so it can be neither
    // `rip`-relative nor link-time absolute — and this is the check that keeps
    // them equal to the constants above. Getting either wrong is a store into
    // somebody else's page, which is the class of bug that does not announce
    // itself.
    if TRAMPOLINE_PHYS + PARAMS_OFFSET + PARAM_STAGE != 0x8F28 {
        return Err("the trampoline's physical stage literal and the constants disagree");
    }
    if stage_address() != 0xffff_8000_0000_8f28 {
        return Err("the trampoline's direct-map stage literal and the constants disagree");
    }
    Ok(())
}

/// The trampoline, as bytes to be copied into low memory.
#[must_use]
pub fn program() -> &'static [u8] {
    let start = (&raw const ap_trampoline_start).cast::<u8>();
    let end = (&raw const ap_trampoline_end).cast::<u8>();
    let len = (end as usize) - (start as usize);
    // SAFETY: both symbols are emitted by the assembly below, in that order, in
    // one section, so the region between them is exactly the trampoline. It is
    // in `.rodata` and immutable for the life of the kernel, which is what
    // makes a `'static` shared slice of it sound.
    unsafe { core::slice::from_raw_parts(start, len) }
}

/// Write the trampoline, its descriptor table and the parameters every arriving
/// core reads, into [`TRAMPOLINE_PHYS`].
///
/// `cr3` is the address space an arriving core lands in — the kernel's — and
/// `entry` is where it goes once it is executing in the higher half.
///
/// # Safety
///
/// The direct map must be live, [`TRAMPOLINE_PHYS`] must be a page no other
/// part of this kernel is using, and no core may be executing the trampoline:
/// this overwrites it.
pub unsafe fn install(cr3: u64, entry: u64) {
    let page = (PHYS_OFFSET + TRAMPOLINE_PHYS) as *mut u8;
    let program = program();

    // SAFETY: the caller's guarantee that the page is reachable through the
    // direct map and owned by nobody. The length is checked against the space
    // before the descriptor table by `self_test`, which runs before this.
    unsafe { core::ptr::copy_nonoverlapping(program.as_ptr(), page, program.len()) };

    for (index, descriptor) in TRAMPOLINE_GDT.iter().copied().enumerate() {
        let at = page.wrapping_add(GDT_OFFSET as usize).cast::<u64>().wrapping_add(index);
        // SAFETY: as above; the offset is inside the page by `self_test`.
        unsafe { at.write_unaligned(descriptor) };
    }

    // The six bytes `lgdt` reads: a limit one less than the table's size, and
    // the table's *physical* address, because the core reading this is not
    // using a page table yet.
    let limit = (core::mem::size_of::<[u64; 4]>() - 1) as u16;
    let at = page.wrapping_add(GDT_POINTER_OFFSET as usize);
    // SAFETY: as above.
    unsafe { at.cast::<u16>().write_unaligned(limit) };
    // SAFETY: as above, into the four bytes after the limit.
    unsafe {
        at.wrapping_add(2).cast::<u32>().write_unaligned((TRAMPOLINE_PHYS + GDT_OFFSET) as u32);
    }

    // One block each, with the value computed outside it, because a block
    // covering five writes has a SAFETY comment covering whichever one the
    // reader happens to think of. `rsp` is zero here and written per core by
    // `wake`: it is the only parameter that differs between them.
    let cr4 = read_cr4();
    let efer = read_msr_efer();
    // SAFETY: as above.
    unsafe { write_param(PARAM_CR3, cr3) };
    // SAFETY: as above.
    unsafe { write_param(PARAM_CR4, cr4) };
    // SAFETY: as above.
    unsafe { write_param(PARAM_EFER, efer) };
    // SAFETY: as above.
    unsafe { write_param(PARAM_RSP, 0) };
    // SAFETY: as above.
    unsafe { write_param(PARAM_ENTRY, entry) };
}

/// Put one word in the parameter block.
///
/// # Safety
///
/// As [`install`], and `offset` must be one of the `PARAM_*` constants.
unsafe fn write_param(offset: u64, value: u64) {
    let at = (PHYS_OFFSET + TRAMPOLINE_PHYS + PARAMS_OFFSET + offset) as *mut u64;
    // SAFETY: the caller's guarantee. The address is inside the trampoline page
    // and reachable through the direct map, which is writable.
    unsafe { at.write_unaligned(value) };
}

/// Start the core with this APIC id and leave it to find its own way.
///
/// Returns as soon as the startup interrupts have been delivered. Whether the
/// core arrived is the caller's question, answered through the mailbox in
/// [`crate::smp`] rather than here — this file knows how to poke a processor
/// and nothing at all about what the kernel then expects of it.
///
/// # The sequence, and why the delays are in it
///
/// An assert-level `INIT` resets the core; the architecture requires ten
/// milliseconds before the startup interrupt that follows it. Two startup
/// interrupts rather than one, two hundred microseconds apart, because a core
/// that has already begun executing ignores the second — and a core that has
/// not is given a second chance rather than declared missing. This is the
/// sequence the manual sets out, and the delays are what makes it that sequence
/// rather than a race.
///
/// # Why the caller is given something to do during them
///
/// Those delays are four hundred microseconds of the boot processor doing
/// nothing, and they begin *before* the arriving core executes its first
/// instruction — the startup interrupt is inside them. A core that walks the
/// whole trampoline does so in a few microseconds, so by the time this function
/// returns the byte [`stage`] reads has already reached its last value, and a
/// caller that only starts looking afterwards can never see a stage before the
/// end. On a hypervisor that is slow enough to deliver a startup interrupt the
/// difference does not show, which is why it did not show here: QEMU catches
/// intermediate stages and the second machine this kernel booted on caught
/// exactly one.
///
/// So `watching` is called while the delays run. It can make the wait *longer*,
/// which is always safe — the second startup interrupt is ignored by a core
/// that has begun executing and welcome to one that has not — and it cannot
/// make it shorter, because the deadline is taken before the first call. What
/// it must not do is touch the trampoline page, which the core being started is
/// executing out of.
///
/// # Safety
///
/// `apic` must be this core's mapped register window, `tsc_khz` must be a
/// measured rate for this machine's timestamp counter, [`install`] must have
/// run, and `cpu` must not be a core that is already running.
pub unsafe fn wake(
    apic: u64,
    tsc_khz: u64,
    cpu: usize,
    stack_top: u64,
    watching: &mut dyn FnMut(),
) {
    // SAFETY: the trampoline is installed and nothing is executing it — the
    // core about to read it has not been started.
    unsafe { write_param(PARAM_RSP, stack_top) };
    // Cleared per core rather than once, so that a stage read while starting
    // the third core is the third core's and not a leftover from the second.
    // SAFETY: as above.
    unsafe { record(0) };

    let dest = (cpu as u32) << 24;
    let vector = u32::try_from(TRAMPOLINE_PHYS / FRAME_SIZE).unwrap_or(0);

    // SAFETY: the caller's guarantee that `apic` is this core's window.
    unsafe { icr(apic, dest, INIT_ASSERT) };
    spin_micros(tsc_khz, 10_000, watching);
    // SAFETY: as above.
    unsafe { icr(apic, dest, STARTUP | vector) };
    spin_micros(tsc_khz, 200, watching);
    // SAFETY: as above.
    unsafe { icr(apic, dest, STARTUP | vector) };
    spin_micros(tsc_khz, 200, watching);
}

/// Put a core the boot processor has given up on back where it started.
///
/// An `INIT` with no startup interrupt after it. The destination resets and
/// waits for a vector that is not coming, which is the state every core on this
/// machine was in before [`wake`] was called on it.
///
/// # Why giving up needs an action at all
///
/// [`crate::smp::start`] declares a core absent when it has not answered inside
/// its give-up bound, and then starts the *next* core — which rewrites the one
/// stack pointer in the trampoline page. A core that was merely slow rather
/// than absent would arrive after that write and run on its successor's stack,
/// which is the two-cores-on-one-stack failure the serial bring-up exists to
/// prevent. So a core that is given up on is not left alone; it is held, and
/// the assumption "it is not going to arrive" becomes something this kernel
/// enforces rather than something it hopes for. RFC 0068.
///
/// A destination that was never there ignores this, which is the ordinary case
/// and the reason it costs one register write.
///
/// # Safety
///
/// `apic` must be this core's mapped register window, and `cpu` must be a core
/// this kernel has not accepted as running — resetting a core that is executing
/// kernel code loses whatever it was doing.
pub unsafe fn hold(apic: u64, cpu: usize) {
    // SAFETY: the caller's guarantee that `apic` is this core's window.
    unsafe { icr(apic, (cpu as u32) << 24, INIT_ASSERT) };
}

/// Send an ordinary inter-processor interrupt to one core.
///
/// # Safety
///
/// As [`wake`], and `vector` must have a gate installed on the destination
/// core — an interrupt delivered to a vector with no handler is a fault on a
/// core that is in the middle of something else.
pub unsafe fn send(apic: u64, cpu: usize, vector: u8) {
    // SAFETY: the caller's guarantee.
    unsafe { icr(apic, (cpu as u32) << 24, FIXED | u32::from(vector)) };
}

/// Write the interrupt command register, waiting for the previous command to
/// have been accepted first and for this one afterwards.
///
/// # Safety
///
/// `apic` must be this core's mapped register window and `command` a legal
/// combination of delivery mode, level and vector.
unsafe fn icr(apic: u64, destination: u32, command: u32) {
    // The delivery-status bit is the only interlock the hardware offers: two
    // commands written without it are one command sent twice or not at all.
    // SAFETY: the caller's guarantee.
    unsafe { wait_idle(apic) };
    // The high half first. Writing the low half is what sends, so a destination
    // written afterwards would be the destination of the *next* command.
    // SAFETY: as above.
    unsafe { write_reg(apic, REG_ICR_HIGH, destination) };
    // SAFETY: as above.
    unsafe { write_reg(apic, REG_ICR_LOW, command) };
    // SAFETY: as above.
    unsafe { wait_idle(apic) };
}

/// Spin until the local APIC has accepted whatever was last written to the
/// interrupt command register.
///
/// # Safety
///
/// As [`icr`].
unsafe fn wait_idle(apic: u64) {
    // Bounded, because a local APIC that never clears this bit is a machine
    // that is not going to start another core and the boot has to say so rather
    // than stop. The bound is iterations rather than time on purpose: this runs
    // before the arriving core exists and must not depend on a clock it might
    // be sharing.
    for _ in 0..1_000_000u32 {
        // SAFETY: the caller's guarantee.
        if unsafe { read_reg(apic, REG_ICR_LOW) } & ICR_DELIVERY_PENDING == 0 {
            return;
        }
        core::hint::spin_loop();
    }
}

/// Wait, in microseconds, by watching the timestamp counter, and call
/// `watching` while waiting.
///
/// The only clock available here. `Env` is the substrate every *observation* of
/// time goes through, and this is not one: nothing is recorded, nothing is
/// reported, and the value never reaches a decision the seed could reproduce
/// differently. It is a delay the architecture requires between two writes to
/// a hardware register, in the same way `pit` counts a calibration interval.
///
/// The deadline is taken before the first call to `watching`, so what the
/// caller does in there can only lengthen the wait. [`wake`] is where that
/// matters and says why it is safe in both directions.
fn spin_micros(tsc_khz: u64, micros: u64, watching: &mut dyn FnMut()) {
    let ticks = tsc_khz.saturating_mul(micros) / 1_000;
    let deadline = super::read_tsc().saturating_add(ticks);
    while super::read_tsc() < deadline {
        watching();
        core::hint::spin_loop();
    }
}

/// This core's `CR4`.
///
/// Read rather than composed, because the arriving core has to agree with this
/// one about page-global enable and physical address extension both, and the
/// list of what `paging::enable_features` turned on lives there rather than
/// here.
fn read_cr4() -> u64 {
    let value: u64;
    // SAFETY: reading a control register is privileged and has no effect.
    unsafe {
        core::arch::asm!("mov {}, cr4", out(reg) value, options(nomem, nostack, preserves_flags));
    }
    value
}

/// The bits of `IA32_EFER` an arriving core is given.
///
/// System-call enable, long-mode enable, no-execute enable — bits 0, 8 and 11.
/// Everything else the boot processor happens to have is dropped, and the
/// dropped bit that matters is **`LMA`, bit 10**.
///
/// # Why a mask and not the register
///
/// `LMA` is *long mode active*, and it is a status bit the processor maintains:
/// it reads back as 1 on the boot processor because long mode is on. The core
/// reading this parameter is in thirty-two-bit protected mode with paging off,
/// where long mode is by definition **not** active, so handing it the boot
/// processor's value means writing a 1 to a read-only bit that currently reads
/// 0. Some processors ignore that. Some raise `#GP`, and a `#GP` in the
/// trampoline is taken with no descriptor table, no handler and nowhere to
/// report it — which is a triple fault and a silent reset, and from outside
/// looks exactly like a core that was started and never arrived.
///
/// The three bits kept are the three the arriving core cannot do without.
/// `LME` is what the far jump below depends on. `NXE` has to match the boot
/// processor's or the no-execute bit in the page tables it is about to load
/// becomes a reserved bit, and every kernel page that sets it faults. `SCE` is
/// `syscall`, which `ring3::init` would otherwise have to set again.
///
/// *Reversal:* a bit outside these three that a core has to arrive with. It
/// belongs in the mask with its own sentence, not in a copy of the whole
/// register — the register is what this function exists to stop copying.
const EFER_TO_ARRIVE_WITH: u64 = (1 << 0) | (1 << 8) | (1 << 11);

/// This core's `IA32_EFER`, masked to [`EFER_TO_ARRIVE_WITH`].
///
/// Read rather than composed, for the same reason as [`read_cr4`]: the arriving
/// core has to agree with this one about no-execute, and reading is how that
/// stays true when this one changes.
fn read_msr_efer() -> u64 {
    // SAFETY: `IA32_EFER` exists on every processor that can be in long mode,
    // which this one is.
    unsafe { super::read_msr(0xC000_0080) & EFER_TO_ARRIVE_WITH }
}

/// Read one APIC register. Same contract as the copy in [`super::apic`]; it is
/// duplicated rather than shared because that one is private to a module whose
/// state this file deliberately does not touch.
///
/// # Safety
///
/// `regs` must be a mapped local APIC window and `offset` a defined register.
unsafe fn read_reg(regs: u64, offset: u32) -> u32 {
    let at = (regs + u64::from(offset)) as *const u32;
    // SAFETY: the caller's guarantee. Volatile: this is a device.
    unsafe { at.read_volatile() }
}

/// Write one APIC register.
///
/// # Safety
///
/// As [`read_reg`], and the value must be one the register accepts.
unsafe fn write_reg(regs: u64, offset: u32, value: u32) {
    let at = (regs + u64::from(offset)) as *mut u32;
    // SAFETY: the caller's guarantee.
    unsafe { at.write_volatile(value) };
}

/// The low half of the interrupt command register. Writing it sends.
const REG_ICR_LOW: u32 = 0x300;

/// The high half, which holds the destination APIC id in its top eight bits.
const REG_ICR_HIGH: u32 = 0x310;

/// The command has not been accepted by the destination yet.
const ICR_DELIVERY_PENDING: u32 = 1 << 12;

/// Delivery mode 101, level asserted: reset the destination core.
const INIT_ASSERT: u32 = (0b101 << 8) | (1 << 14);

/// Delivery mode 110, level asserted: begin executing at `vector << 12`.
const STARTUP: u32 = (0b110 << 8) | (1 << 14);

/// Delivery mode 000, level asserted: an ordinary interrupt at a vector.
const FIXED: u32 = 1 << 14;

// The trampoline itself.
//
// Written in AT&T syntax for the same reason `boot.rs` is: a far jump with an
// immediate selector is unambiguous there, and this file has four of them.
//
// Every address in it is a literal. That is not a style choice — this code is
// assembled at one address and copied to another, so a `rip`-relative or
// link-time-absolute reference would name where it was built rather than where
// it runs. Where a label inside the trampoline is needed, it is written as
// `TRAMPOLINE_PHYS + (label - start)`, which the assembler folds to a constant
// because both labels are in this one section.
global_asm!(
    r#"
    .section .rodata.trampoline, "a", @progbits
    .balign 16
    .globl ap_trampoline_start
ap_trampoline_start:
    .code16
    // A startup interrupt arrives with CS = vector << 8 and IP = 0, which is
    // the right physical address expressed in the wrong pair of numbers. Every
    // reference below assumes a zero segment base, so the first thing to do is
    // make one.
    cli
    cld
    ljmp $0, $(0x8000 + (1f - ap_trampoline_start))
1:
    xorw %ax, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs

    // Every `movb` to 0x8F28 below is one number written to one byte, and the
    // boot processor is watching it while it waits. `PARAM_STAGE` says why the
    // progress of this code is reported by the code itself rather than by
    // whoever is waiting for it. Paging is off until the far jump at the end,
    // so these are plain physical stores into a page nothing else is using.
    movb $1, 0x8F28

    // The descriptor table the boot processor left in this page. `lgdtl` and
    // not `lgdt`: the sixteen-bit form truncates the base to twenty-four bits,
    // which is a table at an address that is almost right.
    lgdtl 0x8E20
    movb $2, 0x8F28

    movl %cr0, %eax
    orl $1, %eax
    movl %eax, %cr0
    ljmpl $0x08, $(0x8000 + (2f - ap_trampoline_start))

    // Thirty-two-bit protected mode, paging still off.
    .code32
2:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs
    movb $3, 0x8F28

    // Physical address extension and whatever else the boot processor turned
    // on, then the address space it is running in. Order matters: `CR4.PAE`
    // must be set before long mode is enabled, and `CR3` must hold a four-level
    // table before paging is.
    movl 0x8F08, %eax
    movl %eax, %cr4
    movb $4, 0x8F28

    movl 0x8F00, %eax
    movl %eax, %cr3
    movb $5, 0x8F28

    // Long-mode enable, the no-execute switch and system-call enable, taken
    // from the boot processor and masked to those three — `EFER_TO_ARRIVE_WITH`
    // is where the mask is argued, and the bit it exists to drop is `LMA`.
    // Assigned rather than merged, because what is being assigned is the other
    // core's value for these exact bits.
    movl $0xC0000080, %ecx
    movl 0x8F10, %eax
    movl 0x8F14, %edx
    wrmsr
    // The last store before paging: the on-ramp page is mapped read-only into
    // the address space `CR3` now names, so from the far jump onwards this
    // progress goes through the direct map instead. A core that stops at 6 is a
    // core the `wrmsr` above killed; one that stops at 5 was killed by it.
    movb $6, 0x8F28

    // Paging. The instruction after this one executes at this same low address
    // through the kernel's page tables, which is what the on-ramp page is for.
    movl %cr0, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0

    ljmpl $0x18, $(0x8000 + (3f - ap_trampoline_start))

    // Sixty-four-bit mode, in the kernel's address space, still executing out
    // of the on-ramp page. The only job left down here is to leave.
    .code64
3:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs

    // Through the direct map from here: the page this instruction is fetched
    // from is read-only in the space now loaded, and its physical address is
    // writable only through the map that covers all of memory.
    movabsq $0xffff800000008F28, %rax
    movb $7, (%rax)

    // Loaded through a register rather than as an absolute memory operand:
    // sixty-four-bit addressing has no plain absolute form for a destination
    // other than the accumulator, and spelling it this way is unambiguous.
    movl $0x8F18, %eax
    movq (%rax), %rsp
    xorq %rbp, %rbp

    movabsq $0xffff800000008F28, %rax
    movb $8, (%rax)

    movl $0x8F20, %eax
    movq (%rax), %rax
    jmp *%rax

    .globl ap_trampoline_end
ap_trampoline_end:
"#,
    options(att_syntax)
);
