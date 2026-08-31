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
pub mod cap;
pub mod env;
pub mod jitter;
pub mod mem;
pub mod percpu;
pub mod process;
pub mod smp;

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
    // First, before the serial port. Everything after this line is boot, and a
    // stamp taken later would be measuring the part of boot somebody chose to
    // include. The counter is free-running and needs no calibration to *read*;
    // what needs calibration is turning the delta into nanoseconds, and that
    // has happened by the time anything asks.
    let entered = arch::x86_64::read_tsc();

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
    let mut seeded = SeededEnv::new(SEED, 100);
    kprintln!("  seed          {SEED:#018x}");

    let mut mixed: u64 = 0;
    for _ in 0..8 {
        mixed ^= seeded.next_u64();
    }
    kprintln!("  env digest    {mixed:#018x}");
    kprintln!("  env clock     {} ns", seeded.now().as_nanos());

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
    let mut frames = mem::FrameAllocator::new(seeded.next_u64());
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
        // is off for: nothing worth switching between yet, or nothing to switch
        // with. E0-B09 was the milestone this line used to wait for and it has
        // arrived: there is a second address space now, entered once per boot
        // and left once. Tagging translations buys nothing at that rate, and an
        // address-space identifier that is wrong is a process reading another
        // one's memory through a stale translation — the one failure mode in
        // this file with no fault behind it. The condition to revisit is a
        // scheduler that switches between processes often enough to measure,
        // which is E1.
        if features.pcid { "available, and deliberately off" } else { "unavailable" },
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

    match mem::self_test(&mut frames, &mut seeded) {
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

    let clocks = calibrate();

    // M2, and the substrate's other half. Everything above this line ran on the
    // seed. This is what a machine gives instead: the timestamp counter behind
    // the clock that has ordering authority, and the CMOS behind a wall time
    // that may be stamped on things and may order nothing. RFC 0009.
    //
    // The arithmetic first, because it is the part that can be wrong quietly —
    // an overflow an hour into a boot, a calendar wrong by a day — and because
    // there is no point checking a contract on top of a conversion that lies.
    if let Err(why) = env::self_test().and_then(|()| arch::x86_64::rtc::self_test()) {
        kprintln!("FAIL: env arithmetic: {why}");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    // SAFETY: boot processor, once on this core, after `apic::calibrate` on the
    // same core, and with interrupts disabled — `apic::run` disabled them again
    // before returning. That is the whole list `Hardware::new` asks for.
    let mut hardware = unsafe { env::Hardware::new(clocks.tsc_khz) };

    match hardware.wall() {
        // The uncertainty and not the time. A boot log is a fixture, and the
        // one number here that does not move between two runs of one commit is
        // how wrong the reading could be.
        Some(stamp) => kprintln!(
            "  wall clock    firmware rtc, uncertain to {} s",
            stamp.uncertainty_nanos / 1_000_000_000
        ),
        // Not a failure. RFC 0009 makes this an `Option` precisely so a machine
        // with nothing trustworthy can say so, rather than produce a plausible
        // number that is usable and wrong.
        None => kprintln!("  wall clock    none — nothing here worth believing"),
    }

    // One contract, both environments, on the same boot. A property checked
    // against only the seeded environment is a property the hardware one gets
    // to violate, and the hardware one is where a violation cannot be
    // reproduced afterwards.
    if let Err(why) = f_env::contract::check(&mut seeded) {
        kprintln!("FAIL: seeded env: {}", why.message());
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }
    if let Err(why) = f_env::contract::check(&mut hardware) {
        kprintln!("FAIL: hardware env: {}", why.message());
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }
    kprintln!("  env contract  arithmetic ok, seeded ok, hardware ok");

    // E0-B10. Every other core the machine has, brought up to the same point
    // this one reached above — its own descriptor tables, its own local APIC,
    // its own system-call entry — and then left waiting for something to do.
    //
    // Nothing an arriving core does is printed by the core that does it. Two
    // cores writing to one serial port produce interleaved bytes and the boot
    // log is a fixture, so a started core records what it found in its own
    // shards and this is where the count is said.
    //
    // SAFETY: the boot processor, once, with the kernel's address space active,
    // `frames` rebound onto its direct map, after `apic::init` and
    // `apic::calibrate` on this core, and with interrupts disabled.
    match unsafe { smp::start(&mut frames, &space, clocks) } {
        Ok(found) => {
            if found.cores == 1 {
                kprintln!("  cores         1 — this machine has no other, and nothing waits");
            } else {
                kprintln!(
                    "  cores         {} of {} shards, each with its own tables and stacks",
                    found.cores,
                    percpu::MAX_CPUS
                );
            }
            // Said rather than left implicit. A machine with more cores than
            // this kernel shards for runs correctly on the ones it started and
            // leaves the rest asleep, and a log that reported only the number
            // started would be hiding which of those two it was.
            if found.present > found.cores {
                kprintln!(
                    "  note          the processor reports {} — {} left asleep, past MAX_CPUS",
                    found.present,
                    found.present - found.cores
                );
            }
        }
        Err(why) => {
            kprintln!("FAIL: bringing up a core: {}", why.message());
            if let Some(cpu) = why.core() {
                kprintln!("  core          {cpu}");
            }
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // M4. The five properties, against a real table and against five tables
    // broken on purpose — one per property. This runs before ring 3 exists on
    // this boot, and that order is the point: the negative suite from ring 3
    // can only report that a hostile handle was refused, and it cannot report
    // that a table which *stopped* refusing would have been noticed. That is
    // what the flawed fixtures are for, and a suite whose checks cannot fail is
    // a suite nobody has tested.
    match cap::properties::self_test() {
        Ok(caught) => kprintln!(
            "  capabilities  {} slots, {} properties hold, {caught} flawed tables caught",
            cap::TABLE_SLOTS,
            cap::properties::Property::all().len(),
        ),
        Err(why) => {
            kprintln!("FAIL: capability table: {}", why.message());
            // Which fixture and which property, when either is involved. A
            // suite that says only "something is wrong with the suite" is a
            // suite somebody rewrites rather than reads.
            if let Some(flaw) = why.flaw() {
                kprintln!("  flaw          {flaw:?}");
            }
            if let Some(property) = why.property() {
                kprintln!("  property      {property:?}");
            }
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // M3. The other privilege level, and the first thing in this system that is
    // not the kernel. It runs inside a timer window on purpose: the milestone's
    // exit criterion is not that a process runs and not that the timer runs, it
    // is that both are true at once.
    timed_window(&boot, &mut frames, &space, features, clocks);

    // A fault on purpose, when asked for one. This is how the report path is
    // tested: `cargo xtask fault <kind>` boots with the parameter, and the run
    // is expected to end in a dump and a failure exit rather than in `M0 ok`.
    provoke(&boot, features);

    // The two endings a harness has to tell apart from success and from each
    // other, each reachable on purpose. `cargo xtask panic` boots all three.
    deliberate_stop(&boot);

    boot_time(&boot, entered);

    kprintln!("M0 ok");
    arch::x86_64::exit_qemu(arch::x86_64::Exit::Success)
}

/// How long this boot took, when the command line asks for it.
///
/// # Why this is not simply printed
///
/// The boot log is a fixture. Two runs of the same commit produce the same
/// bytes, and that property is asserted — it is what E0-B02 closed on and what
/// every reproduction check since rests on. A duration in it would be a
/// different number on every run, so the log would stop being comparable and
/// the one contract M0 actually makes would be gone.
///
/// This is the same answer `timer=` already gives for the jitter histogram, and
/// the reason is worth stating twice because the temptation is to print it
/// once: a boot log carrying a measurement is a fixture that fails at random.
///
/// The number is nanoseconds from the first instruction of `kmain` to here, on
/// the timestamp counter, converted with the frequency `apic::calibrate`
/// measured. It excludes the loader and the emulator's own start-up, which is
/// the honest boundary: this is what the *kernel* took, and nothing here can
/// see what happened before it was entered.
fn boot_time(boot: &BootInfo, entered: u64) {
    if !boot.has_parameter(b"boottime") {
        return;
    }

    let khz = arch::x86_64::apic::tsc_khz();
    if khz == 0 {
        kprintln!("  boot time     unavailable: the timestamp counter was never calibrated");
        return;
    }

    let ticks = arch::x86_64::read_tsc().saturating_sub(entered);
    // Divide first and scale the remainder, for the reason E0-B08 recorded:
    // `ticks * 1_000_000 / khz` overflows a u64 after about ninety minutes of
    // uptime at 3.4 GHz, and wraps rather than failing. A boot is nowhere near
    // that, and doing the arithmetic the other way here would leave the pattern
    // that is wrong elsewhere sitting in the tree looking correct.
    let micros = ticks / khz;
    let remainder = ticks % khz;
    let nanos = micros.saturating_mul(1_000).saturating_add(remainder * 1_000 / khz);

    // A line the harness parses, deliberately shaped so that a person reading
    // the log knows it is not part of the fixture.
    kprintln!("  boot time     {nanos} ns to M0 (measurement run; not the fixture log)");
}

/// Panic or hang, when the command line asks for one.
///
/// # Why a kernel ships the ability to fail on purpose
///
/// The same argument RFC 0017 makes for the mutation build, and the same one
/// `cargo xtask fault` already rests on: a failure path nothing exercises is a
/// failure path nobody has checked. Three endings have to be distinguishable by
/// a machine — a clean run, a panic, and a run that never ends — and the only
/// way to know they are is to produce all three.
///
/// The third is the one that motivates this. A hang is not a failure the kernel
/// can report, by definition: it is the absence of any report. Whether it is
/// noticed is a property of the *harness* rather than of the kernel, and it
/// cannot be tested without something that actually hangs. Before this, a
/// kernel that stopped making progress would have held a runner until the CI
/// job's own timeout killed it, which is indistinguishable from a slow build.
///
/// Both are behind a boot parameter, absent from every ordinary run, and
/// neither is reachable without one. That is a weaker guarantee than the
/// mutation build's cargo feature and is enough here: this decides nothing and
/// computes nothing, so the worst a mistake can do is stop a boot that asked to
/// be stopped.
fn deliberate_stop(boot: &BootInfo) {
    if boot.has_parameter(b"hang") {
        kprintln!("  hanging       on purpose; the harness is expected to time out");
        // A spin and not a halt. A halted core is one the emulator can see is
        // idle; the fixture is meant to look like work that is not finishing,
        // which is what a livelock looks like and is the harder case for a
        // harness to call.
        loop {
            core::hint::spin_loop();
        }
    }

    if boot.has_parameter(b"panic") {
        // A message with a value in it. A panic that prints only a location
        // proves the handler ran. One that formats a number proves the handler
        // can still reach the formatting machinery — which is the part most
        // likely to be what broke, and the part a real panic message needs.
        panic!(
            "deliberate panic from the boot path, with {} KiB reported usable",
            boot.mem_upper_kib()
        );
    }
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

/// How many ticks the frame takes out of ring 3 before it tells the process it
/// has run long enough.
///
/// Eight, at a kilohertz: eight milliseconds of a process holding the core
/// while the timer keeps its schedule. Small enough to disappear inside the
/// hundred-tick probe an ordinary boot already runs, and large enough that
/// "the timer ran while ring 3 did" is a statement about a schedule rather than
/// about one lucky interrupt.
///
/// A count of *ticks* and not of instructions, which is what keeps the boot log
/// a fixture on machines two orders of magnitude apart in speed. `process` says
/// why at length.
const USER_TICKS: u64 = 8;

/// How long the core that built a process waits for the core running it, in
/// microseconds.
///
/// Five seconds, which is four orders of magnitude past the eight milliseconds
/// the process is meant to take. It is not a timeout on a slow process — the
/// process has its own bound, and it is the one that decides how long ring 3
/// gets — it is the answer to a core that has stopped answering at all, and the
/// only thing on the other side of it is a boot that hangs with no output.
///
/// A count of *time* and not of ticks, unlike everything else about the window,
/// because what is being waited for is a core rather than a schedule.
const PROCESS_MICROS: u64 = 5_000_000;

/// Measure both clocks against the 8254 and say which mechanism will drive the
/// timer.
///
/// Separated from the run it used to be part of, because at M3 the interesting
/// thing inside a timer window stopped being a wait. The calibration has to
/// happen before the hardware `Env` is denominated in it, and the `Env` is
/// reported before the window opens — so the two are no longer one function.
fn calibrate() -> arch::x86_64::apic::Clocks {
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

    clocks
}

/// Open a timer window, run every process this boot has inside it, and report
/// all of it afterwards.
///
/// # The two runs, and why they are still two
///
/// An ordinary boot takes [`PROBE_TICKS`] and prints only things that cannot
/// vary — which mechanism, how many ticks, how many the frame took out of ring
/// 3 — because the boot log is a fixture and two runs of one commit have to
/// match byte for byte. `timer=<seconds>` on the command line is the
/// measurement: it prints the histogram, the frequencies it was denominated in,
/// and everything else that moves. Nothing asserts on that output, and nothing
/// should — `claims/0002-timer-jitter.toml` is where a measurement becomes
/// something anybody is allowed to quote.
///
/// # Why the processes run on another core
///
/// M3's exit criterion was that a process runs, faults deliberately and is
/// killed cleanly *while core 0 holds its jitter bound throughout*. It was met
/// with both happening on one core, which is the strongest version of the claim
/// available with one core and a weaker version of the question: a timer that
/// keeps its schedule across a privilege-level transition is not the same
/// property as a timer that keeps its schedule while another core is busy.
///
/// E0-B10 asks the second question. Core 0 opens this window and does nothing
/// else; the processes are built here, handed to another core, and run there
/// inside timer windows that core opens for itself. The two schedules are
/// independent — separate local APICs, separate deadlines, separate histograms
/// — and neither is a term in the other. What is asserted is what it always
/// was: every tick this core's schedule asked for arrived.
///
/// On a machine with one core the windows are sequential rather than
/// concurrent, because one core cannot hold two. The processes run first, each
/// in a window of its own, and this one opens afterwards. That is a weaker boot
/// and the log says which shape it was.
///
/// # Why two processes
///
/// The first is the component the loader placed in memory, which from E0-B10 is
/// `user/init`: ordinary Rust, compiled and linked separately, with no `unsafe`
/// in it, that the kernel does not contain and cannot see inside. The second is
/// the frame's own adversary, which provokes whatever the command line asked
/// for.
///
/// Running both on every boot is deliberate. It means the second process starts
/// on a core where a first one has already lived and died, so every boot checks
/// what M4 could only assert: that a table cleared between processes does not
/// let the second resolve a handle the first held. The generations are what make
/// that true and `cap::Table::clear_all` is where it is written down; this is
/// where it is exercised.
fn timed_window(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    clocks: arch::x86_64::apic::Clocks,
) {
    // The arithmetic before the machinery, for the same reason `env::self_test`
    // runs before the contract check: a selector layout that is wrong is a
    // `sysret` into whatever descriptor happened to be there, which is not a
    // fault and cannot be debugged after the fact.
    match process::self_test() {
        Ok(()) => kprintln!("  process       layout ok, sysret selectors agree"),
        Err(why) => {
            kprintln!("FAIL: process: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    let component = component(boot);
    match component {
        Some(program) => kprintln!(
            "  init          {} bytes from boot module 1 of {}",
            program.len(),
            boot.modules().len()
        ),
        None => kprintln!("  init          no boot module; only the frame's own program runs"),
    }

    let asked = process::Provoke::chosen(boot);
    let provoke = if asked.needs_no_execute() && !features.nx {
        // Not a failure of the kernel, and not something to pretend was tested.
        // The same answer `fault=nx` gives: say the provocation is unavailable
        // and run the one that needs nothing.
        kprintln!("  no-execute    unavailable on this machine; ring 3 has nothing to provoke");
        process::Provoke::Exit
    } else {
        asked
    };
    kprintln!("  provoking     {}, from ring 3", provoke.label());

    let seconds = boot.parameter_u32(b"timer=");
    let target = match seconds {
        Some(seconds) => u64::from(seconds) * u64::from(TIMER_HZ),
        None => PROBE_TICKS,
    };

    // The core that will hold the processes. Another one where there is another
    // one, and this one where there is not.
    let me = arch::x86_64::current_cpu();
    let worker = if smp::started() > 1 { smp::first_worker() } else { me };
    if worker == me {
        // Only reachable on a single-core machine, and only there does this
        // core need a system-call entry: the transition happens on the core the
        // process runs on, and on every other boot that is not this one.
        // SAFETY: boot processor, once on this core, after `gdt::init` installed
        // every descriptor the selectors written there name, and before anything
        // enters ring 3 on it.
        unsafe { arch::x86_64::ring3::init() };
    }

    // Nothing between here and `stop` prints. A serial port at 115 200 baud
    // spends most of a millisecond on a line, which at a kilohertz is most of a
    // tick interval — so a window that logged what happened inside it would be
    // a measurement of the logging. Both reports are collected and said
    // afterwards.
    let concurrent = worker != me;
    let window = if concurrent {
        // This window is open across the whole of the other core's, which is
        // the property the milestone is about. Interrupts are enabled from
        // here, and that is not incidental either: the core running a process
        // may have to ask this one to forget a mapping, and a core with
        // interrupts disabled cannot answer that.
        //
        // SAFETY: this core was brought up and calibrated, `idt::init` has
        // installed the timer's vector, and interrupts are disabled on entry —
        // `start` enables them and `stop` disables them again.
        match unsafe { arch::x86_64::apic::start(TIMER_HZ, target) } {
            Ok(window) => Some(window),
            Err(why) => {
                kprintln!("FAIL: timer: {}", why.message());
                arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
            }
        }
    } else {
        None
    };

    let first = component.map(|program| {
        let plan = process::Plan {
            program,
            // The expectation of a process that does nothing wrong, which is
            // exactly what this component is written to be. It is the same
            // expectation the frame already had, rather than a second one:
            // `user/init/src/component.rs` says why that matters.
            provoke: process::Provoke::Exit,
            wanted: USER_TICKS,
            hz: TIMER_HZ,
            target,
            cpu: worker,
        };
        run_one(frames, space, features, clocks, plan)
    });

    let plan = process::Plan {
        program: arch::x86_64::probe::program(),
        provoke,
        wanted: USER_TICKS,
        hz: TIMER_HZ,
        target,
        cpu: worker,
    };
    let second = run_one(frames, space, features, clocks, plan);

    let summary = match window {
        Some(window) => {
            // SAFETY: on the core `start` was called on, while its run is still
            // armed.
            let _ = unsafe { arch::x86_64::apic::wait(&window) };
            // SAFETY: as above, once per `start`.
            unsafe { arch::x86_64::apic::stop(&window) }
        }
        // One core: the processes have finished and their windows are closed,
        // so this one has nothing to overlap with and simply runs.
        None => run_window(target),
    };

    if let Some(report) = first {
        kprintln!(
            "  init process  core {}, {} call(s) answered, {} refused, ended with status {}",
            report.cpu,
            report.caps.ok,
            report.caps.refused(),
            match report.death {
                process::Death::Exited(status) => status,
                _ => u64::MAX,
            }
        );
        if let Err(why) = report.verdict(process::Provoke::Exit, USER_TICKS) {
            kprintln!("FAIL: init: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    report(&second, provoke);

    // A short run is a failure however it is dressed up: the timer stopped
    // firing and the histogram is of whatever happened before it did. Since
    // E0-B10 it is also the assertion that a core holding a process at ring 3
    // cost this core's schedule nothing — the window it covers is the one the
    // other core ran inside.
    if summary.ticks != summary.target {
        kprintln!("FAIL: timer stopped after {} of {} ticks", summary.ticks, summary.target);
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    if seconds.is_none() {
        kprintln!(
            "  timer         {} ticks at {} Hz, {}",
            summary.ticks,
            summary.hz,
            if concurrent { "across another core's ring 3" } else { "after ring 3, on one core" }
        );
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
    kprintln!("    from ring 3   {} tick(s) on core {}", second.ticks, second.cpu);
    kprintln!("    missed        {} tick(s) a full period or more late", summary.missed);

    let mut serial = arch::x86_64::serial::Serial;
    summary.late.report(summary.tsc_khz, &mut serial);
    kprintln!();
}

/// The program the loader placed in memory, if it placed one.
///
/// # Why a dropped module is fatal here and not at M1
///
/// `BootInfo` counts the modules it could not keep rather than refusing the
/// handoff, because at M1 nothing depended on a module's contents and the
/// memory they occupy was reserved either way. This is the milestone that
/// depends on one, and a module the kernel did not keep is a module whose
/// memory *was not* reserved — so the frame allocator may already have handed
/// it to somebody. Booting past that means reading a component out of memory
/// something else is writing.
fn component(boot: &BootInfo) -> Option<&'static [u8]> {
    if boot.modules_dropped() > 0 {
        kprintln!(
            "FAIL: the loader placed {} module(s) this kernel did not keep, so their memory \
             was never reserved",
            boot.modules_dropped()
        );
        kprintln!("  raise         arch::x86_64::multiboot::MAX_MODULES");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    let module = *boot.modules().first()?;
    // SAFETY: the direct map is live and `frames` was rebound onto it long
    // before this runs, and every module is in the reserved list — see
    // `reserved_ranges` — so nothing else owns these bytes.
    Some(unsafe { module.bytes() })
}

/// Build one process, hand it to a core, and take its memory back.
///
/// Prints nothing: it runs inside the timer window, and the window is the one
/// thing in this boot that a serial port would measurably disturb.
fn run_one(
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    clocks: arch::x86_64::apic::Clocks,
    plan: process::Plan,
) -> process::Report {
    // SAFETY: boot processor, kernel address space in `CR3`, `frames` rebound
    // onto its direct map, and `plan.cpu` a core that is up and idle. `frames`
    // is not touched again until `reap`.
    let built = unsafe { process::prepare(frames, space, features, plan) };
    let prepared = match built {
        Ok(prepared) => prepared,
        Err(why) => {
            kprintln!("FAIL: process: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    // SAFETY: `plan.cpu` reports ready, everything `process::execute` depends on
    // was put in its shards by `prepare`, and interrupts are enabled unless this
    // is the single-core shape — where they are disabled, which is what the
    // same-core branch of `run_on` requires.
    let ran = unsafe { smp::run_on(plan.cpu, space.root(), clocks.tsc_khz, PROCESS_MICROS) };
    if let Err(cpu) = ran {
        kprintln!("FAIL: core {cpu} did not finish the process it was given");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    // SAFETY: on the core that prepared it, after the core that ran it has
    // reported finished — which is what `run_on` returning `Ok` means.
    match unsafe { process::reap(frames, prepared) } {
        Ok(report) => report,
        Err(why) => {
            kprintln!("FAIL: process: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
}

/// Say what the frame's own program did, and check it did what it was told.
fn report(report: &process::Report, provoke: process::Provoke) {
    kprintln!(
        "  user space    core {}, root {:#018x}, {} kernel slot(s) shared",
        report.cpu,
        report.root,
        report.shared_slots
    );
    kprintln!("  user frames   {} given back, free count unchanged", report.frames);
    kprintln!(
        "  user caps     {} granted, {} call(s) answered, {} refused, {} held at the end",
        report.granted,
        report.caps.ok,
        report.caps.refused(),
        report.held,
    );
    kprintln!(
        "  user process  announced itself, then ran until the frame had taken {USER_TICKS} \
         tick(s) from ring 3"
    );
    match report.death {
        process::Death::Killed { vector, error, address, rip } => kprintln!(
            "  user death    exception {vector} at {address:#018x}, error {error:#x}, \
             rip {rip:#018x} — killed"
        ),
        process::Death::Exited(status) => kprintln!(
            "  user death    asked to end with status {status}, after {} refused call(s)",
            report.refused
        ),
        // `process::reap` turns this into `Error::NoDeath` before it returns, so
        // reaching it means the two paths disagree about disagreeing.
        process::Death::Running => {
            kprintln!("FAIL: the process ended and the frame did not notice");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // The provocation has to have provoked. A protection that did not fire is
    // not a smaller result than a fault — it is the opposite result, and the
    // one this whole milestone exists to rule out.
    if let Err(why) = report.verdict(provoke, USER_TICKS) {
        kprintln!("FAIL: {why}");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }
}

/// Open this core's timer window, wait it out and close it.
///
/// Only the single-core shape uses it, and only because there the window has
/// nothing to overlap with: the processes have already run and finished, so
/// there is nothing to do inside it but wait.
fn run_window(target: u64) -> arch::x86_64::apic::Summary {
    // SAFETY: this core was brought up and calibrated, `idt::init` has installed
    // the timer's vector, and interrupts are disabled on entry —
    // `process::execute` left them that way when it stopped its own window.
    let window = match unsafe { arch::x86_64::apic::start(TIMER_HZ, target) } {
        Ok(window) => window,
        Err(why) => {
            kprintln!("FAIL: timer: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };
    // SAFETY: on the core `start` was called on, while its run is still armed.
    let _ = unsafe { arch::x86_64::apic::wait(&window) };
    // SAFETY: as above, once per `start`.
    unsafe { arch::x86_64::apic::stop(&window) }
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
        // Noted here and fatal later, in `component`, which is where a module's
        // contents are first depended on. Reporting it twice is deliberate: the
        // number belongs in the memory report beside the modules it is about,
        // and the refusal belongs where the dependency is — an unreserved
        // module is memory handed out from under its owner, and the sentence
        // that says so should be next to the code that would read it.
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
    // `Panic` and not `Failure`. A kernel that reports a failed assertion and a
    // kernel that panicked are different events, and the exit code is the only
    // channel that survives the panic having interrupted the log.
    arch::x86_64::exit_qemu(arch::x86_64::Exit::Panic)
}
