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

pub mod admit;
pub mod arch;
pub mod blk;
pub mod cap;
pub mod churn;
pub mod component;
pub mod doorbell;
pub mod env;
// The third driver's supervisor. Beside `blk` and `net` and deliberately not
// merged with them; `kernel/src/gpu.rs` says why and RFC 0054 argues it.
pub mod gpu;
pub mod iommu;
pub mod jitter;
pub mod mem;
pub mod net;
pub mod percpu;
pub mod process;
pub mod ring;
pub mod runtime;
pub mod smp;
pub mod state;

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

    // The loader's account of what is ordinary memory, copied out **here** and
    // not where it is used.
    //
    // `BootInfo::regions` walks the loader's map through the boot stub's
    // identity mapping, which stops existing at `paging::activate` a hundred
    // lines below. Everything else `BootInfo` carries is a copy — the command
    // line is an array in the struct — and this was the one thing left as a
    // pointer, because until E1-B01 nothing wanted the map after the switch.
    // Reading it inside `discover` is a page fault at ring 0, and it is one
    // this file paid for once already.
    //
    // What wants it is `acpi::Ram`: a physical address firmware describes as
    // device registers is about to become an uncacheable writable mapping, and
    // the loader's map is the only second opinion available on whether that
    // address is memory somebody else owns.
    let mut ram = arch::x86_64::acpi::Ram::new();
    for region in boot.regions() {
        if region.kind == RegionKind::Usable {
            ram.add(region.base, region.len);
        }
    }

    // The determinism substrate is live from the first line of kernel code that
    // observes anything. Nothing below may read the clock directly.
    let mut seeded = SeededEnv::new(SEED, 100);
    kprintln!("  seed          {SEED:#018x}");

    let mut mixed: u64 = 0;
    for _ in 0..8 {
        mixed ^= seeded.next_u64();
    }

    // A deliberate defect, off by default, and the one E0-P02 is built around.
    //
    // Every other check in this tree would pass with this on. The kernel boots,
    // every assertion holds, `M0 ok` is printed and the exit code is 33 — the
    // only thing that changes is that two runs of the same commit no longer
    // agree, and nothing except a reproduction check looks at that. RFC 0017
    // argues why a defect like this lives in the shipped source behind a feature
    // rather than in a patch somebody applies; this is the second instance, and
    // the argument is the same one.
    #[cfg(feature = "mutate-unseeded-time")]
    {
        mixed ^= arch::x86_64::read_tsc();
    }

    kprintln!("  env digest    {mixed:#018x}");
    kprintln!("  env clock     {} ns", seeded.now().as_nanos());

    // The same seed must always produce the same digest. This is the weakest
    // possible form of the reproducibility contract, asserted at boot so that a
    // regression in the substrate is caught on the very next run rather than
    // months later when the simulator stops reproducing.
    //
    // The whole block is compiled out under the reproduction defect, not just
    // the comparison. That defect must make two runs *differ*, not make one run
    // fail — a boot that goes red is caught by every check in the tree, and the
    // question E0-P02 asks is whether anything catches a boot that goes green
    // twice with two different answers. Compiling out only the `if` would leave
    // the computation behind as an unused-variable warning, and a defect build
    // that does not compile cleanly is a defect build somebody will disable.
    #[cfg(not(feature = "mutate-unseeded-time"))]
    {
        let mut check = SeededEnv::new(SEED, 100);
        let mut expect: u64 = 0;
        for _ in 0..8 {
            expect ^= check.next_u64();
        }
        if expect != mixed {
            kprintln!("FAIL: determinism substrate is not reproducible");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
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

    let allocator = match mem::self_test(&mut frames, &mut seeded) {
        Ok(report) => report,
        Err(why) => {
            kprintln!("FAIL: frame allocator: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };
    // `orders 0..=N` is the largest order this machine could actually serve,
    // asked for rather than assumed: order 18 is a gibibyte, and a boot
    // fixture with 128 MiB has no gibibyte to hand out. A line that claimed 18
    // on a machine that cannot reach it would be the kind of number this tree
    // registers claims to prevent. `cargo xtask orders` is the command that
    // reads this number on a machine that does have one, and requires 18.
    kprintln!(
        "  frame alloc   ok — orders 0..={}, {} split, {} merged",
        allocator.largest,
        allocator.splits,
        allocator.merges
    );
    // The exit criterion of E1-B12, as a line rather than an assertion. The
    // first number is what the allocation path cost in cross-core traffic
    // while it was being driven the way a running system drives it; the last
    // is what the self-test had to provoke to prove the counter can move at
    // all.
    kprintln!(
        "  frame shards  {} shards, {} cross-core on the hot path, {} refill(s), {} forced",
        percpu::MAX_CPUS,
        allocator.hot_remote,
        allocator.refills,
        allocator.steals
    );

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

    // E1-B01. What a *device* may address, which is the one protection in this
    // kernel the processor's page tables have nothing to do with. A driver at
    // ring 3 that can program a bus master can address every byte of the
    // machine, and until this stage runs the whole capability system it is
    // running inside is decoration.
    //
    // A machine with no remapping unit takes the other branch, says so, and
    // carries on: `-machine pc` is that machine, and RFC 0031 is why the
    // default is no longer.
    //
    // SAFETY: the boot processor, once, with the kernel's address space active,
    // `frames` rebound onto its direct map, and no device in this kernel yet
    // performing DMA — which is the whole list `vtd::Unit::open` asks for.
    let mut remapping = unsafe { discover(&ram, &mut frames, &space, features) };

    // M4. The five properties, against a real table and against five tables
    // broken on purpose — one per property. This runs before ring 3 exists on
    // this boot, and that order is the point: the negative suite from ring 3
    // can only report that a hostile handle was refused, and it cannot report
    // that a table which *stopped* refusing would have been noticed. That is
    // what the flawed fixtures are for, and a suite whose checks cannot fail is
    // a suite nobody has tested.
    //
    // Since E1-B13 it runs twice, at two sizes: once on tables holding only
    // what the frame gave them and once on tables that have bought a page out
    // of an `Untyped` — which is why it needs the allocator, and why the flawed
    // count is ten rather than five.
    //
    // Since E1-B05 it also checks the storage under the authority model: the
    // notice packed into a slot's type byte, the watermark that moves back, and
    // the name given up along with its descendants. None of those is reachable
    // through the trait the five properties run over, so a line that reported
    // only the five would have been reporting on half the file.
    match cap::properties::self_test(&mut frames) {
        Ok(caught) => kprintln!(
            "  capabilities  {} free slots, {} more per page bought, {} properties and {} \
             storage checks hold, {caught} flawed tables caught",
            cap::TABLE_SLOTS,
            cap::SLOTS_PER_PAGE,
            cap::properties::Property::all().len(),
            cap::properties::STORAGE_CHECKS,
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

    // RFC 0013, and E0-B14. Published *before* the subsystems that fill it,
    // because a node names a live word rather than a value copied in later —
    // the tree has to exist for the store to have somewhere to go.
    //
    // Nothing time-derived goes in, deliberately. The boot log is what
    // `cargo xtask trace` hashes, and a tick count in it would make two runs of
    // one commit disagree for a reason that has nothing to do with the kernel.
    // *Reversal:* a boot log that is no longer the reproduction artefact.
    let tree = match state::Tree::publish(&mut frames) {
        Ok(tree) => tree,
        Err(why) => {
            kprintln!("FAIL: the state tree: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };
    tree.set(state::node::TOPOLOGY_STARTED, smp::started() as u64);
    tree.set(state::node::CAPS_SLOTS, cap::TABLE_SLOTS as u64);

    // M5, the first piece of it. One channel laid out by `f_abi::layout` in a
    // real frame, a batch of four published with one store, and both opcodes
    // answered — including the one this build does not implement, which is the
    // half of a protocol that is easy to leave untested.
    //
    // Before ring 3 for the same reason the capability suite is: this runs
    // against the kernel's own memory, where a failure can be reported. A ring
    // that only ever ran with a process at the other end could not tell a
    // service that refuses a bad entry from one that never saw it.
    match ring::self_test(&mut frames, hardware.now().as_nanos()) {
        Ok(report) => {
            kprintln!(
                "  ring          {} entries in {} B, {} B arena, two ends at ABI v{}, \
                 {} published with one store, {} refused, forged slot caught, \
                 hostile header refused",
                report.entries,
                report.bytes,
                report.arena,
                report.version,
                report.drained.executed,
                report.drained.refused,
            );
            // A second line rather than a longer one. The doorbell answers a
            // different question — whether an interrupt arrived — and a reader
            // looking for a suppression figure should not have to find it
            // inside a sentence about arena sizes.
            kprintln!(
                "  doorbell      {:?}, {} delivered, {} per 1000 operations, \
                 a draining consumer was not rung",
                report.path,
                report.doorbells,
                report.per_thousand,
            );
            // Into the tree at the one place these numbers are established. Two
            // homes for a count is the second copy that can disagree with the
            // first, which is what RFC 0013 refuses.
            tree.set(state::node::RING_EXECUTED, u64::from(report.drained.executed));
            tree.set(state::node::RING_REFUSED, u64::from(report.drained.refused));
        }
        Err(why) => {
            kprintln!("FAIL: the frame's ring: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // E1-B01's exit, and the only thing in this boot that makes a real device
    // read a real descriptor. Behind a boot parameter, like every other
    // provocation in this file: an ordinary boot has no device to provoke and
    // no business turning one on.
    //
    // Before the numbers below rather than after them, and that placement is a
    // fix rather than a preference: everything this stage does — a domain taken,
    // frames spent and given back, and a fault recorded — is something the tree
    // publishes, and a tree rendered first would publish the state of a machine
    // this boot had not finished being.
    let provoked = dma_provocation(&boot, &mut frames, &space, features, remapping.as_mut());

    // E1-B14. What it costs to take a translation back, at the rate the
    // datapath produces one. Behind its own parameter for the reason every
    // stage here is: it takes a domain and a quarter of a mebibyte, and an
    // ordinary boot has no business doing either.
    //
    // Beside `dma_provocation` rather than inside it, because it is asking a
    // different question of the same unit: that one asks whether a device is
    // confined, this one asks what confining it costs to undo. Before the tree
    // is rendered, for the reason recorded above.
    churn_measurement(&boot, &mut frames, remapping.as_mut());

    // E1-B02. The datapath, with the driver outside the frame: a component
    // whose crate forbids `unsafe` brings a real device up through granted
    // register windows, a client registers a buffer set and writes a sector
    // through a ring, reads it back, and compares the bytes — and the driver's
    // copy counter is required to be zero while the counter beside it, moved by
    // the same function on purpose, is required not to be.
    //
    // Beside `dma_provocation` and behind its own parameter for the same
    // reason: an ordinary boot has no device to drive and no business turning
    // one on. Before the tree is rendered, because everything it does — a
    // domain taken, frames spent and given back, a fault recorded — is
    // something the tree publishes.
    let datapath = blk_datapath(
        &boot,
        &mut frames,
        &space,
        features,
        remapping.as_mut(),
        clocks,
        tree.physical(),
    );

    // E1-B08. A component that holds a core and schedules its own work inside
    // it, with the frame counting what crossed. Behind its own parameter, like
    // the two stages above and for a sharper version of their reason: it enters
    // ring 3 and takes timer ticks there, and a tick count differs between a
    // fast host and a slow one — so a default boot that ran it would stop being
    // the fixture `cargo xtask trace` hashes.
    //
    // Here rather than beside `timed_window`, which is where a reader would
    // look for it, because everything it produces is something the tree
    // publishes and a tree rendered first would publish the state of a machine
    // this boot had not finished being. The same fix `dma_provocation` records
    // above.
    let scheduled =
        runtime_demonstration(&boot, &mut frames, &space, features, clocks, tree.physical());

    // E1-B03. The same shape as the stage above it, with a second driver in it:
    // a component whose crate forbids `unsafe` brings a real network device up
    // through granted register windows, a client registers a buffer set, posts
    // one receive buffer through a ring, puts a hand-formed frame on the link,
    // and requires the answer to land in the registered buffer.
    //
    // Behind its own parameter, like every other provocation in this file, and
    // for the sharper version of their reason: an ordinary boot has no network
    // device at all — `MACHINE` passes `-net none` — so this stage is the only
    // one that adds a bus master the default machine does not have.
    //
    // Before the tree is rendered, because everything it does is something the
    // tree publishes and a tree rendered first would publish the state of a
    // machine this boot had not finished being.
    let packets = net_datapath(
        &boot,
        &mut frames,
        &space,
        features,
        remapping.as_mut(),
        clocks,
        tree.physical(),
    );
    let _ = packets;

    // E1-B04. The same shape again with a third driver in it, and the first one
    // whose result is not inside this machine: a component whose crate forbids
    // `unsafe` brings a real display controller up through granted register
    // windows, a client registers a buffer set, draws a pattern into one buffer
    // and submits one entry, and the driver turns that into six display commands
    // that put the client's pixels on scanout zero.
    //
    // Behind its own parameter like every other provocation in this file, and
    // for a reason none of the others has: this stage **holds the machine still**
    // at the end of itself, so that `cargo xtask gpu` can capture the emulator's
    // framebuffer while the picture is on it. A default boot that ran this would
    // wait for a harness that is not there.
    let picture = gpu_datapath(
        &boot,
        &mut frames,
        &space,
        features,
        remapping.as_mut(),
        clocks,
        tree.physical(),
    );
    let _ = picture;

    // E1-B07. What this machine can reserve, asked of the machine rather than
    // assumed about it. Behind its own parameter for the fixture's sake, like
    // the three stages above, and printing nothing on a default boot.
    admission_demonstration(&boot);

    // Last of the frame's own numbers, because the allocator is still handing
    // out frames until the line above. The self-test is what says the hash
    // works: two readings with nothing in between must agree, and a reading
    // after a deliberate change must not — a hash over bytes nothing writes
    // agrees with itself forever, which is indistinguishable from one that
    // works. Before the first process, because nothing may map a tree this
    // kernel has not yet agreed with.
    tree.set(state::node::MEMORY_TOTAL, frames.total_count());
    tree.set(state::node::MEMORY_FREE, frames.free_count());
    // The three allocation paths, published rather than printed once: a reader
    // that maps this tree later can see whether allocation is still local
    // without a boot log to compare against. RFC 0027.
    //
    // A fourth node beside them, because `remote` on its own is a number a
    // reader would misread. The self-test provokes the remote path on purpose
    // every boot — a counter nothing can move is not a counter — so `remote`
    // is never zero and the part that answers the exit criterion is the
    // difference. The boot log takes that difference already; the tree
    // publishes both halves and lets a reader take it.
    tree.set(state::node::MEMORY_SERVED, frames.served_count());
    tree.set(state::node::MEMORY_REFILL, frames.refill_count());
    tree.set(state::node::MEMORY_REMOTE, frames.remote_count());
    tree.set(state::node::MEMORY_FORCED, allocator.steals);
    // The remapping unit's three numbers, *after* the provocation above and not
    // before it. That ordering is the whole of what makes them instruments: a
    // fault count written before the only code on this boot that can produce a
    // fault is a node whose value is a constant, and a gauge of domains handed
    // out, read before anything takes one, is the same defect wearing a
    // different unit. Both were exactly that until they were read against the
    // boot log, where the tree printed `faults = 0` twelve lines above the
    // fault it was meant to be counting.
    //
    // `faults` is the one worth a sentence: it is a counter nothing in a
    // healthy machine moves, so a reader watching it rise is watching a device
    // try to address memory nobody gave it — and that is a different event from
    // every other counter in this tree, which all count work being done.
    if let Some(found) = remapping.as_ref() {
        tree.set(state::node::IOMMU_DOMAINS, u64::from(found.unit.domains()));
        tree.set(state::node::IOMMU_USED, u64::from(found.unit.domains_used()));
        // Both provocations' faults, summed rather than kept apart, because
        // this node is the unit's own record of transactions it refused and the
        // unit does not care which stage produced one. Only one of the two runs
        // on any boot — they are different parameters — so the sum is a sum of
        // one number and a zero.
        let refused =
            u64::from(provoked) + datapath.as_ref().map_or(0, |report| u64::from(report.faults));
        tree.set(state::node::IOMMU_FAULTS, refused);
    }
    // The datapath's four numbers, published at the one place they are
    // established. RFC 0013: a node names a live word rather than a value
    // copied in later, and two homes for a count is the second copy that can
    // disagree with the first.
    if let Some(report) = datapath.as_ref() {
        tree.set(state::node::BLK_SERVED, u64::from(report.counters.served));
        tree.set(state::node::BLK_BYTES, report.counters.bytes);
        tree.set(state::node::BLK_COPIES, report.counters.copies);
        tree.set(state::node::BLK_PROVOKED, report.counters.provoked);
    }
    // The runtime's five crossings, published where they were established. Zero
    // on a boot that ran none, which is what makes the default boot's log the
    // same bytes it was before this landed — and a `runtime=` boot is not the
    // fixture, for the reason `runtime_demonstration` gives.
    if let Some(report) = scheduled.as_ref() {
        tree.set(state::node::RUNTIME_HOT, report.entries.on_the_hot_path());
        tree.set(state::node::RUNTIME_PROVOKED, u64::from(report.tally.provoked));
        tree.set(state::node::RUNTIME_BOUNDARY, report.entries.boundary);
        tree.set(state::node::RUNTIME_TICKS, report.entries.ticks);
        tree.set(state::node::RUNTIME_INTERRUPTS, report.entries.interrupts);
        // Zero rather than what the field holds, on a run that never adopted a
        // ring: there `completed` carries the refusal's domain, and a node that
        // published it would be publishing an `f_abi::error` under a name that
        // says work items. `f_store::report::refusal` is where that lives.
        tree.set(
            state::node::RUNTIME_WORK,
            if f_store::report::refusal_of(&report.tally).is_some() {
                0
            } else {
                u64::from(report.tally.completed)
            },
        );
    }
    match tree.self_test() {
        Ok(hash) => kprintln!(
            "  state tree    {} nodes, snapshot {hash:#018x}, stable across a re-read",
            state::NODES
        ),
        Err(why) => {
            kprintln!("FAIL: the state tree: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
    tree.render();

    // E1-B05. The component lifecycle, end to end, against real memory: a place
    // built from a manifest the loader carried, a component spawned into it, a
    // client connected, the component killed, the place refilled under its
    // declared policy, and the client's connect pending across the gap and
    // resuming at the higher epoch.
    //
    // Before the timer window and not inside it, for the reason `timed_window`
    // gives about its own contents: this builds address spaces and writes
    // serial lines, and a window that logged what happened inside it would be a
    // measurement of the logging. Nothing here is a measurement — every number
    // it prints is a count — so it has no window to be inside.
    //
    // The tick count the restart budget's window is measured against is read
    // here, from the hardware `Env`, and converted once. RFC 0004 permits no
    // other route to a clock, and RFC 0008 states the window in timer ticks
    // because a supervisor compares it against a count the frame keeps rather
    // than against a duration. Only the *epoch* comes from the machine: the
    // demonstration advances its own count by the backoff it was told to wait,
    // which is what a supervisor does, so nothing it prints moves between a
    // fast host and a slow one.
    let now = hardware.now().as_nanos() / (1_000_000_000 / u64::from(TIMER_HZ));
    // SAFETY: the boot processor, once, with the kernel's address space in
    // `CR3`, `frames` rebound onto its direct map, and no process running. The
    // direct map covers every module: `reserved_ranges` put them all in the
    // reserved list before the allocator was populated.
    match unsafe { component::demonstrate(&mut frames, &space, features, &boot, now) } {
        Ok(report) => kprintln!(
            "  supervisor    ok — {} place(s), {} spawn(s), {} fault(s), {} restart(s), \
             {} resumed, {} client(s) lost, {} probe(s) refused, {} retired, \
             {} need(s) bound to nothing",
            report.places,
            report.spawns,
            report.faults,
            report.restarts,
            report.resumed,
            report.lost,
            report.probed,
            report.retired,
            report.unbound,
        ),
        // A machine that carried no component file is not a broken machine.
        // `docs/booting-on-hardware.md` installs one module and E0-P18 landed a
        // kernel that boots on metal from exactly that, so a demonstration the
        // milestone does not require must not be the thing that stops it. The
        // same shape `discover` uses for a machine with no DMAR, and for the
        // same reason: a boot log line is what a machine missing something
        // optional earns, and an exit is what a machine that has it and got it
        // wrong earns. Every other `Failure` below is the second case.
        Err(component::Failure::NoComponent) => {
            kprintln!("  supervisor    no component file among the boot modules; no place to fill");
        }
        Err(why) => {
            kprintln!("FAIL: the component lifecycle: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // M3. The other privilege level, and the first thing in this system that is
    // not the kernel. It runs inside a timer window on purpose: the milestone's
    // exit criterion is not that a process runs and not that the timer runs, it
    // is that both are true at once.
    timed_window(&boot, &mut frames, &space, features, clocks, tree.physical());

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
    tree: u64,
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
            tree,
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
        tree,
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

/// Everything E1-B01 found: the unit, and the machine it was found on.
///
/// Held by value in `kmain` for the length of the boot, because the unit owns
/// the root table every device's translation hangs from — a unit dropped while
/// translation is enabled would be a machine whose devices walk tables nothing
/// remembers the address of.
struct Remapping {
    /// The unit itself.
    unit: iommu::Unit,
    /// Where configuration space is, so a provocation can reach a function
    /// again without re-reading `MCFG`.
    window: arch::x86_64::pci::Space,
    /// What answered on the bus.
    survey: arch::x86_64::pci::Survey,
}

/// Find the machine's remapping unit, program it, and turn translation on.
///
/// # Why every failure here is a line rather than an exit
///
/// A machine with no ACPI, no configuration-space window or no `DMAR` is a
/// machine this kernel runs on with one protection fewer, and there are a great
/// many of them — every emulator default before RFC 0031, and every machine
/// whose firmware has the unit switched off. A kernel that refused to boot on
/// those would be a kernel that boots on strictly fewer machines than one with
/// no IOMMU support at all, which is the wrong direction for a change whose
/// whole purpose is a protection.
///
/// What is *not* tolerated is a unit that answers and then does not do what it
/// was told: [`vtd::Unit::enable`](arch::x86_64::vtd::Unit::enable) failing
/// after the tables are built is the frame having programmed a device wrong,
/// and it stops the boot.
///
/// # Safety
///
/// Boot processor, once, with the kernel's address space active and `frames`
/// rebound onto its direct map.
unsafe fn discover(
    ram: &arch::x86_64::acpi::Ram,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
) -> Option<Remapping> {
    use arch::x86_64::{acpi, pci, vtd};

    // SAFETY: the caller's guarantee, which is exactly what `Phys::new` asks
    // for.
    let phys = unsafe { acpi::Phys::new(frames, space.direct_limit()) };

    let root = match acpi::root(&phys) {
        Ok(root) => root,
        Err(why) => {
            kprintln!("  acpi          none: {}", why.message());
            return None;
        }
    };

    let ecam = match acpi::ecam(&phys, &root, ram) {
        Ok(ecam) => ecam,
        Err(why) => {
            kprintln!("  acpi          revision {}, no mcfg: {}", root.revision, why.message());
            return None;
        }
    };
    let dmar = match acpi::dmar(&phys, &root, ram) {
        Ok(dmar) => dmar,
        Err(why) => {
            kprintln!("  acpi          revision {}, no dmar: {}", root.revision, why.message());
            return None;
        }
    };
    kprintln!(
        "  acpi          revision {}, mcfg at {:#018x} buses {}..={}, dmar {} unit(s) of {} \
         structure(s), {}-bit addressing",
        root.revision,
        ecam.base,
        ecam.start_bus,
        ecam.end_bus,
        dmar.units,
        dmar.structures,
        dmar.host_address_width,
    );

    let window = pci::Space::new(ecam);
    // SAFETY: the caller's guarantee, passed down.
    let survey = match unsafe { pci::survey(frames, space, features, &window) } {
        Ok(survey) => survey,
        Err(why) => {
            kprintln!("FAIL: configuration space: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };
    kprintln!(
        "  pci           {} function(s) on {} bus(es), {} kept, {} page(s) of window mapped",
        survey.seen,
        survey.buses,
        survey.functions().len(),
        survey.pages,
    );

    // SAFETY: the caller's guarantee, and `dmar.unit` came from a table whose
    // checksum was checked before its register base was believed.
    let mut unit = match unsafe { vtd::Unit::open(frames, space, features, &dmar) } {
        Ok(unit) => unit,
        Err(why) => {
            kprintln!("  iommu         none: {}", why.message());
            return None;
        }
    };
    let found = iommu::Found::of(&unit);
    kprintln!(
        "  iommu         vt-d at {:#018x}, {}-bit in {} levels, {} domains, {}, drhd flags {:#04x}",
        dmar.unit.register_base,
        found.width,
        found.levels,
        found.domains,
        if found.caching_mode { "caching mode" } else { "no caching mode" },
        dmar.unit.flags,
    );
    kprintln!("  iommu caps    cap {:#018x}, ecap {:#018x}", found.capability, found.extended);
    // Which of the two ways the unit is being kept in step with what this
    // kernel wrote. Printed rather than left to be derived from `ecap` above,
    // because a build that flushed nothing would produce an identical boot on
    // this emulator — QEMU reads guest memory directly and has no cache to be
    // behind — so the log line is the only place the choice is visible at all.
    kprintln!(
        "  iommu walks   {}",
        if found.coherent {
            "coherent: the unit snoops this kernel's writes to its tables"
        } else {
            "not coherent: every table entry is flushed by hand before the invalidation"
        }
    );

    // SAFETY: nothing in this kernel drives a device that performs DMA, so
    // there is no transfer in flight for translation to interrupt. `vtd`'s own
    // comment states the reversal — firmware that leaves a device transferring.
    if let Err(why) = unsafe { unit.enable() } {
        kprintln!("FAIL: the remapping unit: {}", why.message());
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }
    // Read back rather than assumed, for the reason `apic::init` reads its
    // spurious register back: a status bit that was written and not checked is
    // a protection that is intended rather than on.
    if !unit.enabled() {
        kprintln!("FAIL: the remapping unit accepted the command and did not enable");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }
    kprintln!("  iommu on      translation enabled; a device with no domain now faults");

    Some(Remapping { unit, window, survey })
}

/// Make a real device read a real descriptor, and require the right thing to
/// happen to it.
///
/// This is E1-B01's exit criterion, and the two halves are the whole of it:
/// `dma=outside` points a descriptor at a page the device's domain does not
/// translate and requires the transaction to be refused *and recorded*, and
/// `dma=inside` points it at a page the domain does translate and requires the
/// transfer to land. The second exists because without it the first passes on a
/// device that was never started, which is the same argument `mutate` makes
/// about a boot that goes red for the wrong reason.
///
/// The verdict is the kernel's rather than the harness's, exactly as `user` and
/// `cap` already are: the kernel knows what it asked for and what a pass looks
/// like, and a harness that only read an exit code could not tell a refused
/// transfer from a device that never answered.
///
/// Answers how many faults the unit recorded, so that the caller can publish it
/// rather than this reaching into the state tree from underneath. A boot with
/// no provocation answers zero, which is the same number and a different claim
/// — and the difference is visible in the log, where a boot that provoked
/// nothing prints no `dma` line at all.
fn dma_provocation(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    remapping: Option<&mut Remapping>,
) -> u32 {
    let inside = if boot.has_parameter(b"dma=inside") {
        true
    } else if boot.has_parameter(b"dma=outside") {
        false
    } else {
        return 0;
    };

    let Some(found) = remapping else {
        kprintln!("FAIL: dma provocation asked for on a machine with no remapping unit");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    kprintln!(
        "  provoking     a device transfer with its buffer {} the grant",
        if inside { "inside" } else { "outside" }
    );

    // SAFETY: the kernel's address space is active, `frames` is rebound onto
    // its direct map, translation is enabled, and nothing else in this kernel
    // drives the device this finds — the frame has no block driver, which is
    // why E1-B02 is a later task.
    let outcome = unsafe {
        arch::x86_64::dma::provoke(
            frames,
            space,
            features,
            &mut found.unit,
            &found.window,
            &found.survey,
            inside,
        )
    };

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(why) => {
            // A provocation that could not be arranged is not a provocation
            // that was survived, and reporting it as a pass is how this whole
            // check would come to mean nothing.
            kprintln!("FAIL: dma provocation: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    kprintln!(
        "  dma target    {:#018x}, requester {:#06x}",
        outcome.target,
        outcome.bdf.source_id()
    );
    // What the interface the three driver tasks build on answered on the way
    // in. Printed rather than only asserted, because these two are the half of
    // the exit that is about authority rather than about hardware: the frame
    // refused to give a device a page whose capability carries no right to hand
    // it on, and its own page walk agrees with the tables the unit walks.
    kprintln!(
        "  dma grant     a capability without GRANT was {}; the domain's walk {} the buffer",
        if outcome.checks.refused_without_grant { "refused" } else { "ACCEPTED" },
        if outcome.checks.reaches_data { "reaches" } else { "does not reach" },
    );
    kprintln!(
        "  dma result    {}, status {:#04x}, buffer {}",
        if outcome.completed { "completed" } else { "no completion" },
        outcome.status,
        if outcome.landed { "written" } else { "untouched" },
    );
    match outcome.fault {
        Some(fault) => kprintln!(
            "  dma fault     requester {:#06x} {} {:#018x}, reason {:#04x} — {} record(s)",
            fault.source,
            if fault.read { "read" } else { "wrote" },
            fault.address,
            fault.reason,
            outcome.faults,
        ),
        None => kprintln!("  dma fault     none recorded"),
    }

    // A device's own completion is not evidence that bytes moved, and this is
    // where that stops being a slogan. The emulator's block device answers a
    // refused transfer with a *successful* status: the request completed as far
    // as the device is concerned, and the write went nowhere. So the refused
    // run is judged on what the unit recorded and on what is in the buffer, and
    // the device's opinion of itself is printed rather than believed.
    //
    // E1-B02 inherits this: a driver that treated a completion as proof its
    // buffer was filled would be wrong here, on this emulator, today.
    if !inside && outcome.completed {
        kprintln!(
            "  dma note      the device called a refused transfer a success; a completion \
             is not evidence that bytes moved"
        );
    }

    // The verdict. Both halves are asserted here rather than in the harness for
    // the reason `process::Report::verdict` gives: a protection that did not
    // fire is not a smaller result than a fault, it is the opposite result.
    let verdict = if inside {
        if !outcome.completed {
            Err("the device never completed a transfer into memory it was granted")
        } else if !outcome.landed {
            Err("the device completed and wrote nothing, so nothing was transferred")
        } else if outcome.faults != 0 {
            Err("the device faulted on memory it was granted")
        } else {
            Ok(())
        }
    } else if outcome.faults == 0 {
        Err("the device addressed memory outside its grant and the unit recorded nothing")
    } else if outcome.landed {
        Err("the unit recorded a fault and the transfer landed anyway, which is a corruption")
    } else {
        Ok(())
    };

    match verdict {
        Ok(()) => kprintln!(
            "  dma verdict   {}",
            if inside {
                "a granted transfer landed"
            } else {
                "the attempt was a fault, not a corruption"
            }
        ),
        Err(why) => {
            kprintln!("FAIL: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
    outcome.faults
}

/// What an unmap costs under churn, both ways, in one boot.
///
/// `E1-B14`. The task permits two outcomes and demands a number either way, so
/// this stage's job is to produce the number rather than to defend a design:
/// it drives the two churn sources the datapath actually has — a client cycling
/// its registered buffers, and a driver restart retiring a component's whole
/// grant — and reports what each cost under each invalidation policy.
///
/// # What is asserted here and what is only printed
///
/// The counts are printed, because they are the result. What is *asserted* is
/// the set of things that would let a run report a flattering result while
/// nothing was measured:
///
/// - both halves retired the same number of sets, so the improvement is a ratio
///   between two runs of one workload rather than between two workloads;
/// - both cleared the same number of pages, for the same reason;
/// - a set spanned more than one page, because a one-page set makes a batched
///   unmap and an unbatched one the same run and the ratio would be 1 while the
///   code was wrong in either direction;
/// - the control's invalidations equal its pages and the candidate's equal its
///   requests, which is what each policy *means* — a build where the policy
///   argument had stopped being read would report one number twice and pass
///   every other check here.
///
/// The shootdown counters are printed as a delta and a total. The delta is the
/// finding — zero — and the total is what makes the zero worth reading, for the
/// reason `state.rs` gives about every counter beside it: a number nothing in a
/// boot can move is indistinguishable from a number that does not work.
fn churn_measurement(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    remapping: Option<&mut Remapping>,
) {
    if !boot.has_parameter(b"churn=unmap") {
        return;
    }

    let Some(found) = remapping else {
        kprintln!("FAIL: the unmap churn was asked for on a machine with no remapping unit");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    // The control first and the candidate second, so that a reader following
    // the log reads the cost before the saving. Both against the same unit, in
    // fresh domains of their own — `churn::run` takes and releases one per half
    // — so neither inherits the other's tables.
    let mut measured = [churn::Counts::default(); 2];
    for (index, when) in
        [iommu::Invalidation::PerPage, iommu::Invalidation::PerRequest].into_iter().enumerate()
    {
        // SAFETY: the kernel's address space is active, `frames` is rebound onto
        // its direct map, and the unit is one this boot programmed and enabled.
        match unsafe { churn::run(frames, &mut found.unit, when) } {
            Ok(counts) => {
                if let Some(slot) = measured.get_mut(index) {
                    *slot = counts;
                }
            }
            Err(why) => {
                // A workload that could not be arranged is not a workload that
                // reported a small number, which is `dma_provocation`'s rule
                // one stage up and the reason this ends the boot.
                kprintln!("FAIL: unmap churn: {}", why.message());
                arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
            }
        }
    }

    let [control, batched] = measured;
    let (shot_pages, shot_ipis) = smp::shootdowns();

    kprintln!(
        "  churn work    {} set(s) registered and {} retired per half; {} page(s) per set",
        control.registered,
        control.retired,
        control.pages.checked_div(control.requests).unwrap_or(0)
    );
    kprintln!(
        "  churn perpage {} unmap request(s), {} page(s), {} invalidation(s), {} round trip(s)",
        control.requests,
        control.pages,
        control.invalidations,
        control.round_trips()
    );
    kprintln!(
        "  churn batched {} unmap request(s), {} page(s), {} invalidation(s), {} round trip(s)",
        batched.requests,
        batched.pages,
        batched.invalidations,
        batched.round_trips()
    );
    kprintln!(
        "  churn saved   {} of {} round trip(s) — {}% of what one page at a time cost",
        control.round_trips().saturating_sub(batched.round_trips()),
        control.round_trips(),
        batched.round_trips().saturating_mul(100).checked_div(control.round_trips()).unwrap_or(0)
    );
    // The other side of the same cycle, and it is not this task's to fix. A
    // registration maps its pages one at a time and invalidates after each,
    // exactly as the unmap did — so the saving above is half the saving that is
    // there. Printed rather than left out, because a measurement that reported
    // only the half it improved would be `R12`'s concession hidden in a metric.
    // `CHURN_GAP` in `xtask` carries the number and names the owner.
    kprintln!(
        "  churn mapping {} page(s) mapped cost {} invalidation(s) — still one per page, \
             unbatched, per half",
        control.pages_mapped,
        control.map_invalidations
    );
    // The number the task named, and the number it turned out to be. Delta over
    // the churn, then the machine's running total, because the second is what
    // says the first is a finding.
    kprintln!(
        "  churn shoot   {} shootdown(s) and {} ipi(s) from the churn; {} and {} on this boot",
        control.shootdowns.saturating_add(batched.shootdowns),
        control.ipis.saturating_add(batched.ipis),
        shot_pages,
        shot_ipis
    );

    // What the tables say, after the batch cleared them and published one
    // invalidation for the lot. Printed before the verdict reads it, because
    // these are the lines here that are an observation rather than a count.
    kprintln!(
        "  churn revoke  {} set(s) reachable while registered, {} still reachable after the \
             unmap",
        control.reachable_registered.saturating_add(batched.reachable_registered),
        control.standing_after_unmap.saturating_add(batched.standing_after_unmap)
    );
    kprintln!(
        "  churn frames  {} free before the churn, {} after everything was given back",
        control.frames_before,
        batched.frames_after
    );
    // The correctness half of the batch, and it is a separate pass because it is
    // not a measurement: a set with a page taken out from under it, unmapped as
    // one request, with every page walked afterwards. `unmap_range` returns the
    // first refusal *after* attempting the rest, and a version that stopped at
    // the hole would leave the pages beyond it translated — a device still
    // reaching memory a client took back. No client can make such a set, which
    // is why the stage makes one.
    // SAFETY: as the counted halves above.
    let holed = match unsafe { churn::hole(frames, &mut found.unit) } {
        Ok(holed) => holed,
        Err(why) => {
            kprintln!("FAIL: the unmap churn's hole stage: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };
    kprintln!(
        "  churn hole    {} page(s) mapped, {} taken out from under the request, {} still \
             translated after it",
        holed.mapped,
        holed.punched,
        holed.standing
    );
    churn_cost(boot, frames, found);

    let verdict = if control.retired != batched.retired || control.pages != batched.pages {
        Err("the two halves did not do the same work, so the ratio between them is not a saving")
    } else if control.requests == 0 || control.pages <= control.requests {
        Err("a buffer set spanned one page or none, so batching an unmap could save nothing \
                 and this run would report that whether or not anything was wrong")
    } else if control.invalidations != control.pages {
        Err("the per-page control did not invalidate once per page, which is what that \
                 policy means")
    } else if batched.invalidations != batched.requests {
        Err("the batched half did not invalidate once per request, which is what that \
                 policy means")
    } else if control.map_invalidations != control.pages_mapped
        || batched.map_invalidations != batched.pages_mapped
    {
        Err("the mapping half no longer invalidates once per page, which is either the \
                 gap `CHURN_GAP` declares having been closed — update it — or the attribution \
                 between the two halves having stopped working")
    } else if control.shootdowns != 0 || batched.shootdowns != 0 {
        Err("the churn issued a shootdown, which would mean the datapath has grown a path \
                 into a running process's address space — read RFC 0052 before changing this")
    } else if shot_pages == 0 {
        Err("this boot recorded no shootdown at all, so the zero above is a counter that \
                 does not work rather than a datapath that does not shoot down")
    } else if control.standing_after_unmap != 0 || batched.standing_after_unmap != 0 {
        Err("a retired buffer set is still translated in the unit's own tables, so an unmap \
                 counted pages it did not clear — which under `Invalidation::PerRequest` is a \
                 device left reaching memory its client took back")
    } else if control.reachable_registered != control.registered
        || batched.reachable_registered != batched.registered
    {
        Err("a registered set was not reachable in the unit's tables, so the zero beside it \
                 is a walk that answers no to everything rather than a revocation that worked")
    } else if holed.standing != 0 {
        Err("a batched unmap over a set with a hole in it left pages translated after the \
                 hole, so the request stopped at the first page it could not clear rather than \
                 attempting the rest — a device still reaching memory its client took back")
    } else if holed.mapped < 3 || holed.punched == 0 {
        Err("the hole stage did not arrange a hole with pages on both sides of it, so the \
                 zero beside it is a request that had nothing to skip")
    } else if control.frames_after != control.frames_before
        || batched.frames_after != batched.frames_before
    {
        Err("the churn did not give back every frame it took, which is a leak under exactly \
                 the churn `docs/test-taxonomy`'s frame-leak-under-churn row names")
    } else {
        Ok(())
    };

    match verdict {
        Ok(()) => kprintln!(
            "  churn verdict batching the invalidation is worth {} round trip(s) per set; \
                 the churn shoots down nothing",
            control.round_trips().saturating_sub(batched.round_trips()) / control.requests.max(1)
        ),
        Err(why) => {
            kprintln!("FAIL: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
}

/// The time half of `E1-B14`, recorded here and published only where a
/// nanosecond may be published.
///
/// # Why the boot is the workload and the host binary is not
///
/// `claims/0015` is a p99 for one unmap request, and most of that number is on
/// the far side of a device register: a page-table walk over the set, then one
/// global invalidation — two register writes and two spins on the unit clearing
/// a request bit. `bench/src/bin/unmap_churn.rs` cannot reach any of it, has
/// never claimed to, and says so on every run: there is no remapping unit on
/// the host, so what it times is the registry arithmetic above the hardware.
/// The operation the claim is about exists inside a boot on a machine with a
/// unit, so the workload is `churn::time` and this is where it runs.
///
/// # Why it refuses here
///
/// Recording is not publishing. The distribution is taken on every churn boot,
/// because a measurement apparatus that only runs on a machine nobody has yet
/// is an apparatus nobody has checked — `worst` being zero is what says the
/// clock did not move, and it is checked here rather than on the owed machine.
/// What is gated is the *quotable* number: percentiles are printed only when
/// the command line says this is a measurement environment, and `xtask` writes
/// that parameter from `f_bench::Environment::detect`. One rule about what may
/// be quoted, decided in the one place that already owns it, rather than a
/// second rule in the kernel that could disagree with it.
///
/// The refusal is the honest reading and not a formality: QEMU answers a global
/// invalidation instantly and in software, so a p99 taken here would be a
/// measurement of the emulator's dispatch loop wearing a hardware unit's name.
/// `claims/0015`'s `[hardware]` note is that sentence.
fn churn_cost(boot: &BootInfo, frames: &mut mem::FrameAllocator, remapping: &mut Remapping) {
    // The instrument's own arithmetic first, because it is the one part of this
    // that no host test can reach: `kernel/` builds for `x86_64-unknown-none`,
    // so the bucketing and the percentile underneath `claims/0015` are checked
    // the way `mem::self_test` checks the allocator — on the boot that is about
    // to use them.
    if let Err(why) = churn::Cost::self_test() {
        kprintln!("FAIL: the timed churn's own histogram: {why}");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    // SAFETY: as the counting pass one function up — the kernel's address space
    // is active, `frames` is rebound onto its direct map, and the unit is one
    // this boot programmed and enabled.
    let cost = match unsafe { churn::time(frames, &mut remapping.unit) } {
        Ok(cost) => cost,
        Err(why) => {
            kprintln!("FAIL: the timed unmap churn: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    // Both halves of *the instrument works*, and both are counts rather than
    // durations, so they mean the same thing on this machine and on the one
    // E0-D10 owes. A short run is a loop that stopped early; a zero maximum is
    // a timestamp counter that did not move, which is the failure that would
    // otherwise publish a beautiful p99 of nothing.
    if cost.taken() as usize != churn::OBSERVATIONS {
        kprintln!(
            "FAIL: the timed churn recorded {} observation(s) and owed {}",
            cost.taken(),
            churn::OBSERVATIONS
        );
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }
    if cost.worst() == 0 {
        kprintln!(
            "FAIL: every timed unmap took zero ticks, so the clock did not move and this \
                 distribution is an instrument that does not work"
        );
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    }

    let khz = arch::x86_64::apic::tsc_khz();
    kprintln!(
        "  churn cost    {} timed unmap request(s) through the shipped path, worst {} tick(s)",
        cost.taken(),
        cost.worst()
    );

    if !boot.has_parameter(b"measure") || khz == 0 {
        kprintln!(
            "  churn cost    latency refused — this machine is not a measurement environment"
        );
        kprintln!(
            "                a global invalidation is answered instantly and in software here, \
                 so a"
        );
        kprintln!(
            "                percentile would be the emulator's dispatch loop wearing a \
                 hardware unit's"
        );
        kprintln!("                name. claims/0015, pending on the machine E0-D10 owes.");
        return;
    }

    // A line shaped so that a reader knows it is not part of the fixture, which
    // is `boot_time`'s convention and its reason: a boot log carrying a
    // measurement is a fixture that fails at random.
    kprintln!(
        "  churn cost    p50 {} ns, p99 {} ns, p999 {} ns, max {} ns per unmap request \
             (measurement run; not the fixture log)",
        churn::nanos(cost.ticks_at(500), khz),
        churn::nanos(cost.ticks_at(990), khz),
        churn::nanos(cost.ticks_at(999), khz),
        churn::nanos(cost.worst(), khz)
    );
}

/// Give a component a core, let it schedule its own work inside it, and count
/// what crossed.
///
/// This is E1-B08's exit — *async work under load produces zero kernel entries
/// on the hot path, counted* — and the four halves are what make the zero worth
/// anything. `runtime=load` is the exit itself. `runtime=provoke` is the same
/// run with one crossing on purpose, and requires the count to move by exactly
/// as many as the component says it made: the two numbers are taken on opposite
/// sides of the boundary, so a build where counting had stopped publishes zero
/// twice and fails rather than looking clean. `runtime=reclaim` posts the notice
/// from the timer handler after the runtime has been working for a tick, and
/// requires it to park at its next allocation boundary with its own queue empty.
/// `runtime=hostile` scribbles its control ring's header before entry, and
/// requires the adoption to refuse rather than to fault, hang or believe it.
///
/// The verdict is the kernel's rather than the harness's, exactly as `user`,
/// `cap`, `iommu` and `blk` already are: it knows which half it was asked for,
/// what the counters say and what the component reported, and a harness reading
/// an exit code could not tell a runtime that parked from one that ran out of
/// work.
fn runtime_demonstration(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    clocks: arch::x86_64::apic::Clocks,
    tree: u64,
) -> Option<runtime::Report> {
    let half = if boot.has_parameter(b"runtime=load") {
        runtime::Half::Load
    } else if boot.has_parameter(b"runtime=provoke") {
        runtime::Half::Provoke
    } else if boot.has_parameter(b"runtime=reclaim") {
        runtime::Half::Reclaim
    } else if boot.has_parameter(b"runtime=hostile") {
        runtime::Half::Hostile
    } else {
        return None;
    };

    // The core the runtime is allocated. Another one where there is another
    // one, and this one where there is not — `timed_window`'s choice, and the
    // single-core branch needs the same system-call entry for the same reason.
    let me = arch::x86_64::current_cpu();
    let worker = if smp::started() > 1 { smp::first_worker() } else { me };
    if worker == me {
        // SAFETY: boot processor, after `gdt::init` installed every descriptor
        // the selectors written there name, and before anything enters ring 3 on
        // it. Idempotent: it writes four model-specific registers with the same
        // values `timed_window` would write later.
        unsafe { arch::x86_64::ring3::init() };
    }

    kprintln!(
        "  runtime       core {} allocated to a component, and the {} half: {}",
        worker,
        half.name(),
        match half {
            runtime::Half::Load => "it schedules its own work and nothing crosses the boundary",
            runtime::Half::Provoke => "the same, and one crossing on purpose so the zero moves",
            runtime::Half::Reclaim => "the timer posts a reclaim under load; it must park cleanly",
            runtime::Half::Hostile => "its control ring header is scribbled; adoption must refuse",
        }
    );

    // SAFETY: the boot processor, with the kernel's address space in `CR3`,
    // `frames` rebound onto its direct map, the direct map covering every boot
    // module — `reserved_ranges` put them all in the reserved list before the
    // allocator was populated — and `worker` a core that is up and idle.
    let outcome = unsafe {
        runtime::demonstrate(
            frames,
            space,
            features,
            boot,
            half,
            worker,
            TIMER_HZ,
            RUNTIME_TICKS,
            clocks.tsc_khz,
            RECLAIM_DEADLINE_NS,
            tree,
        )
    };

    let report = match outcome {
        Ok(report) => report,
        Err(why) => {
            kprintln!("FAIL: the runtime: {}", why.message());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    kprintln!(
        "  allocation    {} core(s) held; a reclaim of a hard-class core was {}, and a second \
         reclaim {} an earlier deadline",
        report.cores,
        if report.reserved_refused { "refused ADMISSION/RESERVED" } else { "SERVED" },
        if report.deadline_kept { "kept" } else { "MOVED" },
    );
    kprintln!(
        "  adoption      {} capabilit(ies) granted, {} notice(s) published before the first \
         instruction, {} drained by the component itself",
        report.granted,
        report.posted,
        report.tally.notices,
    );
    kprintln!(
        "  entries       {} on the hot path ({} call(s), {} fault(s)); {} at the allocation \
         boundary, {} timer tick(s), {} other interrupt(s) — {} in all",
        report.entries.on_the_hot_path(),
        report.entries.hot,
        report.entries.faults,
        report.entries.boundary,
        report.entries.ticks,
        report.entries.interrupts,
        report.entries.total(),
    );
    // A run that never adopted a ring completed no work, and the two fields
    // that would say how much carry the refusal instead — so the work line
    // would be reporting an `f_abi::error` domain as a number of work items.
    // `report::refusal` is where the two live and why; this is the log
    // agreeing with it rather than printing the same bytes under the wrong
    // heading.
    if let Some((domain, reason)) = f_store::report::refusal_of(&report.tally) {
        kprintln!(
            "  work          none: it refused before it had a ring to put any on, f_abi::error \
             domain {} reason {} ({} left on the ring)",
            domain,
            reason,
            report.left_behind,
        );
    } else {
        kprintln!(
            "  work          {} of {} item(s) completed, {} parked, {} left on the ring; \
             reclaimed {}, quiescent {}",
            report.tally.completed,
            f_store::report::LOAD,
            report.tally.parked,
            report.left_behind,
            report.tally.reclaimed(),
            report.tally.quiescent(),
        );
    }
    if report.half == runtime::Half::Reclaim {
        kprintln!(
            "  parking       the notice went out after {} item(s); the runtime finished {} more \
             and stopped, having crossed nothing in between",
            report.progress,
            report.tally.completed.saturating_sub(report.progress),
        );
        // Beside it and not instead of it, because it is the latency somebody
        // will eventually want — and not a bound, because under an emulator the
        // first execution of the exit path is translation rather than work.
        kprintln!(
            "  parking       in time: the notice went out at ring-3 tick {} and the runtime \
             exited at tick {}",
            report.posted_at,
            report.entries.ticks,
        );
    }
    if report.tally.code != f_store::report::OK {
        kprintln!(
            "  component     it stopped saying: {}",
            f_store::report::label(report.tally.code)
        );
    }

    match report.verdict() {
        Ok(()) => kprintln!(
            "  runtime verdict  the {} half held: {}",
            half.name(),
            match half {
                runtime::Half::Load =>
                    "a component scheduled its own work and crossed the boundary once, on the \
                     way out",
                runtime::Half::Provoke =>
                    "the frame and the component agree about every crossing that happened",
                runtime::Half::Reclaim =>
                    "an interrupt happened and a preemption did not: it parked at its own \
                     boundary with nothing outstanding",
                runtime::Half::Hostile =>
                    "a scribbled header was refused with a structured error rather than \
                     believed",
            }
        ),
        Err(why) => {
            kprintln!("FAIL: the runtime, {} half: {why}", half.name());
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    Some(report)
}

/// How many timer ticks the core running a runtime arms its own timer for.
///
/// A bound rather than a schedule, exactly as [`PROBE_TICKS`] is: the runtime's
/// own load is what ends the run, and this is what stops a wedged one holding
/// the core forever. Three seconds at [`TIMER_HZ`].
/// Unit: timer ticks.
const RUNTIME_TICKS: u64 = 3_000;

/// The deadline a reclaim notice carries.
///
/// A constant rather than a reading of the clock, and that is a decision rather
/// than a shortcut. Nothing in this build can *read* a deadline — a component
/// observes time only through a ring, and RFC 0004 gives it no clock — so a
/// number derived from the machine would put a different value in the boot log
/// on every run and buy nothing that could be checked. What is measured instead
/// is the mechanism the deadline exists for: how many timer intervals the
/// runtime took to reach an allocation boundary after it was told, which
/// `runtime::Report::verdict` bounds.
///
/// *Reversal:* a component that can read `Cqe::timestamp` against a clock of its
/// own, at which point this is `env.now()` plus a budget and the check is
/// whether the deadline was met rather than whether the boundary was reached.
/// Unit: nanoseconds, monotonic, in the control channel's epoch.
const RECLAIM_DEADLINE_NS: u64 = 1_000_000;

/// Ask this machine what it can reserve, and require the arithmetic to be able
/// to say both things.
///
/// **E1-B07**, and the half of its exit a boot can honestly observe. RFC 0007's
/// admission control is arithmetic over a machine description, and `admit.rs`
/// is the only place in the tree that fills one in from `cpuid`. What is printed
/// is what this part reports and what the arithmetic did with it — including,
/// on QEMU, a refusal: no thread level, no cache topology and no RDT allocation
/// leaf means one contention domain covering every core, the frame's own core
/// inside it, and no whole domain left to give. That is RFC 0007's expensive
/// branch taken rather than waived, and it is the honest answer on this machine
/// rather than a disappointment.
///
/// A stage that could only ever refuse would pass on a build whose admission
/// control had become a function that returns `Err`. So the same arithmetic is
/// asked a second time about a *described* part with siblings and RDT, which
/// must grant, must record all four of RFC 0007's components as obtained by a
/// mechanism, and must then refuse a second demand its capacity cannot hold.
/// The described half is not a claim about any machine and the log says so on
/// its own line — `blk`'s two boots and `mutate`'s argument, in one stage.
///
/// The other half of the exit — *a granted one meets its deadline under
/// adversarial load* — is `cargo xtask admission`, and `sim/src/reserve.rs`
/// says why it cannot be here: a deadline met is a timing, and under TCG a
/// timing is a property of the emulator.
fn admission_demonstration(boot: &BootInfo) {
    if !boot.has_parameter(b"admission") {
        return;
    }

    let report = match admit::demonstrate() {
        Ok(report) => report,
        Err(why) => {
            kprintln!(
                "FAIL: admission: the described part is one the table refuses: {}",
                why.why()
            );
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    let machine = report.machine;
    kprintln!(
        "  machine       {} physical core(s), {} thread(s) each; cache by {}, bandwidth \
         by {}, {} partition(s); the frame keeps {}",
        machine.physical_cores,
        machine.threads_per_core,
        if matches!(machine.cache, f_abi::reserve::Offers::Partition) {
            "partition"
        } else {
            "exclusion"
        },
        if matches!(machine.bandwidth, f_abi::reserve::Offers::Partition) {
            "partition"
        } else {
            "exclusion"
        },
        machine.partitions,
        machine.frame_cores,
    );
    kprintln!(
        "  contention    {} core(s) per cache domain, {} per bandwidth domain; the \
         sibling clause here would be {}",
        machine.cores_per_cache,
        machine.cores_per_bandwidth,
        report.sibling_here(),
    );
    match report.here {
        Some(why) => kprintln!("  here          REFUSED ADMISSION/{}: {}", why.reason(), why.why()),
        None => kprintln!("  here          admitted: this part can hold the demand"),
    }

    // And the described part. Named as described on its own line, every time,
    // because a number about a machine nobody has is a number somebody will
    // otherwise quote.
    match report.there {
        Some(grant) => kprintln!(
            "  described     NOT THIS MACHINE — a part with siblings and RDT grants {} \
             core(s), {} held idle; sibling by {}, cache by {}, bandwidth by {}, \
             memory by {}",
            grant.cores.count_ones(),
            grant.excluded.count_ones(),
            f_abi::reserve::obtained::label(grant.sibling),
            f_abi::reserve::obtained::label(grant.cache),
            f_abi::reserve::obtained::label(grant.bandwidth),
            f_abi::reserve::obtained::label(grant.memory),
        ),
        None => kprintln!("  described     nothing was granted"),
    }
    kprintln!(
        "  described     {} admission(s), {} refusal(s); the second demand was {}",
        report.admissions,
        report.refusals,
        match report.over {
            Some(why) => why.why(),
            None => "ADMITTED, which it must not be",
        }
    );
    // The two rows `claims/0010` publishes out of a boot rather than out of the
    // model, under the names it publishes them under. Prose above for a reader,
    // a key and a number here for `xtask::admission_reached` — because a claim
    // whose reproduction command does not print its own numbers is a claim
    // nobody can check, and these two were only ever in a sentence.
    //
    // `machine_grants` is deliberately not gated, and `claims/0010` writes out
    // why at length: zero is QEMU and non-zero is RDT silicon, and a threshold
    // either way makes one of those a red build for the wrong reason.
    kprintln!("  machine_grants {}", u32::from(report.here.is_none()));
    kprintln!("  described_grants {}", report.admissions);
    kprintln!(
        "  idle depth    state {} computed from the reservation table for the granted \
         core, and a core under no reservation answered {}",
        report.depth,
        if report.fallback { "fallback rather than a number" } else { "A NUMBER IT DID NOT EARN" },
    );

    match report.verdict() {
        Ok(()) => kprintln!(
            "  admission verdict  the arithmetic granted what fits, refused what does not, \
             and named which of RFC 0007's four components it could not deliver"
        ),
        Err(why) => {
            kprintln!("FAIL: admission: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
}

/// Drive the block datapath through a driver that lives outside the frame, and
/// require the right thing to happen to it.
///
/// This is E1-B02's exit and the clause E1-B01's could not observe, and the two
/// halves are the whole of it: `blk=inside` leaves the client's page in the
/// driver's device domain and requires a sector written through a ring to come
/// back byte for byte; `blk=outside` takes the page back between the write and
/// the read and requires the read to be a fault the unit records rather than a
/// transfer into memory the driver no longer holds.
///
/// The verdict is the kernel's rather than the harness's, exactly as `user`,
/// `cap` and `iommu` already are: it knows which half it was asked for and what
/// is in the client's buffer afterwards, and a harness reading an exit code
/// could not tell a refused transfer from a device that never answered.
///
/// Answers the report so the caller can publish it, rather than this reaching
/// into the state tree from underneath. A boot with no datapath answers `None`,
/// which is the same absence and a different claim from a datapath that
/// counted zero.
fn blk_datapath(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    remapping: Option<&mut Remapping>,
    clocks: arch::x86_64::apic::Clocks,
    tree: u64,
) -> Option<blk::Report> {
    let half = if boot.has_parameter(b"blk=inside") {
        blk::Half::Inside
    } else if boot.has_parameter(b"blk=outside") {
        blk::Half::Outside
    } else if boot.has_parameter(b"blk=escape") {
        blk::Half::Escape
    } else if boot.has_parameter(b"deadline=ordered") {
        blk::Half::Ordered
    } else if boot.has_parameter(b"deadline=arrival") {
        blk::Half::Arrival
    } else if boot.has_parameter(b"deadline=unadmitted") {
        blk::Half::Unadmitted
    } else {
        return None;
    };

    let Some(found) = remapping else {
        kprintln!("FAIL: the block datapath asked for on a machine with no remapping unit");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    // The core the driver is given. Another one, always: a driver and its
    // client are two ends of a ring and this frame is the client, so a machine
    // with one core has nowhere to put the server. `runtime_demonstration` can
    // fall back to running on this core because a runtime talks to nobody
    // while it runs; this cannot, and says so rather than pretending.
    let me = arch::x86_64::current_cpu();
    let Some(worker) = (smp::started() > 1).then(smp::first_worker).filter(|core| *core != me)
    else {
        kprintln!(
            "FAIL: the block datapath needs a second core — the driver serves from ring 3 and \
             the frame is its client"
        );
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    kprintln!(
        "  datapath      a driver outside the frame, and the {} half: {}",
        half.name(),
        match half {
            blk::Half::Inside => "the client's buffer stays in the driver's grant",
            blk::Half::Outside => "the client takes its page back between the two transfers",
            blk::Half::Escape => "the driver points the device past what it was answered",
            blk::Half::Ordered =>
                "batch work is queued and a hard-class read is submitted behind it",
            blk::Half::Arrival => "the same burst, with the driver ordering by arrival instead",
            blk::Half::Unadmitted =>
                "the same burst from a client admitted for the batch class, writing HARD",
        }
    );

    // SAFETY: the boot processor, with the kernel's address space in `CR3`,
    // `frames` rebound onto its direct map, translation enabled, and nothing
    // else in this kernel driving the device this finds — the `dma` stage runs
    // on a different parameter and this one has already returned if it was not
    // asked for.
    let outcome = unsafe {
        blk::demonstrate(
            frames,
            space,
            features,
            &mut found.unit,
            &found.window,
            &found.survey,
            boot,
            half,
            blk::Scheduling {
                cpu: worker,
                hz: TIMER_HZ,
                target: RUNTIME_TICKS,
                tsc_khz: clocks.tsc_khz,
                tree,
            },
        )
    };

    let report = match outcome {
        Ok(report) => report,
        Err(why) => {
            // A datapath that could not be set up is not a datapath that was
            // exercised, and reporting it as a pass is how this whole check
            // would come to mean nothing.
            //
            // A wall-clock bound running out is said apart from everything
            // else here. Every other arm is something the frame *observed*
            // going wrong, so a red line on one of those means a protection
            // fired; the two `bound` arms are spins bounded by a number scaled
            // off `tsc_khz`, and that fires for a wedged component and for a
            // runner slower than the number alike. One sentence for both is how
            // a slow machine comes to be read as a datapath defect, and how a
            // real wedge comes to be dismissed as one. `blk::Trouble::bound`.
            match why.bound() {
                Some(micros) => {
                    kprintln!(
                        "FAIL: the block datapath ran out of time: {} of {} us",
                        why.message(),
                        micros,
                    );
                    kprintln!(
                        "      That is an anti-wedge bound and not a check of the datapath: \
                         nothing above this line reported a failure, and a red here is a \
                         component that is stuck or a machine slower than the bound."
                    );
                }
                None => kprintln!("FAIL: the block datapath: {}", why.message()),
            }
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    // What the component's own manifest declares, read out of the record the
    // build compiled rather than repeated as constants in the frame. The
    // content hash is what a spawn names, so a driver whose *code* changed is
    // as visible here as one whose declaration did.
    kprintln!(
        "  blk manifest  virtio-blk declares {} register page(s) and {} B of untyped for its \
         queues, content {:#018x}",
        report.declared.frames,
        report.declared.bytes,
        report.declared.id.bits(),
    );
    kprintln!(
        "  blk device    requester {:#06x}, {} page(s) of register window, {} sector(s) of \
         capacity",
        report.bdf.source_id(),
        report.windows,
        report.capacity,
    );
    // Where the code that answered all of this ran. The line RFC 0047 added and
    // the one a reader should check first: everything below it is a claim about
    // a component, and this says the component was one.
    kprintln!(
        "  blk component core {} at ring 3, {} entr(ies) drained from its own loop, {} served, \
         {} refused, {} translation(s) asked of the frame, ended {}",
        report.cpu,
        report.drained,
        report.counters.served,
        report.counters.refused,
        report.asked,
        if report.exited { "by EXIT" } else { "in a FAULT" },
    );
    kprintln!(
        "  blk grant     the client registered one page at {:#018x}; a capability with no \
         right to grant was {}",
        report.registered_at,
        if report.refused_without_grant { "refused" } else { "ACCEPTED" },
    );
    kprintln!(
        "  blk transfer  write {}, read {}, {} byte(s) of the sink still unwritten, bytes {}",
        if report.wrote { "completed" } else { "refused" },
        if report.read { "completed" } else { "refused" },
        report.untouched,
        if report.matched { "match" } else { "DO NOT match" },
    );
    // The exit criterion, as two numbers rather than as a sentence. `copies` is
    // the driver's own tally of bytes it moved on the data path and must be
    // zero; `provoked` is the same function's tally when the boot calls it on
    // purpose and must not be, because a counter nothing can move is not a
    // counter.
    kprintln!(
        "  blk copies    {} byte(s) copied on the data path of {} transferred; {} byte(s) \
         moved through the same function on purpose",
        report.counters.copies,
        report.counters.bytes,
        report.counters.provoked,
    );
    // What the driver aimed at, beside where the unit says the transaction
    // went. On `escape` these are a page apart and the second is the address the
    // component's own arithmetic produced; on the other two halves nothing is
    // provoked and the count is zero, which `Report::verdict` requires.
    kprintln!(
        "  blk escape    {} descriptor(s) pointed past a registration's answer; this \
         half expects a fault at {:#018x}",
        report.counters.escaped,
        report.expected_fault(),
    );
    // E1-B06's numbers, printed on every block boot rather than only on the
    // halves that are about them: a driver ordering by deadline when nothing
    // asked it to is as much a finding as one that would not.
    kprintln!(
        "  blk deadline  admitted {}, client {}, ordering asked {} used {}; {} request(s) \
         deepest in the queue, {} overtaken, {} in flight at once",
        report.declared.admitted,
        report.half.client_admitted(),
        report.half.ordering(),
        report.ordered,
        report.queued_max,
        report.overtaken,
        report.in_flight,
    );
    kprintln!(
        "  blk overtake  the hard-class read came back at position {} of {}, ahead of {} \
         batch request(s) submitted before it; {} completion(s) reported a shortfall, {} \
         entr(ies) refused for a class the client does not hold",
        report.hard_at,
        report.burst(),
        report.overtook,
        report.counters.shortfall,
        report.counters.unadmitted,
    );
    match report.fault {
        Some(fault) => kprintln!(
            "  blk fault     requester {:#06x} {} {:#018x}, reason {:#04x} — {} record(s)",
            fault.source,
            if fault.read { "read" } else { "wrote" },
            fault.address,
            fault.reason,
            report.faults,
        ),
        None => kprintln!("  blk fault     none recorded"),
    }

    match report.verdict() {
        Ok(()) => kprintln!(
            "  blk verdict   {}",
            match half {
                blk::Half::Inside =>
                    "a sector went out and came back through a ring, and nothing was copied",
                blk::Half::Outside =>
                    "the grant was withdrawn under a live registration and the transfer faulted",
                blk::Half::Escape =>
                    "the driver pointed the device outside its own grant and the unit faulted it",
                blk::Half::Ordered =>
                    "a hard-class read submitted last was handed to the device first, and the \
                     batch work queued ahead of it waited",
                blk::Half::Arrival =>
                    "the same burst in arrival order put the read last, so the half above \
                     measured an ordering and not an array",
                blk::Half::Unadmitted =>
                    "a client that was not admitted for the hard class wrote it and was \
                     refused rather than served",
            }
        ),
        Err(why) => {
            kprintln!("FAIL: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
    Some(report)
}

/// E1-B03: a second driver outside the frame, and one frame in and out.
///
/// # Why a second driver has a stage of its own
///
/// Because it is a different device, a different domain and a different
/// experiment — and because the point of it is the *comparison*. `blk_datapath`
/// asks whether a component can drive hardware with no `unsafe` and copy
/// nothing; this asks whether the shape that answer arrived in is a shape or a
/// coincidence, which is a question only a second sample can be evidence about.
/// `kernel/src/net.rs` and
/// `docs/rfc/0051-a-second-driver-is-what-says-the-shape-is-a-shape.md` are
/// where the comparison is written down.
///
/// Three halves, and the middle one is what makes the first mean anything.
/// `net=inside` sends an address-resolution request and requires the reply to
/// land in a registered buffer. `net=silent` is the identical client with the
/// transmit removed and requires nothing to land. `net=escape` sends the same
/// request and has the driver point the device past what its registration
/// answered before the address becomes a *receive* descriptor, so what the
/// remapping unit must refuse is a device **writing** into memory the component
/// never held.
///
/// The verdict is the kernel's rather than the harness's, exactly as `user`,
/// `cap`, `iommu` and `blk` already are: a harness reading an exit code could
/// not tell a refused write from a link with nothing on it.
fn net_datapath(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    remapping: Option<&mut Remapping>,
    clocks: arch::x86_64::apic::Clocks,
    tree: u64,
) -> Option<net::Report> {
    let half = if boot.has_parameter(b"net=inside") {
        net::Half::Inside
    } else if boot.has_parameter(b"net=silent") {
        net::Half::Silent
    } else if boot.has_parameter(b"net=escape") {
        net::Half::Escape
    } else {
        return None;
    };

    let Some(found) = remapping else {
        kprintln!("FAIL: the network datapath asked for on a machine with no remapping unit");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    // Another core, always, for `blk_datapath`'s reason: a driver and its client
    // are two ends of a ring and this frame is the client, so a machine with one
    // core has nowhere to put the server.
    let me = arch::x86_64::current_cpu();
    let Some(worker) = (smp::started() > 1).then(smp::first_worker).filter(|core| *core != me)
    else {
        kprintln!(
            "FAIL: the network datapath needs a second core - the driver serves from ring 3 \
             and the frame is its client"
        );
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    kprintln!(
        "  packets       a second driver outside the frame, and the {} half: {}",
        half.name(),
        match half {
            net::Half::Inside => "a frame goes out and the answer lands in a registered buffer",
            net::Half::Silent => "the same client with nothing sent, so nothing may arrive",
            net::Half::Escape =>
                "the driver points the device past what it was answered, on the descriptor \
                 the device writes",
        }
    );

    // SAFETY: the boot processor, with the kernel's address space in `CR3`,
    // `frames` rebound onto its direct map, translation enabled, and nothing
    // else in this kernel driving the device this finds - the `dma` and `blk`
    // stages run on different parameters and drive a different function.
    let outcome = unsafe {
        net::demonstrate(
            frames,
            space,
            features,
            &mut found.unit,
            &found.window,
            &found.survey,
            boot,
            half,
            net::Scheduling {
                cpu: worker,
                hz: TIMER_HZ,
                target: RUNTIME_TICKS,
                tsc_khz: clocks.tsc_khz,
                tree,
            },
        )
    };

    let report = match outcome {
        Ok(report) => report,
        Err(why) => {
            // A datapath that could not be set up is not a datapath that was
            // exercised. A wall-clock bound running out is said apart from
            // everything else, for the reason `blk_datapath` gives at length:
            // every other arm is something the frame observed going wrong, and
            // these two fire for a wedged component and for a slow runner alike.
            match why.bound() {
                Some(micros) => {
                    kprintln!(
                        "FAIL: the network datapath ran out of time: {} of {} us",
                        why.message(),
                        micros,
                    );
                    kprintln!(
                        "      That is an anti-wedge bound and not a check of the datapath: \
                         nothing above this line reported a failure, and a red here is a \
                         component that is stuck or a machine slower than the bound."
                    );
                }
                None => kprintln!("FAIL: the network datapath: {}", why.message()),
            }
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    kprintln!(
        "  net manifest  virtio-net declares {} register page(s) and {} B of untyped for its \
         queues, content {:#018x}",
        report.declared.frames,
        report.declared.bytes,
        report.declared.id.bits(),
    );
    kprintln!(
        "  net device    requester {:#06x}, {} page(s) of register window",
        report.bdf.source_id(),
        report.windows,
    );
    kprintln!(
        "  net component core {} at ring 3, {} entr(ies) drained from its own loop, {} served, \
         {} refused, {} translation(s) asked of the frame, ended {}",
        report.cpu,
        report.drained,
        report.counters.served,
        report.counters.refused,
        report.asked,
        if report.exited { "by EXIT" } else { "in a FAULT" },
    );
    kprintln!(
        "  net grant     the client registered one page at {:#018x}; a capability with no \
         right to grant was {}",
        report.registered_at,
        if report.refused_without_grant { "refused" } else { "ACCEPTED" },
    );
    // The two directions side by side, and neither says what the other does. A
    // transmit that completed is a device that took the frame and never evidence
    // that it was delivered - virtio-net answers a transmit with no status at
    // all - so the only thing on this line that says a frame left the machine is
    // that something outside it answered.
    kprintln!(
        "  net frame     transmit {}, {} buffer(s) posted, {} frame(s) received, {} byte(s) \
         landed, {} byte(s) of the buffer still unwritten, reply {}",
        if report.transmitted { "taken by the device" } else { "not sent" },
        report.counters.posted,
        report.counters.received,
        report.frame_bytes,
        report.untouched,
        if report.matched { "answers this boot's request" } else { "ABSENT" },
    );
    // The exit criterion, as two numbers rather than as a sentence, and it is
    // the receive half of it that is the hard one: this component is the only
    // thing between a device and a client's buffer, and the obvious
    // implementation reads the frame to find out how long it is.
    kprintln!(
        "  net copies    {} byte(s) copied on the data path of {} transferred; {} byte(s) \
         moved through the same function on purpose",
        report.counters.copies,
        report.counters.bytes,
        report.counters.provoked,
    );
    // The obligation the receive direction creates, as a number. A posted
    // receive is a buffer with no answer owed, so a driver that stopped while
    // holding one would leave its client with an in-flight buffer RFC 0024 gives
    // no way to take back.
    kprintln!(
        "  net teardown  {} receive buffer(s) given back as cancellations; {} turn(s) of the \
         receive poll found nothing",
        report.counters.cancelled,
        report.counters.spun,
    );
    // The expectation, printed only by the half that holds it. Two halves out of
    // three require `faults == 0`, so a line telling their reader to expect a
    // fault at an address is a boot log stating something the run must not do.
    if report.half.beyond() == 0 {
        kprintln!(
            "  net escape    {} descriptor(s) pointed past a registration's answer; this half \
             expects no fault",
            report.counters.escaped,
        );
    } else {
        kprintln!(
            "  net escape    {} descriptor(s) pointed past a registration's answer; this half \
             expects a fault at {:#018x}",
            report.counters.escaped,
            report.expected_fault(),
        );
    }
    kprintln!(
        "  net deadline  admitted {}, {} completion(s) reported a shortfall, {} entr(ies) \
         refused for a class the client does not hold",
        report.declared.admitted,
        report.counters.shortfall,
        report.counters.unadmitted,
    );
    match report.fault {
        Some(fault) => kprintln!(
            "  net fault     requester {:#06x} {} {:#018x}, reason {:#04x} - {} record(s)",
            fault.source,
            if fault.read { "read" } else { "wrote" },
            fault.address,
            fault.reason,
            report.faults,
        ),
        None => kprintln!("  net fault     none recorded"),
    }

    match report.verdict() {
        Ok(()) => kprintln!(
            "  net verdict   {}",
            match half {
                net::Half::Inside =>
                    "a frame went out and the answer came back into a registered buffer, and \
                     nothing was copied",
                net::Half::Silent =>
                    "the same client sent nothing and nothing arrived, so the half above \
                     measured a reply and not a link",
                net::Half::Escape =>
                    "the driver pointed the device outside its grant on the direction the \
                     device writes, and the unit faulted it",
            }
        ),
        Err(why) => {
            kprintln!("FAIL: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }
    Some(report)
}

/// How long the machine waits for the harness to say it has looked at the
/// screen. Unit: microseconds.
///
/// **The only wall-clock wait in this kernel that is not an anti-wedge bound**,
/// and it is here because E1-B04's exit criterion is an observation nothing
/// inside this machine can make. A scanout has no read-back command, so the
/// evidence that a picture reached the display is a capture taken from outside
/// the emulator — and a capture needs the machine to still exist when it is
/// taken.
///
/// Sixty seconds, which is far longer than a harness needs and far shorter than
/// the harness's own boot timeout, so a run where nothing answers ends by this
/// number rather than by being killed. It is a count of microseconds against
/// `tsc_khz` like every other bound in this file, so a slow host waits the same
/// wall-clock time rather than the same number of turns.
///
/// RFC 0046 says a hang is a count: this one is counted, it is printed, and the
/// boot carries on either way. A harness that never answers produces a boot that
/// says so and still reaches its own verdict, which is the direction to be wrong
/// in — the picture is then unverified rather than the machine being stuck.
const CAPTURE_MICROS: u64 = 60_000_000;

/// E1-B04. A third driver, a device of a different kind, and a result that is
/// not in this machine.
///
/// # Why a third one, after `blk` and `net`
///
/// Because a block device and a network interface both move opaque bytes, and a
/// display controller does not: it takes structured commands, answers every one
/// of them, and owns a scanout. `kernel/src/gpu.rs` and
/// `docs/rfc/0054-a-third-driver-is-a-device-of-a-different-kind.md` are where
/// the comparison is written down, and the short version is that four of the
/// five things RFC 0051 said a second driver could not reuse turn out to have
/// been about *receiving* rather than about being a second driver.
///
/// # Why this stage ends by waiting
///
/// The other two datapaths judge themselves: the evidence is bytes in a client's
/// buffer and records in a remapping unit, and both are inside the machine. A
/// scanout is not. The 2D display protocol has no command that reads a resource
/// back, so this kernel can say what the display *accepted* and cannot say what
/// it drew. So the boot publishes one number — the hash of the client's own
/// pixels, in the order a screen capture reports them — and then holds still
/// while `cargo xtask gpu` captures the emulator's framebuffer and compares. The
/// byte on the serial port is the harness saying it has looked.
///
/// The picture survives everything above the wait, and that is not luck:
/// `TRANSFER_TO_HOST_2D` copies the pixels into a resource on the host's side of
/// the emulator, so clearing the bus-master bit, detaching the function from its
/// domain and freeing the client's page leave the scanout exactly as it was.
/// `user/virtio-gpu` never resets the device, which is the one thing that would
/// take it away.
fn gpu_datapath(
    boot: &BootInfo,
    frames: &mut mem::FrameAllocator,
    space: &paging::AddressSpace,
    features: paging::Features,
    remapping: Option<&mut Remapping>,
    clocks: arch::x86_64::apic::Clocks,
    tree: u64,
) -> Option<gpu::Report> {
    let half = if boot.has_parameter(b"gpu=inside") {
        gpu::Half::Inside
    } else if boot.has_parameter(b"gpu=blank") {
        gpu::Half::Blank
    } else if boot.has_parameter(b"gpu=escape") {
        gpu::Half::Escape
    } else {
        return None;
    };

    let Some(found) = remapping else {
        kprintln!("FAIL: the display datapath asked for on a machine with no remapping unit");
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    // Another core, always, for `blk_datapath`'s reason: a driver and its client
    // are two ends of a ring and this frame is the client, so a machine with one
    // core has nowhere to put the server.
    let me = arch::x86_64::current_cpu();
    let Some(worker) = (smp::started() > 1).then(smp::first_worker).filter(|core| *core != me)
    else {
        kprintln!(
            "FAIL: the display datapath needs a second core - the driver serves from ring 3 \
             and the frame is its client"
        );
        arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
    };

    kprintln!(
        "  picture       a third driver outside the frame, and the {} half: {}",
        half.name(),
        match half {
            gpu::Half::Inside => "a client's pixels are put on a scanout through a ring",
            gpu::Half::Blank => "the same pixels, and nothing submitted, so nothing may appear",
            gpu::Half::Escape =>
                "the driver points the device past what it was answered, at the memory the \
                 display reads a frame out of",
        }
    );

    // SAFETY: the boot processor, with the kernel's address space in `CR3`,
    // `frames` rebound onto its direct map, translation enabled, and nothing
    // else in this kernel driving the device this finds - the `dma`, `blk` and
    // `net` stages run on different parameters and drive different functions.
    let outcome = unsafe {
        gpu::demonstrate(
            frames,
            space,
            features,
            &mut found.unit,
            &found.window,
            &found.survey,
            boot,
            half,
            gpu::Scheduling {
                cpu: worker,
                hz: TIMER_HZ,
                target: RUNTIME_TICKS,
                tsc_khz: clocks.tsc_khz,
                tree,
            },
        )
    };

    let report = match outcome {
        Ok(report) => report,
        Err(why) => {
            match why.bound() {
                Some(micros) => {
                    kprintln!(
                        "FAIL: the display datapath ran out of time: {} of {} us",
                        why.message(),
                        micros,
                    );
                    kprintln!(
                        "      That is an anti-wedge bound and not a check of the datapath: \
                         nothing above this line reported a failure, and a red here is a \
                         component that is stuck or a machine slower than the bound."
                    );
                }
                None => kprintln!("FAIL: the display datapath: {}", why.message()),
            }
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    };

    kprintln!(
        "  gpu manifest  virtio-gpu declares {} register page(s) and {} B of untyped for its \
         queue, content {:#018x}",
        report.declared.frames,
        report.declared.bytes,
        report.declared.id.bits(),
    );
    kprintln!(
        "  gpu device    requester {:#06x}, {} page(s) of register window",
        report.bdf.source_id(),
        report.windows,
    );
    kprintln!(
        "  gpu component core {} at ring 3, {} entr(ies) drained from its own loop, {} served, \
         {} refused, {} translation(s) asked of the frame, ended {}",
        report.cpu,
        report.drained,
        report.counters.served,
        report.counters.refused,
        report.asked,
        if report.exited { "by EXIT" } else { "in a FAULT" },
    );
    kprintln!(
        "  gpu grant     the client registered one page at {:#018x}; a capability with no \
         right to grant was {}",
        report.registered_at,
        if report.refused_without_grant { "refused" } else { "ACCEPTED" },
    );
    // The device's own vocabulary, which is the thing this driver has and
    // neither of the other two does: every display command is answered with a
    // typed response, so *the display refused this* is a number rather than a
    // silence.
    kprintln!(
        "  gpu commands  {} answered by the display, {} declined, {} resource(s) created and \
         never freed, {} frame(s) flushed",
        report.counters.commands,
        report.counters.declined,
        report.counters.resources,
        report.counters.shown,
    );
    kprintln!(
        "  gpu copies    {} byte(s) copied on the data path of {} transferred; {} byte(s) \
         moved through the same function on purpose",
        report.counters.copies,
        report.counters.bytes,
        report.counters.provoked,
    );
    if report.half.beyond() == 0 {
        kprintln!(
            "  gpu escape    {} backing entr(ies) pointed past a registration's answer; this \
             half expects no fault",
            report.counters.escaped,
        );
    } else {
        kprintln!(
            "  gpu escape    {} backing entr(ies) pointed past a registration's answer; this \
             half expects a read fault at {:#018x}",
            report.counters.escaped,
            report.expected_fault(),
        );
    }
    kprintln!(
        "  gpu deadline  admitted {}, {} completion(s) reported a shortfall, {} entr(ies) \
         refused for a class the client does not hold",
        report.declared.admitted,
        report.counters.shortfall,
        report.counters.unadmitted,
    );
    match report.fault {
        Some(fault) => kprintln!(
            "  gpu fault     requester {:#06x} {} {:#018x}, reason {:#04x} - {} record(s)",
            fault.source,
            if fault.read { "read" } else { "wrote" },
            fault.address,
            fault.reason,
            report.faults,
        ),
        None => kprintln!("  gpu fault     none recorded"),
    }

    match report.verdict() {
        Ok(()) => kprintln!(
            "  gpu verdict   {}",
            match half {
                gpu::Half::Inside =>
                    "the display accepted every command and the client's buffer came back \
                     unwritten, with nothing copied",
                gpu::Half::Blank =>
                    "the same client submitted nothing and the display was sent nothing",
                gpu::Half::Escape =>
                    "the driver pointed the device outside its grant at the memory a display \
                     reads, and the unit faulted it on a read",
            }
        ),
        Err(why) => {
            kprintln!("FAIL: {why}");
            arch::x86_64::exit_qemu(arch::x86_64::Exit::Failure);
        }
    }

    // The line the harness waits for, and the only line in this kernel written
    // for a reader outside the machine. It carries what the harness cannot
    // derive: how large the picture is, and the hash of the client's own pixels
    // in the order a screen capture reports them. `cargo xtask gpu` captures the
    // emulator's framebuffer when it sees this, hashes what it got, and requires
    // the two to agree on the half that shows and to disagree on the two that do
    // not.
    //
    // Deliberately *after* the verdict, so that a boot whose datapath failed has
    // already exited and the harness never captures a screen from a run that
    // went red.
    kprintln!(
        "  gpu display   {} x {} pixels, client rgb fnv1a {:#018x}",
        report.width(),
        report.height(),
        report.display_hash,
    );

    // And now the wait. See `CAPTURE_MICROS`.
    let deadline = smp::deadline_after(clocks.tsc_khz, CAPTURE_MICROS);
    let mut acknowledged = false;
    while !smp::past(deadline) {
        if arch::x86_64::serial::Serial.received().is_some() {
            acknowledged = true;
            break;
        }
        core::hint::spin_loop();
    }
    if acknowledged {
        kprintln!("  gpu captured  the harness acknowledged the frame");
    } else {
        kprintln!(
            "  gpu captured  nothing acknowledged the frame inside {} us, so the picture on \
             the display is unverified by this boot",
            CAPTURE_MICROS,
        );
    }
    Some(report)
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
