// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The frame.
//!
//! # Milestone M0
//!
//! Boots, reports, and is already deterministic. The done-criterion from
//! `docs/design/ring-scene-boot.html` section 15:
//!
//! > `cargo xtask run` prints a banner in QEMU and exits with status 0, driven
//! > by `-device isa-debug-exit` so the kernel can set its own exit code and a
//! > real integration test can assert on it — and a seed plus a commit hash
//! > reproduces a run byte for byte.
//!
//! # What is deliberately absent
//!
//! No filesystem, no general heap, no drivers beyond serial, no graphics, no
//! imported code. Phase 00 exists to prove exactly two propositions — that
//! isolation holds under adversarial use, and that the ring is genuinely faster
//! than a system call. Anything serving neither is later work.

#![no_std]
#![no_main]

pub mod arch;
pub mod jitter;
pub mod mem;
pub mod percpu;

use core::panic::PanicInfo;

use arch::x86_64::multiboot::{BootInfo, Region, RegionKind};
use arch::x86_64::paging;
use f_env::{Env, SeededEnv};

// `kprint!`/`kprintln!` are `#[macro_export]`ed from the serial module, which
// places them at the crate root — so they are already in scope here, and
// importing them again is a redefinition rather than a clarification. Do not
// re-add the `use`; the export is what makes the ordering non-fragile.

/// The seed this build runs under.
///
/// A later milestone has `xtask` generate this so a simulator run is selected
/// from the command line. Until then it is fixed, which is enough for the M0
/// contract: the same binary must always produce the same digest.
const SEED: u64 = 0xF00D_BEEF_CAFE_1234;

/// Kernel entry point, called by the boot stub once the machine is in long
/// mode.
///
/// # The handoff
///
/// `magic` and `info` are the two registers a multiboot 1 loader leaves behind,
/// and they are the entire width of this system's dependency on how it was
/// booted. Everything above this function sees a validated
/// [`arch::x86_64::multiboot::BootInfo`] and nothing else, so replacing the
/// protocol later — Limine or UEFI, when E5 names a real machine — is a change
/// to two files and no interface.
///
/// The memory map is what M1 builds a physical frame allocator from. Printing a
/// summary of it is not decoration: it is the evidence that the handoff
/// happened, and it is the first thing to read when a machine boots to a black
/// screen.
#[unsafe(no_mangle)]
pub extern "C" fn kmain(magic: u32, info: u32) -> ! {
    let serial = arch::x86_64::serial::Serial;
    serial.init();

    kprintln!();
    kprintln!("F — milestone M0");
    kprintln!("  abi version   {}", f_abi::ABI_VERSION);
    kprintln!("  sqe size      {} bytes", core::mem::size_of::<f_abi::Sqe>());
    kprintln!("  cqe size      {} bytes", core::mem::size_of::<f_abi::Cqe>());

    // Before the descriptor tables, because they are the first thing to live in
    // a per-CPU slot and a shard that hands out the wrong address would install
    // them somewhere nothing points at.
    match percpu::self_test() {
        Ok(cpu) => kprintln!("  per-cpu       core {cpu} of {}, slots distinct", percpu::MAX_CPUS),
        Err(why) => {
            kprintln!("FAIL: per-cpu state: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // Descriptor tables before anything that could fault. The stub's table is
    // in low memory, which the kernel's address space stops mapping a few lines
    // below — and an exception with no handler is a reset with no output.
    // SAFETY: once on this core, which is the only one running, interrupts
    // still disabled.
    unsafe { arch::x86_64::gdt::init() };
    // SAFETY: as above, and after the code selector its gates name exists.
    unsafe { arch::x86_64::idt::init() };

    // Prove the whole path — stub, save, dispatch, restore, iretq — with an
    // exception that is meant to be survived. If this returns, a fault later
    // will report rather than reset.
    // SAFETY: a breakpoint is a deliberate trap with a handler installed above.
    unsafe { core::arch::asm!("int3", options(nomem, nostack)) };

    let boot = report_memory(magic, info);

    // The determinism substrate is live from the first line of kernel code that
    // observes anything. Nothing below may read the clock directly.
    let mut env = SeededEnv::new(SEED, 100);
    kprintln!("  seed          {SEED:#018x}");

    let mut mixed: u64 = 0;
    for _ in 0..8 {
        mixed ^= env.next_u64();
    }
    kprintln!("  env digest    {mixed:#018x}");
    kprintln!("  env clock     {} ns", env.now().as_nanos());

    // The same seed must always produce the same digest. This is the weakest
    // possible form of the reproducibility contract, asserted at boot so that a
    // regression in the substrate is caught on the very next run rather than
    // months later when the simulator stops reproducing.
    let mut check = SeededEnv::new(SEED, 100);
    let mut expect: u64 = 0;
    for _ in 0..8 {
        expect ^= check.next_u64();
    }
    if expect != mixed {
        kprintln!("FAIL: determinism substrate is not reproducible");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    kprintln!("  determinism   ok");

    // M1. Three steps that have to happen in this order: take a copy of the
    // memory map while the loader's own copy is still reachable, build an
    // allocator from it, then use that allocator to build an address space and
    // stop depending on the boot stub's.
    let (map, regions, truncated) = collect(&boot);
    if truncated {
        kprintln!("  note          memory map truncated at {} regions", map.len());
    }

    // The value that masks every free-list link. It comes from the environment
    // rather than from the hardware because a defence that made a run
    // irreproducible would cost more than it bought — see `mem`.
    let mut frames = mem::FrameAllocator::new(env.next_u64());
    // SAFETY: every region came from a validated handoff, and the reserved list
    // covers the kernel image, the structures the loader still owns, and every
    // module it loaded.
    unsafe { populate(&mut frames, &map[..regions], info, &boot) };
    kprintln!("  frames        {} free of {}", frames.free_count(), frames.total_count());

    let highest = map[..regions]
        .iter()
        .filter(|r| r.kind == RegionKind::Usable)
        .map(|r| r.base.saturating_add(r.len))
        .max()
        .unwrap_or(0);

    // Ask the machine for the two bits the mappings below want to set, before
    // any table that sets them exists. Both are reserved until enabled, so a
    // mapping built for a feature the processor has not agreed to interpret is
    // a page fault rather than the protection it was meant to be.
    // SAFETY: boot processor, in long mode, before any address space carrying
    // these bits is activated.
    let features = unsafe { paging::enable_features() };
    kprintln!(
        "  paging        no-execute {}, global pages {}, pcid {}, direct map in {}",
        if features.nx { "on" } else { "unavailable" },
        if features.global { "on" } else { "unavailable" },
        // Not enabled on purpose, and the log says which of the two reasons it
        // is off for: nothing to switch between yet, or nothing to switch with.
        if features.pcid { "available, unused until E0-B09" } else { "unavailable" },
        // The grain the direct map is built with. Not a protection — a saving,
        // and the one number that says whether this machine offered the larger
        // page or the mapping fell back to the smaller one.
        if features.gigabyte_pages { "1 GiB pages" } else { "2 MiB pages" },
    );

    // SAFETY: the boot stub's identity window is still live, which is what
    // makes a freshly allocated frame writable through `frames.virt()` while
    // the offset is still zero.
    let space = match unsafe { paging::build(&mut frames, highest, features) } {
        Ok(space) => space,
        Err(e) => {
            kprintln!("FAIL: {}", e.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    // The switch, and the two statements that must not be separated: after the
    // first, every physical address the kernel holds is reachable only through
    // the direct map, and the allocator does not know that until the second.
    // The code doing this is in the kernel window, which both address spaces
    // map at the same address — that is what makes it survivable.
    // SAFETY: instruction pointer, stack and every address in use are in the
    // kernel window, which `space` maps exactly as the stub did.
    unsafe { paging::activate(&space) };
    // SAFETY: the direct map built above covers all usable physical memory, so
    // every frame the allocator holds is reachable at PHYS_OFFSET + physical —
    // and the limit is the map's own, reported by the thing that built it
    // rather than assumed to be everything.
    unsafe { frames.rebind(paging::PHYS_OFFSET, space.direct_limit()) };

    kprintln!(
        "  address space {:#018x} root, direct map at {:#018x}",
        space.root(),
        paging::PHYS_OFFSET
    );

    // Memory the identity window could not reach is now reachable. Nothing was
    // skipped on this machine; the pass exists so that the first machine with
    // more than a gibibyte does not quietly lose the rest of it.
    let before = frames.free_count();
    // SAFETY: as the first pass, with everything below the old limit excluded
    // so that no frame is added twice.
    unsafe { reclaim(&mut frames, &map[..regions], info, &boot) };
    let reclaimed = frames.free_count() - before;
    if reclaimed > 0 {
        kprintln!("  reclaimed     {reclaimed} frame(s) above the old identity map");
    }

    match mem::self_test(&mut frames, &mut env) {
        Ok(()) => kprintln!("  frame alloc   ok"),
        Err(why) => {
            kprintln!("FAIL: frame allocator: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // Nothing a frame's last owner wrote may reach its next one. There is no
    // component boundary to cross yet, which is exactly why this is asserted
    // now: the property has to hold on the day one appears, and a property
    // first tested on that day is a property first debugged on that day.
    match mem::hygiene_test(&mut frames) {
        Ok(()) => kprintln!(
            "  frame hygiene ok — {} clean, {} dirty",
            frames.clean_count(),
            frames.dirty_count()
        ),
        Err(why) => {
            kprintln!("FAIL: frame hygiene: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // M2. The first device the kernel maps, and the point at which an
    // interrupt becomes something that can be arranged rather than only
    // survived. Nothing is delivered by this call: it makes delivery possible.
    // SAFETY: boot processor, once, interrupts still disabled, after `idt::init`
    // on this core, and after the switch to `space` with `frames` rebound onto
    // its direct map — which is the whole list `apic::init` asks for.
    match unsafe { arch::x86_64::apic::init(&mut frames, &space, features) } {
        Ok(found) => kprintln!(
            "  local apic    xapic at {:#018x}, version {:#04x}, {} lvt entries",
            found.phys,
            found.version,
            u16::from(found.max_lvt) + 1,
        ),
        Err(why) => {
            kprintln!("FAIL: local apic: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    run_timer(&boot);

    // A fault on purpose, when asked for one. This is how the report path is
    // tested: `cargo xtask fault <kind>` boots with the parameter, and the run
    // is expected to end in a dump and a failure exit rather than in `M0 ok`.
    provoke(&boot, features);

    kprintln!("M0 ok");
    arch::x86_64::exit_qemu(arch::x86_64::Exit::Success)
}

/// The rate everything about M2 is stated at.
///
/// A kilohertz, from `docs/design/ring-scene-boot.html`: *done when a 1 kHz
/// timer runs for 60 seconds and you have a jitter histogram*.
const TIMER_HZ: u32 = 1_000;

/// How many ticks an ordinary boot waits for.
///
/// A tenth of a second. Enough to prove the whole path — arm, deliver,
/// dispatch, record, re-arm, disarm — on every single run, and short enough
/// that nobody starts skipping the boot to avoid it. The measurement run is
/// six hundred times longer and is asked for explicitly.
const PROBE_TICKS: u64 = 100;

/// Calibrate the clocks and run the timer.
///
/// Two runs live here and the difference between them is the whole reason the
/// split exists. An ordinary boot takes [`PROBE_TICKS`] and prints only things
/// that cannot vary — which mechanism, how many ticks — because the boot log is
/// a fixture and two runs of one commit have to match byte for byte.
///
/// `timer=<seconds>` on the command line is the measurement. It prints the
/// histogram, the frequencies it was denominated in, and everything else that
/// moves. Nothing asserts on that output, and nothing should: it is a
/// measurement, and `claims/0002-timer-jitter.toml` is where a measurement
/// becomes something anybody is allowed to quote.
fn run_timer(boot: &BootInfo) {
    match jitter::self_test() {
        Ok(()) => kprintln!("  jitter        ok"),
        Err(why) => {
            kprintln!("FAIL: jitter histogram: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // SAFETY: boot processor, once, interrupts still disabled — which is what
    // keeps the calibration interval the length the 8254 says it is — and after
    // `apic::init` on this core.
    let clocks = match unsafe { arch::x86_64::apic::calibrate() } {
        Ok(clocks) => clocks,
        Err(why) => {
            kprintln!("FAIL: clock calibration: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    // Which mechanism, and against what — both fixed properties of the machine.
    // The two frequencies are measurements and stay out of this line.
    kprintln!(
        "  clocks        measured against the 8254 over {} ms; timer via {}",
        arch::x86_64::pit::CALIBRATE_MICROS / 1_000,
        clocks.backend.label(),
    );

    let seconds = boot.parameter_u32(b"timer=");
    let target = match seconds {
        Some(seconds) => u64::from(seconds) * u64::from(TIMER_HZ),
        None => PROBE_TICKS,
    };

    // SAFETY: this core was brought up and calibrated above, `idt::init` has
    // installed the timer's vector, and interrupts are disabled on entry —
    // `run` enables them for the duration and disables them again.
    let summary = match unsafe { arch::x86_64::apic::run(TIMER_HZ, target) } {
        Ok(summary) => summary,
        Err(why) => {
            kprintln!("FAIL: timer: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    // A short run is a failure however it is dressed up: the timer stopped
    // firing and the histogram is of whatever happened before it did.
    if summary.ticks != summary.target {
        kprintln!("FAIL: timer stopped after {} of {} ticks", summary.ticks, summary.target);
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    if seconds.is_none() {
        kprintln!("  timer         {} ticks at {} Hz", summary.ticks, summary.hz);
        return;
    }

    kprintln!();
    kprintln!(
        "TIMER — {} ticks at {} Hz via {}",
        summary.ticks,
        summary.hz,
        summary.backend.label()
    );
    kprintln!("    tsc           {} kHz", summary.tsc_khz);
    kprintln!("    apic timer    {} kHz", clocks.apic_khz);
    kprintln!("    missed        {} tick(s) a full period or more late", summary.missed);

    let mut serial = arch::x86_64::serial::Serial;
    summary.late.report(summary.tsc_khz, &mut serial);
    kprintln!();
}

/// Fault deliberately, if the command line asked.
///
/// The exception report is the one piece of kernel machinery that cannot be
/// tested by using the kernel normally: it runs only when something has gone
/// wrong, which means it is either exercised on purpose or discovered to be
/// broken at the worst possible moment. So it is exercised on purpose.
fn provoke(boot: &BootInfo, features: paging::Features) {
    if boot.has_parameter(b"fault=nx") {
        if !features.nx {
            // Not a failure of the kernel, and not something to pretend was
            // tested. The run finishes normally and `xtask fault` says the
            // provocation did not fault, which is the truth.
            kprintln!("  no-execute    unavailable on this machine; nothing to provoke");
            return;
        }
        kprintln!("  provoking     an execute from the direct map, deliberately");
        // Physical memory, reached through the window that exists to read and
        // write it. There is a real page there and it is a perfectly good
        // *data* mapping — which is exactly the point: the fault proves the
        // protection is on rather than merely intended, and no other check can
        // prove that, because a bit set in a table nobody executes from tells
        // you nothing about whether the processor is enforcing it.
        let at = paging::PHYS_OFFSET + 0x0010_0000;
        // SAFETY: none. That is the point — this is a deliberate fault, and the
        // no-execute mapping is what is under test.
        unsafe { core::arch::asm!("jmp {}", in(reg) at, options(noreturn)) }
    } else if boot.has_parameter(b"fault=pf") {
        kprintln!("  provoking     a page fault, deliberately");
        // A canonical higher-half address that nothing maps: the direct map
        // covers physical memory and the kernel window covers the image, and
        // this is neither.
        let wild = 0xFFFF_C000_DEAD_B000u64 as *mut u64;
        // SAFETY: none. That is the point — this is a deliberate fault, and the
        // handler is what is under test.
        unsafe { wild.write_volatile(1) };
    } else if boot.has_parameter(b"fault=ud") {
        kprintln!("  provoking     an invalid opcode, deliberately");
        // SAFETY: as above.
        unsafe { core::arch::asm!("ud2", options(nomem, nostack)) };
    } else if boot.has_parameter(b"fault=wx") {
        kprintln!("  provoking     a write to the kernel's own text, deliberately");
        // The other half of write-exclusive-or-execute, and the half that is
        // easy to believe without checking: the text is mapped read-only, and
        // the write-protect bit is what makes that apply to ring 0 rather than
        // only to user space. Without it this write would simply succeed.
        let text = kmain as *const () as *mut u8;
        // SAFETY: none. That is the point.
        unsafe { text.write_volatile(0x90) };
    } else if boot.has_parameter(b"fault=stack") {
        kprintln!("  provoking     a stack overflow, deliberately");
        // Recursion the compiler cannot fold into a loop, because each frame
        // reads a local the next one cannot see. The linker script leaves one
        // page unmapped below the stack, so this ends at the guard: a page
        // fault naming an address just under the stack, rather than a slow
        // corruption of whatever `.bss` used to be underneath it.
        // Clippy is right that this cannot return, which is the whole design:
        // the guard page is what stops it, and the report is what proves the
        // guard page is there.
        #[allow(unconditional_recursion)]
        fn descend(depth: u64) -> u64 {
            let anchor = [depth; 32];
            // SAFETY: reading a local this function owns.
            let seen = unsafe { (&raw const anchor).cast::<u64>().read_volatile() };
            descend(depth + 1).wrapping_add(seen)
        }
        let _ = descend(0);
    } else if boot.has_parameter(b"fault=df") {
        kprintln!("  provoking     a fault with no usable stack, deliberately");
        // The stack is pointed somewhere unmapped and then an exception is
        // raised. Delivering it means pushing a frame, pushing faults, and a
        // fault while delivering a fault is a double fault — which is the one
        // case that cannot be reported on the stack that caused it. It is
        // reported anyway, on the stack the descriptor names.
        //
        // Deliberately *not* a stack overflow, which is the other way to get
        // here and now has its own provocation: `fault=stack` ends at the guard
        // page, as a page fault. Before that guard existed it ended here — an
        // overflow walking down through the descriptor tables, corrupting the
        // machinery that would have reported it, and resetting the machine.
        // SAFETY: none, on purpose. The handler is what is under test.
        unsafe {
            core::arch::asm!(
                "mov rsp, {bad}",
                "ud2",
                bad = in(reg) 0xFFFF_C000_0000_0000u64,
                options(noreturn),
            )
        }
    }
}

/// Validate the boot handoff and print what the loader reported.
///
/// A failure here is fatal and says so. There is no recovery available: without
/// a memory map there is nothing for M1's frame allocator to allocate from, and
/// a kernel that carries on regardless would be inventing the one fact it most
/// needs to be true.
fn report_memory(magic: u32, info: u32) -> BootInfo {
    // SAFETY: `magic` and `info` are the values the boot stub captured from
    // `eax` and `ebx` at entry and passed through unchanged. `BootInfo::new`
    // treats both as untrusted and validates before dereferencing anything.
    let boot = unsafe { BootInfo::new(magic, info) };

    let boot = match boot {
        Ok(boot) => boot,
        Err(e) => {
            kprintln!("FAIL: {}", e.message());
            kprintln!("  magic         {magic:#010x}");
            kprintln!("  info          {info:#010x}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    kprintln!("  loader        multiboot 1");

    let mut usable: u64 = 0;
    let mut regions: u32 = 0;
    for region in boot.regions() {
        regions += 1;
        if region.kind == RegionKind::Usable {
            usable += region.len;
        }
        kprintln!(
            "    {:#018x}  {:>10} KiB  {}",
            region.base,
            region.len / 1024,
            region.kind.label()
        );
    }

    kprintln!("  regions       {regions}");
    kprintln!("  usable        {} KiB", usable / 1024);
    kprintln!(
        "  loader says   {} KiB low, {} KiB high",
        boot.mem_lower_kib(),
        boot.mem_upper_kib()
    );

    // Modules sit inside memory the loader also called usable, so they are
    // printed next to the map rather than somewhere else: the reserved list and
    // this report should be read together or neither is worth much.
    for module in boot.modules() {
        kprintln!(
            "  module        {:#018x}..{:#018x}  {} KiB",
            module.start,
            module.end,
            module.len() / 1024
        );
    }
    if boot.modules_dropped() > 0 {
        // Not fatal yet, because nothing depends on a module at M1. It becomes
        // fatal at E0-B10, where the first one is loaded and where an
        // unreserved module is memory handed out from under its owner.
        kprintln!(
            "  note          {} module(s) beyond what is tracked, and NOT reserved",
            boot.modules_dropped()
        );
    }

    // A map with no usable memory in it is a map that was misread, not a
    // machine with no memory: the kernel is running out of some of it.
    if usable == 0 {
        kprintln!("FAIL: memory map reports nothing usable");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    boot
}

/// How many regions a memory map may have before this kernel stops reading it.
///
/// QEMU reports seven and a large server reports a few dozen. Thirty-two is
/// past both and small enough to sit on the stack, which is where it has to sit
/// because there is no allocator yet at the point it is filled.
const MAX_REGIONS: usize = 32;

/// Take a copy of the memory map before anything can unmap the original.
///
/// The loader's structure is walked lazily, and it lives in low physical memory
/// that the kernel's own address space does not map. Reading it after the
/// switch would be a fault; reading it before and keeping the answer is free.
fn collect(boot: &BootInfo) -> ([Region; MAX_REGIONS], usize, bool) {
    let empty = Region { base: 0, len: 0, kind: RegionKind::Reserved };
    let mut map = [empty; MAX_REGIONS];
    let mut count = 0;
    let mut truncated = false;

    for region in boot.regions() {
        if count == MAX_REGIONS {
            truncated = true;
            break;
        }
        map[count] = region;
        count += 1;
    }

    (map, count, truncated)
}

/// How many reserved ranges the boot path can carry.
///
/// Three fixed, one for the second pass's exclusion of everything the first
/// already took, and one per module the handoff kept. Sized so that the list
/// can never be the thing that drops a reservation: a range that does not fit
/// is memory handed to somebody while its owner is still using it, and there is
/// no diagnostic for that worth the name.
const MAX_RESERVED: usize = 12;

/// Everything inside usable memory that is already spoken for.
///
/// `extra` is for the second pass, which must exclude everything the first pass
/// already added. It is placed before the modules rather than after so that it
/// cannot be the entry that falls off the end.
fn reserved_ranges(
    info: u32,
    boot: &BootInfo,
    extra: Option<mem::Reserved>,
) -> ([mem::Reserved; MAX_RESERVED], usize) {
    let (mmap_base, mmap_len) = boot.mmap_extent();
    let mut list = [mem::Reserved::empty(); MAX_RESERVED];

    // The kernel is running out of this. Handing it out would be a fault with a
    // delay fuse rather than a fault.
    list[0] = mem::Reserved { base: kernel_phys_start(), end: kernel_phys_end() };
    // The loader's info structure. One frame, conservatively: the protocol does
    // not say how large it is and the answer does not matter.
    list[1] = mem::Reserved::new(u64::from(info), mem::FRAME_SIZE);
    // The memory map, which is read from before the address space switch and
    // must survive until it has been copied.
    list[2] = mem::Reserved::new(mmap_base, mmap_len);
    let mut count = 3;

    if let Some(extra) = extra {
        list[count] = extra;
        count += 1;
    }

    // Every file the loader placed in memory. These sit *inside* regions the
    // same loader called usable, which is what makes forgetting them a bug that
    // waits until something first depends on a module's contents — E0-B10 —
    // and then presents as that module being subtly wrong.
    for module in boot.modules() {
        if count == MAX_RESERVED {
            break;
        }
        list[count] = mem::Reserved { base: module.start, end: module.end };
        count += 1;
    }

    (list, count)
}

/// Give the allocator every usable frame that is not already spoken for.
///
/// # Safety
///
/// The regions must come from a validated handoff, and `info` must be the
/// pointer the boot stub captured.
unsafe fn populate(frames: &mut mem::FrameAllocator, map: &[Region], info: u32, boot: &BootInfo) {
    let (reserved, count) = reserved_ranges(info, boot, None);
    for region in map.iter().filter(|r| r.kind == RegionKind::Usable) {
        // SAFETY: the loader reported this region as usable, and everything
        // inside it that is already owned is in `reserved`.
        unsafe { frames.add_region(region.base, region.len, &reserved[..count]) };
    }
}

/// Add the frames the identity window could not reach.
///
/// # Safety
///
/// Must run only after the address space switch, and only once: everything
/// below the old limit is excluded so that no frame is offered twice.
unsafe fn reclaim(frames: &mut mem::FrameAllocator, map: &[Region], info: u32, boot: &BootInfo) {
    // Everything the first pass already added, as one range.
    let already = mem::Reserved { base: 0, end: mem::IDENTITY_LIMIT };
    let (reserved, count) = reserved_ranges(info, boot, Some(already));

    for region in map.iter().filter(|r| r.kind == RegionKind::Usable) {
        // SAFETY: as the first pass, and the added range makes double-adding
        // impossible rather than unlikely.
        unsafe { frames.add_region(region.base, region.len, &reserved[..count]) };
    }
}

unsafe extern "C" {
    /// First byte of the loaded image, physically. From the linker script.
    static __kernel_phys_start: u8;
    /// One past the last byte of the loaded image, physically.
    static __kernel_phys_end: u8;
}

/// Where the kernel image starts in physical memory.
///
/// Physical rather than virtual because the frame allocator deals in frames,
/// and because the kernel is now linked high and loaded low — the two numbers
/// stopped being the same at E0-B04. The linker script computes the subtraction
/// so that nothing here has to.
fn kernel_phys_start() -> u64 {
    (&raw const __kernel_phys_start) as u64
}

/// One past where the kernel image ends in physical memory.
fn kernel_phys_end() -> u64 {
    (&raw const __kernel_phys_end) as u64
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // A panic in the frame is a bug in the frame. Report it in full and stop —
    // never attempt to continue, and never let a panic become the mechanism by
    // which a peer halts the system.
    kprintln!();
    kprintln!("KERNEL PANIC");
    kprintln!("  {info}");
    arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure)
}
