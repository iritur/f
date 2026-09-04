// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A process: an address space, four pages, a table of capabilities, and a way
//! of ending.
//!
//! # What a process is at M4
//!
//! Still less than the word usually means, and the gap is still the honest
//! part. There is no scheduler, no ring, and no second process. What exists is
//! what all three of those need first: a body of code running at privilege
//! level three, in an address space of its own, that the frame can start and can
//! end — including when the process would rather it did not.
//!
//! M4 added the fourth thing on that list. A process holds capabilities now, and
//! the difference that makes is not that it can be refused — it could always be
//! refused, by a page table — but that it can be *given* something, given
//! something weaker than what the giver holds, and have it taken back. One of
//! its four pages is reachable only because a capability authorised the mapping,
//! which is what turns the table from bookkeeping into a boundary.
//!
//! It takes the memory back with the name, since E0-B10. Revoking a frame
//! capability that has been mapped clears the entry, invalidates this core's
//! translation and tells every other running core — and `cap=unmap` is the boot
//! where a process reads the page afterwards and takes a page fault at an
//! address it was reading a moment earlier. This module comment used to say the
//! opposite, and `withdraw` is where the change lives.
//!
//! # One at a time, on a core that is not the one that built it
//!
//! Building a process means allocating, and the frame allocator belongs to one
//! core; taking a privilege-level transition is something any core can do. So
//! the work is three functions across two cores — [`prepare`] on the core that
//! owns the allocator, [`execute`] on the core that runs it, [`reap`] back on
//! the first — and the boot processor is left free to hold the timer window the
//! milestone is about.
//!
//! Still one process at a time. There is no scheduler and no run queue;
//! placement is "the other core", which is the whole of the decision because
//! there is nothing to decide between. Two processes run per boot and they run
//! in sequence, which is what makes the second one evidence about the first: a
//! table cleared between them does not let the second resolve a handle the
//! first held.
//!
//! # Why the frame decides when the process stops running
//!
//! The process asks. It announces itself, then repeatedly asks the frame
//! whether it has run long enough, and the frame answers by counting timer
//! ticks that were taken *out of ring 3* — not ticks in total, and not
//! instructions.
//!
//! The alternative is a process that counts its own iterations, and it fails in
//! a way worth naming: how long a fixed number of iterations takes differs by
//! two orders of magnitude between an emulator and a machine, so the number of
//! timer ticks it spans would differ too, and the boot log carries that number.
//! A fixture whose contents depend on how fast the host is, is not a fixture.
//! Counting in ticks makes the same boot produce the same log on both.
//!
//! It also happens to be the property the milestone is about. `user=…` provokes
//! a fault; what has to be true *while* it runs is that the timer kept its
//! schedule with ring 3 holding the core, and a count of ticks taken from ring
//! 3 is the direct evidence of it.
//!
//! # Ending
//!
//! Two ways in, one way out. A fault taken at ring 3 is recorded and the
//! interrupt frame is pointed back at the kernel; a call to `SYS_EXIT` records
//! a status and jumps there. Both land on the same instruction inside
//! `ring3::enter`, on the same stack, with the same registers restored — see
//! that module. Then the address space stops being the one in `CR3`, and every
//! frame the process was made of goes back on the free list.
//!
//! The free count before and after is compared, and a boot where it does not
//! match fails. It is the same assertion M1 makes about a thousand random
//! allocations, applied to the first thing in this system that owns memory: a
//! process that leaks a page table each time it dies is a kernel that runs out
//! of memory in a week and blames whatever was allocating at the time.

use f_abi::cap::{CapType, Handle, rights};
use f_abi::{door, error};

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::{apic, paging, probe, read_tsc, ring3};
use crate::cap::{Direct, SLOTS_PER_PAGE, TABLE_SLOTS, Table};
use crate::kprintln;
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};
use crate::percpu::PerCpu;

/// Where a process's text is mapped.
///
/// Four mebibytes up, which is a long way from the null page and inside the
/// first two-mebibyte region a page table covers — so text, guard, stack and the
/// page a process maps for itself are one table between them. The address is
/// still fixed, and at M4 that is no longer only for want of somewhere to put
/// it: a process holds an address space capability now, and what it does not
/// hold is anything that would let it pay for a second page table. E0-B10 loads
/// a real component, and the layout stops being a constant when a component
/// arrives with its own idea of where its text goes.
pub const TEXT: u64 = 0x0000_0000_0040_0000;

/// One page, deliberately unmapped, between the text and the stack.
///
/// The same guard the kernel gives its own stacks, for the same reason: a stack
/// that grows past its end should hit nothing rather than hit the text of the
/// program that is running on it.
pub const GUARD: u64 = TEXT + FRAME_SIZE;

/// Where a process's stack is mapped.
pub const STACK: u64 = GUARD + FRAME_SIZE;

/// The stack pointer a process starts with: one past its stack.
pub const STACK_TOP: u64 = STACK + FRAME_SIZE;

/// Where a process may map a frame it holds a capability for.
///
/// One unmapped page above the stack, so a stack that grows the wrong way does
/// not walk into a granted page — the same guard the text side already has, on
/// the side that acquired a neighbour at M4.
///
/// Inside the same two-mebibyte region as text and stack, and that is not an
/// accident: the page table covering it already exists, which is what lets a
/// process map here without the frame allocating a table on its behalf. See
/// [`paging::map_user_live`], which refuses rather than allocates.
pub const GRANT: u64 = STACK_TOP + FRAME_SIZE;

/// Where a process maps the frame's published state tree.
///
/// The third page in this region, and still inside the two mebibytes one table
/// covers — so reading the frame's own counters costs a process no page table
/// and no allocation, which is what makes RFC 0013's *read, never delivered*
/// affordable enough to leave on.
pub const TREE: u64 = GRANT_SECOND + FRAME_SIZE;

/// Where the frame maps a runtime's control ring.
///
/// The fourth page in this region, and still inside the two mebibytes one page
/// table covers — so a runtime's whole world is one table, which is what makes
/// *no kernel involvement on the hot path* an arithmetic fact rather than an
/// aspiration: there is no page a runtime can touch that could fault.
///
/// It must equal `f_store::runtime`'s `CONTROL_AT`, which is the same
/// arrangement `user/init` already has for [`GRANT`] and [`TREE`] and has the
/// same failure mode: a build where the two disagree is a page fault at the
/// component's first adoption, reported by the frame as an ordinary ring-3
/// fault rather than as anything mysterious. *Reversal:* `door::Entry` growing
/// a field for the address, which RFC 0008 says a component is entered with.
pub const RING: u64 = TREE + FRAME_SIZE;

/// Where the frame maps a runtime's own work ring.
///
/// The fifth and last page. This is the region a runtime schedules inside — it
/// is both ends of the ring described here, which is what an executor is — and
/// it is the frame's memory only in the sense that the frame charged an account
/// for it. Nothing in the frame reads it while the runtime runs.
///
/// Must equal `f_store::runtime`'s `WORK_AT`. See [`RING`].
pub const WORK: u64 = RING + FRAME_SIZE;

/// How many pages of text the frame reserves for a component it builds from a
/// component file.
///
/// Sixteen — sixty-four kibibytes — and it is a bound on the layout rather than
/// on the loader. Every process this kernel ran before RFC 0047 was one page of
/// text and the build refused an image that was not, which was a bound nobody
/// had to defend while the only components were an announcement and an
/// executor. A driver is not: `user/virtio-blk` compiles to about thirteen
/// kibibytes the moment its own polling loop reaches the transport, the queue
/// and the registration table.
///
/// Sixteen rather than four, because what this constant really buys is that
/// **the addresses below do not move when a component's code grows.** A layout
/// derived from an image's own length would make [`BLK_BOARD`] a different
/// number on every commit, and it is the one address a component holds as a
/// constant.
///
/// It is a *reservation* and not a charge. `component::spawn` charges the
/// account for the pages an image actually occupies, so a component that fits
/// in one page pays for one; what the reservation costs is address space, of
/// which a component has forty-seven bits.
///
/// *Reversal:* a loader that reads a component's headers and a component that
/// is told where its own world is rather than holding one address — E5, and at
/// that point every constant in this section goes, not just this one.
/// Unit: pages.
pub const TEXT_PAGES: usize = 16;

/// One unmapped page between a component's text reservation and its stack.
///
/// [`GUARD`]'s argument, for every shape whose text is a reservation rather
/// than a page — which since RFC 0047 is every component built from a component
/// file, spawned into a place or scheduled on a core. One layout and not two,
/// because a place holds *any* component and a layout that depended on which
/// one would be a place that was not interchangeable.
pub const SPAWN_GUARD: u64 = TEXT + TEXT_PAGES as u64 * FRAME_SIZE;

/// Where such a component's stack is mapped.
pub const SPAWN_STACK: u64 = SPAWN_GUARD + FRAME_SIZE;

/// The stack pointer it starts with: one past its stack.
pub const SPAWN_STACK_TOP: u64 = SPAWN_STACK + FRAME_SIZE;

/// Where the frame maps such a component's control ring.
///
/// The ring the frame publishes its notices onto, and — for a driver — the ring
/// it asks the frame for a device translation on. RFC 0047. A driver reads this
/// address out of [`BLK_BOARD`] rather than holding it as a constant, which is
/// why it may move without anything outside this file being edited.
pub const SPAWN_CONTROL: u64 = SPAWN_STACK_TOP + FRAME_SIZE;

/// Where the frame maps the ring a driver serves its client on.
pub const BLK_DATA: u64 = SPAWN_CONTROL + FRAME_SIZE;

/// Where the frame maps the page that says where everything else is.
///
/// **The one address a driver component holds as a constant**, and it must
/// equal `f_virtio_blk::routing::AT`. `kernel/src/blk.rs` asserts that at
/// compile time; a comment would be a claim and the assertion is a check, and
/// the kernel is the one artefact that links both definitions.
pub const BLK_BOARD: u64 = BLK_DATA + FRAME_SIZE;

/// Where the frame maps the device's register pages for the driver.
///
/// Four of them, which is what `user/virtio-blk/manifest.toml` declares and
/// what the modern virtio transport lays out in one base-address register.
/// Mapped [`paging::UserPage::Device`] and not `Data`: see that variant.
pub const BLK_REGISTERS: u64 = BLK_BOARD + FRAME_SIZE;

/// How many register pages the driver shape maps. Unit: pages.
pub const BLK_REGISTER_PAGES: usize = 4;

/// Where the frame maps the driver's queue memory.
///
/// The untyped need its manifest declares, sixty-four kibibytes of it, whole
/// and contiguous because a virtqueue is one descriptor table and two rings at
/// fixed offsets from each other.
pub const BLK_QUEUES: u64 = BLK_REGISTERS + BLK_REGISTER_PAGES as u64 * FRAME_SIZE;

/// A second address in the same region, used only by provocations whose mapping
/// is supposed to be refused.
///
/// It exists so that a refusal cannot be the address being already mapped: the
/// refusal under test is about authority, and an argument error arriving in its
/// place would pass the test for the wrong reason.
pub const GRANT_SECOND: u64 = GRANT + FRAME_SIZE;

/// "I am here." Takes nothing, and the frame records that it happened.
///
/// It is the whole of what a process can say before there is a channel to say
/// it on. At M5 this is what channel setup replaces: the same handshake,
/// carrying a ring rather than carrying nothing. RFC 0014.
const SYS_ANNOUNCE: u64 = door::ANNOUNCE;

/// "Have I run long enough?" Answers [`KEEP_GOING`], [`ENOUGH`] or
/// [`GAVE_UP`].
///
/// Replaced at M5 by a blocking wait on a ring, which is the same question
/// asked of something that can answer it without being polled.
const SYS_PROGRESS: u64 = door::PROGRESS;

/// "I am done." The first argument is a status.
///
/// The one of the three with no successor named, because it is the one a
/// process genuinely cannot do through a ring: submitting "I no longer exist"
/// and then waiting for the completion is not a sequence a process can finish.
const SYS_EXIT: u64 = door::EXIT;

/// "What is this handle?" Answers a packed kind and rights, or an authority
/// error.
///
/// The first of four capability calls, and RFC 0015 is the argument for why
/// they are calls at all when RFC 0014 says the door does not accumulate an
/// interface. In short: a ring is named by a `Channel` capability, so the table
/// has to work before there is any ring to work it through, and each of the
/// four names the opcode that retires it. This one becomes an opcode on the
/// component's control ring at M5.
pub(crate) const SYS_CAP_INSPECT: u64 = door::CAP_INSPECT;

/// "Mint me a weaker one." Takes a handle and a rights bitmap, answers a
/// handle.
///
/// Copy is the identity case — the same rights — and is a derivation like any
/// other, so that revoking the source reaches it. `kernel/src/cap.rs` argues
/// that against seL4. Retired by the same control-ring opcode at M5.
pub(crate) const SYS_CAP_DERIVE: u64 = door::CAP_DERIVE;

/// "Take back everything I handed on from this." Answers how many capabilities
/// were withdrawn. Retired at M5 with the rest.
pub(crate) const SYS_CAP_REVOKE: u64 = door::CAP_REVOKE;

/// "Map this frame into this address space." The one call that *uses* a
/// capability rather than managing one, and therefore the one that makes the
/// table load-bearing rather than decorative.
///
/// Arguments are packed because the door hands over two registers and this
/// needs four values: `rdi` is the frame handle in its low half and the address
/// space handle in its high half; `rsi` is a page-aligned address with the
/// requested rights in the twelve bits alignment leaves free. That packing is a
/// consequence of the door being deliberately narrow, and it goes away with the
/// call: at M5 this is an `Sqe`, whose `cap` field is the frame handle and
/// whose `ext` carries the rest with room to spare.
pub(crate) const SYS_CAP_MAP: u64 = door::CAP_MAP;

/// The answer to [`SYS_PROGRESS`] while the process should carry on.
const KEEP_GOING: u64 = door::KEEP_GOING as u64;

/// The answer once the frame has taken as many ticks from ring 3 as it wanted.
const ENOUGH: u64 = door::ENOUGH as u64;

/// The answer when the frame has given up waiting for those ticks.
///
/// Bounded in time rather than in calls, and read from the one clock the kernel
/// is allowed to read directly. A process that polls forever because the timer
/// stopped is a machine that hangs in boot with no output, which is the failure
/// `apic::wait` already refuses to have and this refuses for the same reason.
const GAVE_UP: u64 = door::GAVE_UP as u64;

/// What [`ring3::enter`] returns when the process was killed.
const KILLED: u64 = 1;

/// What it returns when the process asked to end.
const EXITED: u64 = 2;

/// How a process ended.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Death {
    /// It has not. Also the state before one is started.
    Running,
    /// It asked to end, with this status.
    Exited(u64),
    /// It did something it was not allowed to do.
    Killed {
        /// Which exception.
        vector: u64,
        /// The processor's error code.
        error: u64,
        /// The address a page fault was about, or zero.
        address: u64,
        /// Where in the process it happened.
        rip: u64,
    },
}

/// What the frame answered a process's capability calls with.
///
/// A count per refusal code rather than one total, because the negative suite's
/// whole content is *which* refusal each attempt earned. A run that refused the
/// right number of times for the wrong reasons is a run where the table is
/// saying no by accident, and a single counter cannot tell the two apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Tally {
    /// Calls the frame answered.
    pub ok: u32,
    /// The handle named no capability this process was given.
    pub no_such: u32,
    /// The capability was held and did not carry the right asked for.
    pub right_not_held: u32,
    /// The capability had been revoked.
    pub revoked: u32,
    /// The capability was of a kind the operation does not act on.
    pub wrong_type: u32,
    /// A quota was reached — the table is full, or an untyped region is spent.
    pub resource: u32,
    /// Anything else. Never expected, and counted rather than discarded so that
    /// an unexpected refusal shows up as a failed verdict instead of as a
    /// number that happens to still add up.
    pub other: u32,
}

impl Tally {
    /// Nothing asked, nothing answered.
    pub const ZERO: Self = Self {
        ok: 0,
        no_such: 0,
        right_not_held: 0,
        revoked: 0,
        wrong_type: 0,
        resource: 0,
        other: 0,
    };

    /// Count one answer, whichever it was.
    fn record(&mut self, answer: u64) {
        let signed = answer as i64;
        if signed >= 0 {
            self.ok += 1;
            return;
        }
        // The reply is the packed error widened to the register. Narrow it back
        // before unpacking, because `unpack` is defined on the wire width.
        let counter = match error::unpack(signed as i32) {
            Some((error::AUTHORITY, error::authority::NO_SUCH_CAP)) => &mut self.no_such,
            Some((error::AUTHORITY, error::authority::RIGHT_NOT_HELD)) => &mut self.right_not_held,
            Some((error::AUTHORITY, error::authority::REVOKED)) => &mut self.revoked,
            Some((error::AUTHORITY, error::authority::WRONG_TYPE)) => &mut self.wrong_type,
            Some((error::RESOURCE, _)) => &mut self.resource,
            _ => &mut self.other,
        };
        *counter += 1;
    }

    /// How many calls were refused, whatever the reason.
    #[must_use]
    pub const fn refused(&self) -> u32 {
        self.no_such
            + self.right_not_held
            + self.revoked
            + self.wrong_type
            + self.resource
            + self.other
    }
}

/// What the frame observed of a process while it ran.
#[derive(Clone, Copy)]
struct State {
    announced: bool,
    refused: u32,
    death: Death,
    /// Ticks the frame wants to take out of ring 3 before it answers
    /// [`ENOUGH`].
    wanted: u64,
    /// The counter value past which [`SYS_PROGRESS`] answers [`GAVE_UP`].
    giveup: u64,
    /// What its capability calls were answered with.
    caps: Tally,
    /// Its top-level page table.
    ///
    /// Here rather than only in [`Job`] because a capability call may have to
    /// edit the address space the caller is running in — withdrawing a mapping
    /// a revoked capability authorised — and the call arrives with a handle and
    /// nothing else. Zero when no process is running.
    root: u64,
    /// The frame allocator, as an address.
    ///
    /// An address and not a `*const FrameAllocator`, because [`PerCpu`] is
    /// `Sync` only for a `Send` payload and a raw pointer is not `Send`. The
    /// bound is the right one — it is what stops a slot holding something that
    /// must not cross cores — and a pointer here would be asking for an
    /// exception to it rather than needing one: this value never leaves the
    /// core that wrote it, and saying so in a comment is cheaper than widening
    /// the type's promise.
    ///
    /// Zero when no process is running, which is what makes the capability
    /// calls able to refuse rather than dereference.
    frames: usize,
    /// What the machine agreed to interpret, for the mappings a process asks
    /// for while it runs.
    features: paging::Features,
}

/// Per core, because a process runs on one and its faults arrive on that one.
static STATE: PerCpu<State> = PerCpu::new(State {
    announced: false,
    refused: 0,
    death: Death::Running,
    wanted: 0,
    giveup: 0,
    caps: Tally::ZERO,
    root: 0,
    frames: 0,
    features: paging::Features::NONE,
});

/// Timer ticks taken while ring 3 held the core.
///
/// The one piece of process state two paths touch: the interrupt handler writes
/// it and a system call reads it. It lives outside [`State`] and every access is
/// volatile through the raw pointer, for exactly the reason `apic::TICKS` does —
/// a reference here would be a claim that the handler and the code it
/// interrupted are not both looking at it, and they are.
static IN_RING3: PerCpu<u64> = PerCpu::new(0);

/// What crossed into the frame while ring 3 held a core.
///
/// # Why the five are counted apart rather than summed
///
/// Because they are five different events and only two of them are the
/// architecture's claim. RFC 0038 is the argument in full; the short version is
/// that a number whose exclusions are not written down is not a measurement.
///
/// [`Entries::hot`] and [`Entries::faults`] are **the hot path**: a boundary
/// crossing the code running at ring 3 caused, deliberately or otherwise, in the
/// middle of doing its work. [`Entries::boundary`] is the crossing that *is* the
/// allocation boundary — the one door call that ends the residency —
/// [`Entries::ticks`] is the frame's own clock reaching a core it gave away,
/// which is what makes preemption at an allocation boundary possible at all and
/// is not the runtime's work crossing anything, and [`Entries::interrupts`] is
/// the rest of what the frame sends a core it gave away.
///
/// # Why the fifth exists, which is a scar
///
/// It was not here when this landed, and its absence was the exact defect this
/// type exists to prevent one level down. Four buckets were counted, the
/// document said *nothing else is excluded*, and
/// [`interrupt_dispatch`](crate::arch::x86_64::idt::interrupt_dispatch) handled
/// three further vectors — the shootdown, the doorbell and the spurious one —
/// by returning without reading the saved code selector at all. Each of those
/// taken at ring 3 was a kernel entry in no bucket, so `total()` was not a
/// total. Nothing went red, because the demonstration's boot processor only
/// waits while the runtime runs and issues none of them; the `blk`, `cap` and
/// `user` boots do. A count that is complete only on the boot that reports it
/// is the shape of measurement this repository is written against.
///
/// All five are published. A reader who disagrees with where the line is drawn
/// can move it, which is the only honest way to ship an exclusion.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Entries {
    /// Door calls other than the one that ended the residency.
    /// Unit: kernel entries.
    pub hot: u64,
    /// Exceptions taken at ring 3. Unit: kernel entries.
    pub faults: u64,
    /// The door call that ended the residency, which is the allocation
    /// boundary. Never more than one, and a zero here on a run that ended by
    /// `EXIT` would mean the counting stopped.
    /// Unit: kernel entries.
    pub boundary: u64,
    /// Timer interrupts delivered while ring 3 held the core.
    /// Unit: kernel entries.
    pub ticks: u64,
    /// Every other interrupt delivered while ring 3 held the core: a TLB
    /// shootdown another core asked for, a doorbell, and the spurious vector
    /// the local APIC withdrew between asserting and acknowledging.
    ///
    /// Excluded from the hot path for [`Entries::ticks`]'s reason and not for a
    /// weaker one — every one of the three is the frame or another core
    /// reaching this one, and nothing the code at ring 3 does makes one happen.
    /// The spurious vector is the weakest member of that set and is counted
    /// with them rather than dropped, because a bucket with a judgement call in
    /// it is still a number and a vector in no bucket is not.
    /// Unit: kernel entries.
    pub interrupts: u64,
}

impl Entries {
    /// Nothing crossed. `Default` is not `const`, and every one of these is
    /// written into a `PerCpu` slot from a `const` context.
    pub const ZERO: Self = Self { hot: 0, faults: 0, boundary: 0, ticks: 0, interrupts: 0 };

    /// Crossings on the hot path: what the exit criterion requires to be zero.
    /// Unit: kernel entries.
    #[must_use]
    pub const fn on_the_hot_path(&self) -> u64 {
        self.hot + self.faults
    }

    /// Every crossing, including the three that are excluded from the hot path.
    ///
    /// Here so that the exclusion is subtractable rather than assumed: a reader
    /// who wants the unexcluded number has it. It is every ring-3 entry this
    /// kernel's dispatcher can take — which is a claim about
    /// [`interrupt_dispatch`](crate::arch::x86_64::idt::interrupt_dispatch)
    /// rather than about this arithmetic, and the day that function grows a
    /// sixth arm is the day this sentence stops being true unless the arm
    /// counts.
    /// Unit: kernel entries.
    #[must_use]
    pub const fn total(&self) -> u64 {
        self.hot + self.faults + self.boundary + self.ticks + self.interrupts
    }
}

/// Door calls a process made that were not the one that ended it.
///
/// Outside [`State`] and volatile through the raw pointer, for exactly
/// [`IN_RING3`]'s reason: the fault path writes the shard beside it and the
/// system-call path writes this one, and a reference here would be a claim that
/// the handler and the code it interrupted are not both looking at these words.
static HOT_CALLS: PerCpu<u64> = PerCpu::new(0);

/// Exceptions taken at ring 3 on this core.
static RING3_FAULTS: PerCpu<u64> = PerCpu::new(0);

/// Door calls that ended a residency. The allocation boundary, counted so that
/// its exclusion is a number rather than a sentence.
static BOUNDARY_CALLS: PerCpu<u64> = PerCpu::new(0);

/// Interrupts other than the timer taken while ring 3 held this core.
///
/// The shootdown, the doorbell and the spurious vector. Written by the
/// interrupt dispatcher on this core and by nobody else, which is
/// [`IN_RING3`]'s arrangement exactly.
static FRAME_INTERRUPTS: PerCpu<u64> = PerCpu::new(0);

/// Add one to a counting shard of this core's.
fn count(shard: &'static PerCpu<u64>) {
    let slot = shard.mine();
    // SAFETY: this core's slot, read and written volatile through the raw
    // pointer. The three writers on this core — the system-call path, the fault
    // path and the interrupt dispatcher — cannot interleave: `syscall` runs with
    // interrupts masked by `IA32_FMASK`, so no interrupt arrives inside a door
    // call; every gate in this kernel's IDT is an interrupt gate, so a handler
    // cannot be interrupted or interrupt itself; and a fault at ring 3 cannot
    // arrive while ring 0 is inside a call. Volatile because the compiler may
    // not merge or elide a count that another privilege level's behaviour is
    // being judged by.
    let value = unsafe { slot.read_volatile() };
    // SAFETY: as above.
    unsafe { slot.write_volatile(value.wrapping_add(1)) };
}

/// Start counting `cpu`'s crossings from zero.
///
/// Called where [`IN_RING3`] is zeroed and for the same reason: a residency's
/// count is about that residency.
fn arm_entries(cpu: usize) {
    for shard in [&HOT_CALLS, &RING3_FAULTS, &BOUNDARY_CALLS, &FRAME_INTERRUPTS] {
        let slot = shard.at(cpu);
        // SAFETY: the slot of an idle core with no process on it, so neither
        // writer over there can be running. Volatile, as every access to these
        // shards is.
        unsafe { slot.write_volatile(0) };
    }
}

/// Read this core's crossings.
///
/// # Safety
///
/// Call on the core that ran the process, with the process over — which is what
/// makes reading these shards free of a writer.
unsafe fn entries_here(ticks: u64) -> Entries {
    // SAFETY: the caller's guarantee, and volatile as these shards require.
    let hot = unsafe { HOT_CALLS.mine().read_volatile() };
    // SAFETY: as above.
    let faults = unsafe { RING3_FAULTS.mine().read_volatile() };
    // SAFETY: as above.
    let boundary = unsafe { BOUNDARY_CALLS.mine().read_volatile() };
    // SAFETY: as above.
    let interrupts = unsafe { FRAME_INTERRUPTS.mine().read_volatile() };
    Entries { hot, faults, boundary, ticks, interrupts }
}

/// What a system call produced.
pub enum Answer {
    /// A value for `rax`, and the process carries on.
    Reply(u64),
    /// The process is over, with this outcome for [`ring3::enter`] to return.
    Ended(u64),
}

/// Which violation the process is to commit.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Provoke {
    /// Read the kernel's direct map.
    Kernel,
    /// Write to the page at address zero.
    Null,
    /// Write to its own text.
    Text,
    /// Execute its own stack.
    Stack,
    /// Execute an instruction only ring 0 may.
    Privileged,
    /// Make a call the frame does not have, and end normally afterwards.
    Call,
    /// Provoke nothing; ask to end.
    Exit,

    // The capability escapes. Seven, like the isolation provocations above and
    // for the same reason: a property nothing tries to violate is a property
    // nobody has checked. These are the five from
    // `docs/design/ring-scene-boot.html` section 15 M4 — E0-P08 — plus the
    // positive control that stops the other six passing for the wrong reason,
    // and the exhaustion case that is what "cannot panic the kernel by trying"
    // most often means in practice.
    /// Use the capabilities it was given, correctly. Nothing is refused.
    Grant,
    /// Name a slot the frame never filled.
    Unowned,
    /// Sweep the handle space: every slot, several generations, and four words
    /// nobody could have issued.
    Forge,
    /// Derive twice, revoke the root of the tree, and keep using the leaves.
    Stale,
    /// Ask for a right the capability does not carry, twice over.
    Rights,
    /// Present a capability of the wrong kind for the operand.
    Mistyped,
    /// Derive until the table is full.
    Flood,
    /// Map a frame, have the capability behind it revoked, and read the page
    /// anyway.
    ///
    /// The eighth, and the only one of them that is supposed to end in a fault
    /// rather than a refusal — because what it tests is not whether the frame
    /// says no, it is whether the *processor* does. Every other capability
    /// escape is answered by the table; this one is answered by a page table
    /// entry that is no longer there, on a core that has been told it is no
    /// longer there. E0-B10.
    Unmap,
    /// Store into the state tree it was granted read-only.
    ///
    /// The one provocation whose target exists to be *read*. E0-B14 grants
    /// every process the frame's published tree without `WRITE`, and a mapping
    /// that is read-only only in intention would be discovered by the first
    /// component that scribbled on the kernel's own counters. The process maps
    /// it, reads it — which the preamble already did, so the fault below cannot
    /// be the page merely being absent — and then writes.
    State,

    // The two E1-B13 added, and both are about the same surface: a table that
    // can be bought is a table with a second way to be asked for something it
    // cannot give. `Flood` is now the run that *does* buy — it fills the free
    // part, pays for a page and fills that — so it is these two that say what
    // happens at the two edges of paying.
    /// Spend the untyped region on something else, then derive until the table
    /// is full and there is nothing left to buy a page with.
    ///
    /// The refusal has to be `RESOURCE/QUOTA_EXHAUSTED` and the table has to
    /// stop at the size it was given: a frame that served this out of anything
    /// it kept in reserve would pass every other check in this file and would
    /// have made a component's failure depend on what every other component had
    /// spent. RFC 0008 and RFC 0029.
    Quota,
    /// Buy a page, then name slots past the end of what was bought.
    ///
    /// Three handles: one past the bought table, one past the ceiling this
    /// build will ever grow a table to, and the largest index the packing can
    /// express. Before growth, "past the end" was a constant; it is now a
    /// number a component chooses by spending, and a bound that moves is a
    /// bound worth a boot of its own.
    Beyond,
}

/// What a provocation is supposed to produce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expect {
    /// A fault at this vector, and the process killed.
    Fault(u64),
    /// A clean exit, with this many calls refused on the way.
    Exit(u32),
}

/// How many capabilities the frame hands a process at M4.
///
/// Its address space, one frame, one untyped region, and the frame's published
/// state tree — in that order, so their handles are the first four slots and the
/// process can be written against them. Four, and every one of them is something
/// the process needs and nothing more: there is no capability here for the frame
/// it is running out of, because a process that could remap its own text is a
/// process for which write-exclusive-or-execute is advisory.
///
/// The fourth is `E0-B14`'s, and it is the first grant in this system that
/// exists so a process can *observe* rather than so it can act. It carries no
/// `WRITE`, which `cap=state` is the boot that tries.
pub const GRANTS: usize = 4;

/// Capability calls every process makes before it does whatever it was told to.
///
/// Inspect the frame capability, derive a copy of it, map the copy, then map
/// the state tree. Four, and they are the positive path: a process that cannot
/// use a capability correctly cannot meaningfully fail to abuse one, and a suite
/// of nothing but refusals passes on a frame that refuses everything.
///
/// The fourth maps the granted handle directly rather than deriving a copy of
/// it first, and that is not an inconsistency: the derive in the third call is
/// there to be exercised, and doing it twice would exercise it twice while
/// making every live-handle count in this file one larger for no reason.
const PREAMBLE_OK: u32 = 4;

/// Generations the forging sweep tries at each slot.
///
/// Four. Slots in use are at the first generation, so this is three wrong
/// answers per slot against one right one — enough that a table which ignored
/// the generation would be caught at the very first slot.
pub(crate) const SWEEP_GENERATIONS: u32 = 4;

/// Handles in the sweep that no table could have issued: the zero word, one
/// past the last slot, the largest index expressible, and all ones.
const SWEEP_WILD: u32 = 4;

/// How many of the sweep's handles resolve: the three grants and the one the
/// preamble derived, each at the generation it was issued at.
const SWEEP_LIVE: u32 = GRANTS as u32 + 1;

/// How many the sweep is refused, which is all of the rest.
const SWEEP_REFUSED: u32 = TABLE_SLOTS as u32 * SWEEP_GENERATIONS + SWEEP_WILD - SWEEP_LIVE;

/// How many capabilities a flooding process mints before it runs out of money.
///
/// Every free slot, then every slot on the one page its untyped region can buy.
/// That this is a number at all is the point, and since E1-B13 it is a number
/// about a *quota* rather than about an array: the bound is what the process was
/// handed to spend, it is reached with an error rather than a fault, and the
/// error names a resource.
///
/// The untyped region the frame grants is one frame, so it buys exactly one
/// page. A process holding more would flood further, which is the whole claim.
const FLOOD_MINTS: u32 = (TABLE_SLOTS + SLOTS_PER_PAGE) as u32 - SWEEP_LIVE;

/// How many a process mints when it has already spent its untyped region.
///
/// Every free slot and not one more. `cap=quota` derives a frame out of its
/// untyped region first — which consumes the whole of it, the region being one
/// frame — so when the table fills there is nothing to buy a page with and the
/// refusal arrives at the size the frame handed over. The difference between
/// this and [`FLOOD_MINTS`] is the evidence that growth is paid for rather than
/// free.
const QUOTA_MINTS: u32 = TABLE_SLOTS as u32 - SWEEP_LIVE;

/// Handles `cap=beyond` presents that name no slot in a table that has bought
/// one page: one past the bought end, one past the ceiling any table can reach,
/// and the largest index the packing expresses.
const BEYOND_WILD: u32 = 3;

/// The page fault, which is what four of the seven provocations produce.
const PAGE_FAULT: u64 = 14;

/// The general protection fault, which is what a privileged instruction at ring
/// 3 produces.
const GENERAL_PROTECTION: u64 = 13;

impl Provoke {
    /// Which provocation the command line asked for.
    ///
    /// [`Provoke::Kernel`] by default, so that an ordinary boot exercises the
    /// isolation rather than only the transition. It is the one violation whose
    /// failure would be worst: reading kernel memory from ring 3 is not a crash,
    /// it is a quiet success.
    #[must_use]
    pub fn chosen(boot: &BootInfo) -> Self {
        for (parameter, provoke) in [
            (&b"user=null"[..], Self::Null),
            (&b"user=text"[..], Self::Text),
            (&b"user=stack"[..], Self::Stack),
            (&b"user=priv"[..], Self::Privileged),
            (&b"user=call"[..], Self::Call),
            (&b"user=exit"[..], Self::Exit),
            (&b"user=kernel"[..], Self::Kernel),
            (&b"cap=grant"[..], Self::Grant),
            (&b"cap=unowned"[..], Self::Unowned),
            (&b"cap=forge"[..], Self::Forge),
            (&b"cap=stale"[..], Self::Stale),
            (&b"cap=rights"[..], Self::Rights),
            (&b"cap=type"[..], Self::Mistyped),
            (&b"cap=flood"[..], Self::Flood),
            (&b"cap=unmap"[..], Self::Unmap),
            // Absent until E1-B13, which is why `cargo xtask cap` had a ninth
            // boot that ran the default provocation and passed. A boot whose
            // name selects nothing is worse than a missing boot: the suite
            // reports it green.
            (&b"cap=state"[..], Self::State),
            (&b"cap=quota"[..], Self::Quota),
            (&b"cap=beyond"[..], Self::Beyond),
        ] {
            if boot.has_parameter(parameter) {
                return provoke;
            }
        }
        Self::Kernel
    }

    /// Which violation this is, as the process is told it.
    ///
    /// Half of what the process is handed: [`f_abi::door::Entry`] carries this
    /// in its low half and the first granted capability in its high one.
    #[must_use]
    pub const fn selector(self) -> u32 {
        match self {
            Self::Kernel => probe::PROVOKE_KERNEL as u32,
            Self::Null => probe::PROVOKE_NULL as u32,
            Self::Text => probe::PROVOKE_TEXT as u32,
            Self::Stack => probe::PROVOKE_STACK as u32,
            Self::Privileged => probe::PROVOKE_PRIV as u32,
            Self::Call => probe::PROVOKE_CALL as u32,
            Self::Exit => probe::PROVOKE_EXIT as u32,
            Self::Grant => probe::PROVOKE_CAP_GRANT as u32,
            Self::Unowned => probe::PROVOKE_CAP_UNOWNED as u32,
            Self::Forge => probe::PROVOKE_CAP_FORGE as u32,
            Self::Stale => probe::PROVOKE_CAP_STALE as u32,
            Self::Rights => probe::PROVOKE_CAP_RIGHTS as u32,
            Self::Mistyped => probe::PROVOKE_CAP_TYPE as u32,
            Self::Flood => probe::PROVOKE_CAP_FLOOD as u32,
            Self::Unmap => probe::PROVOKE_CAP_UNMAP as u32,
            Self::State => probe::PROVOKE_CAP_STATE as u32,
            Self::Quota => probe::PROVOKE_CAP_QUOTA as u32,
            Self::Beyond => probe::PROVOKE_CAP_BEYOND as u32,
        }
    }

    /// A phrase for the boot log.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Kernel => "a read of the kernel's direct map",
            Self::Null => "a write to the null page",
            Self::Text => "a write to its own text",
            Self::Stack => "an execute from its own stack",
            Self::Privileged => "an instruction only ring 0 may run",
            Self::Call => "a call the frame does not have",
            Self::Exit => "nothing; it asks to end",
            Self::Grant => "nothing; it uses the capabilities it was given",
            Self::Unowned => "naming a capability it was never given",
            Self::Forge => "forging handles, over the whole table and past it",
            Self::Stale => "using a capability after it was revoked",
            Self::Rights => "asking for rights its capability does not carry",
            Self::Mistyped => "presenting a capability of the wrong kind",
            Self::Flood => "deriving until the table is full",
            Self::Unmap => "reading a page after the capability that mapped it was revoked",
            Self::State => "writing to the state tree it was granted read-only",
            Self::Quota => "filling its table with nothing left to buy more with",
            Self::Beyond => "naming slots past the end of the table it bought",
        }
    }

    /// What has to happen for the provocation to have been provoked.
    #[must_use]
    pub const fn expects(self) -> Expect {
        match self {
            // Four isolation provocations and one capability one. `Unmap` is
            // in this list rather than below it because a revoked mapping is
            // supposed to stop being a mapping: the refusal it earns is a page
            // fault, from the processor, and an exit would mean the page was
            // still there.
            Self::Kernel | Self::Null | Self::Text | Self::Stack | Self::Unmap | Self::State => {
                Expect::Fault(PAGE_FAULT)
            }
            Self::Privileged => Expect::Fault(GENERAL_PROTECTION),
            Self::Call => Expect::Exit(1),
            // Every capability escape is refused rather than fatal, which is
            // the fifth property: a process cannot make the kernel panic by
            // trying. So all seven end the same way the process that does
            // nothing wrong does — cleanly, with nothing left of them.
            Self::Exit
            | Self::Grant
            | Self::Unowned
            | Self::Forge
            | Self::Stale
            | Self::Rights
            | Self::Mistyped
            | Self::Flood
            | Self::Quota
            | Self::Beyond => Expect::Exit(0),
        }
    }

    /// Exactly what the frame must have answered this process's capability
    /// calls with.
    ///
    /// The negative suite, as numbers. Every provocation runs [`PREAMBLE`]
    /// first — a process that cannot use a capability correctly cannot
    /// meaningfully fail to abuse one — so every expectation here starts from
    /// it.
    ///
    /// These are exact and not lower bounds. A run that refused more than this
    /// is as wrong as one that refused less: it means the frame turned down
    /// something the process was entitled to, and a capability system that is
    /// too strict fails silently, as a component that mysteriously does not
    /// work.
    /// `generation` is the one this process's capabilities were granted at,
    /// which is one more than the number of processes that ran on this core
    /// before it. It is a parameter rather than a constant because of the
    /// forging sweep: a handle at a *lower* generation than the slot holds is
    /// refused as revoked rather than as unknown, and that is not an accident
    /// of the encoding — it is the frame saying "you had this once", which is
    /// exactly what a stale handle from the previous process on this core is.
    /// A suite that expected the same tally whatever had run before would be a
    /// suite that could not tell those two refusals apart.
    #[must_use]
    pub const fn expects_caps(self, generation: u16) -> Tally {
        /// `(ok, no_such, right_not_held, revoked, wrong_type, resource)`, with
        /// `other` always zero — an answer this suite did not name is a failure
        /// however many of them there are.
        const fn tally(
            ok: u32,
            no_such: u32,
            right_not_held: u32,
            revoked: u32,
            wrong_type: u32,
            resource: u32,
        ) -> Tally {
            Tally { ok, no_such, right_not_held, revoked, wrong_type, resource, other: 0 }
        }
        let base = PREAMBLE_OK;
        match self {
            // Six of the isolation provocations never make a capability call
            // beyond the preamble; the seventh makes an unknown *opcode*, which
            // is a different counter.
            Self::Kernel
            | Self::Null
            | Self::Text
            | Self::Stack
            | Self::Privileged
            | Self::Call
            | Self::Exit => tally(base, 0, 0, 0, 0, 0),

            // Derive weaker, then read back what came out. The control: if this
            // does not pass, every refusal below might be the frame refusing
            // everything.
            Self::Grant => tally(base + 2, 0, 0, 0, 0, 0),

            // A slot the frame never filled, inspected and then used.
            Self::Unowned => tally(base, 2, 0, 0, 0, 0),

            // Four generations over every slot, plus four words nobody issued.
            // Five handles are live at that point — the four grants and the one
            // the preamble derived — so exactly five of the hundred and
            // thirty-two resolve, at the generation this process was granted at.
            //
            // Every generation *below* that one, on those same four slots, is a
            // handle the previous process on this core held. It is refused as
            // revoked, and counting it separately is what makes the boot say so.
            Self::Forge => {
                let stale = SWEEP_LIVE * (generation as u32 - 1);
                tally(base + SWEEP_LIVE, SWEEP_REFUSED - stale, 0, stale, 0, 0)
            }

            // Derive a grandchild, revoke the root, then use both leaves.
            Self::Stale => tally(base + 2, 0, 0, 2, 0, 0),

            // Widen by derivation, then map more permissively than the
            // capability allows.
            Self::Rights => tally(base, 0, 2, 0, 0, 0),

            // A space where a frame belongs, then a frame where a space does.
            Self::Mistyped => tally(base, 0, 0, 0, 2, 0),

            // Every free slot, then the page its untyped region pays for, then
            // the refusal. Exactly one refusal: a table that could keep buying
            // would never reach it, and one that never bought would reach it
            // [`SLOTS_PER_PAGE`] mints early.
            Self::Flood => tally(base + FLOOD_MINTS, 0, 0, 0, 0, 1),

            // The retype that empties the untyped region, then every free slot,
            // then the refusal that says the account is empty.
            Self::Quota => tally(base + QUOTA_MINTS, 0, 0, 0, 0, 1),

            // The flood, and then three handles past the end of what it bought.
            Self::Beyond => tally(base + FLOOD_MINTS, BEYOND_WILD, 0, 0, 0, 1),

            // The preamble's three, and the revoke that withdraws what the
            // third of them mapped. Nothing is refused: the frame answers every
            // call this process makes, and what stops it is the page fault the
            // fourth answer caused.
            Self::Unmap => tally(base + 1, 0, 0, 0, 0, 0),

            // Nothing is refused: the frame answers every call this process
            // makes, and what stops it is the store the *processor* refuses. A
            // capability error here would mean the mapping never happened,
            // which is the fault passing for the wrong reason.
            Self::State => tally(base, 0, 0, 0, 0, 0),
        }
    }

    /// Whether this provocation needs no-execute to mean anything.
    ///
    /// Only one does, and on a machine whose firmware turned the feature off
    /// there is nothing to provoke — the same situation `fault=nx` reports
    /// rather than pretending it tested something.
    #[must_use]
    pub const fn needs_no_execute(self) -> bool {
        matches!(self, Self::Stack)
    }
}

/// Why a process could not be built or did not behave.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Error {
    /// Its address space could not be built.
    Space(paging::BuildError),
    /// There was no frame for its text or its stack.
    NoFrames,
    /// The program does not fit in one page. The frame maps one page of text,
    /// so a program that outgrows it needs a loader that reads its headers
    /// rather than a larger constant — E5. `cargo xtask init` checks the same
    /// bound at build time, so a component reaching this is one that arrived
    /// some other way.
    TooLarge,
    /// There is no program to run: a module of no length, or an empty slice
    /// where a caller meant to pass one. Refused rather than run, because a
    /// process entered at a page of zeroes executes whatever a zero byte means
    /// on this architecture and reports something inexplicable.
    NoProgram,
    /// The process ended and nothing recorded how, which is a bug in the frame
    /// rather than in the process.
    NoDeath,
    /// The capability table had no room for the frame's own grants. A bug in
    /// the frame — it grants three into an empty table of thirty-two — and
    /// reported rather than ignored, because a process that starts without one
    /// of its capabilities fails later and somewhere else.
    NoSlot,
    /// The free count did not come back. Something the process owned was not
    /// given back, and continuing would hide it.
    Leaked,
    /// The core the process was handed to could not arm its own timer, so there
    /// was nothing to count ticks out of ring 3 with and the process would have
    /// polled forever.
    NoTimer,
}

impl Error {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Space(inner) => inner.message(),
            Self::NoFrames => "no frame for the process's text or stack",
            Self::TooLarge => "the process's program does not fit in one page",
            Self::NoProgram => "there is no program for the process to run",
            Self::NoSlot => "the capability table had no room for a process's own grants",
            Self::NoDeath => "the process ended and the frame did not record how",
            Self::Leaked => "a process's frames were not all given back",
            Self::NoTimer => "the core a process was handed to could not arm its timer",
        }
    }
}

/// What one run of a process produced.
#[derive(Clone, Copy)]
pub struct Report {
    /// Which core ran it.
    pub cpu: usize,
    /// The physical address of its top-level table.
    pub root: u64,
    /// How many of the kernel's top-level slots it carried a copy of.
    pub shared_slots: usize,
    /// Frames it was made of, tables and pages together.
    pub frames: u64,
    /// Whether it announced itself.
    pub announced: bool,
    /// How many calls the frame refused.
    pub refused: u32,
    /// Timer ticks taken while it held the core at ring 3.
    pub ticks: u64,
    /// How it ended.
    pub death: Death,
    /// How many capabilities the frame granted it.
    pub granted: usize,
    /// The generation those capabilities were issued at.
    ///
    /// One more than the number of processes that have run on this core, and
    /// therefore the number that says how much of this core's history a handle
    /// could be stale by. [`Provoke::expects_caps`] is the only reader.
    pub generation: u16,
    /// What its capability calls were answered with.
    pub caps: Tally,
    /// Capabilities still in its table when it ended, before the table was
    /// cleared. Everything it derived and did not have revoked.
    pub held: usize,
    /// What crossed into the frame while it held the core, in four buckets.
    ///
    /// Counted on every run and not only on a runtime's, because a counter that
    /// exists on one path is a counter nobody has compared against anything:
    /// `user/init` makes six door calls and the probe makes more, so a build in
    /// which this had stopped counting would publish a zero for them too and be
    /// caught by the same check that requires a runtime's to be zero.
    pub entries: Entries,
}

impl Report {
    /// Whether the provocation provoked what it was supposed to.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold. Every one of them is a failed boot:
    /// a protection that did not fire is not a smaller result than a fault, it
    /// is the opposite result.
    pub fn verdict(&self, provoke: Provoke, wanted: u64) -> Result<(), &'static str> {
        if !self.announced {
            return Err("the process never announced itself, so ring 3 never ran");
        }
        if self.ticks < wanted {
            return Err("the frame gave up before it had taken its ticks out of ring 3");
        }

        // The capability half, checked before the fault half and separately
        // from it. Separately because they fail differently: a wrong tally is
        // an authority answer the frame got wrong, and a wrong death is a
        // protection that did not hold. Before, because the tally covers the
        // preamble every process runs — so a tally that is wrong for a `user=`
        // boot means the capability path broke, and reporting that as the
        // isolation provocation failing would send the reader to the wrong file.
        let expected = provoke.expects_caps(self.generation);
        if self.caps != expected {
            if self.caps.ok != expected.ok {
                return Err("the frame answered a different number of capability calls than the \
                            provocation makes");
            }
            if self.caps.refused() != expected.refused() {
                return Err("the frame refused a different number of capability calls than the \
                            provocation earns");
            }
            if self.caps.other != 0 {
                return Err("a capability call was refused with something outside the authority \
                            and resource domains");
            }
            return Err("the frame refused the right number of capability calls for the wrong \
                        reasons");
        }

        match (provoke.expects(), self.death) {
            (Expect::Fault(vector), Death::Killed { vector: got, .. }) if got == vector => Ok(()),
            (Expect::Fault(_), Death::Killed { .. }) => {
                Err("the process faulted, but not with the exception the provocation names")
            }
            (Expect::Fault(_), _) => {
                Err("the process was not killed — the provocation did not provoke")
            }
            (Expect::Exit(refused), Death::Exited(0)) if refused == self.refused => Ok(()),
            (Expect::Exit(_), Death::Exited(0)) => {
                Err("the process ended cleanly, having been refused a different number of times")
            }
            (Expect::Exit(_), Death::Exited(_)) => {
                Err("the process ended with a status it only reports when a protection held")
            }
            (Expect::Exit(_), _) => Err("the process was killed where it should have ended itself"),
        }
    }
}

/// What a core has been asked to run.
///
/// Separate from [`State`] because the two are written by different cores and
/// read at different times: this is the boot processor telling a core what to
/// do, and [`State`] is that core's record of what happened. Merging them would
/// mean one struct half of which is stale on whichever core is looking at it.
#[derive(Clone, Copy)]
struct Job {
    /// The process's top-level table, which is what goes in `CR3`.
    root: u64,
    /// Where its program starts.
    entry: u64,
    /// The stack pointer it starts with.
    stack: u64,
    /// The one word it is told on entry: which provocation to commit.
    argument: u64,
    /// The rate the running core arms its own timer at.
    hz: u32,
    /// How many ticks that timer asks for. It is a bound rather than a
    /// schedule: the process ends long before it is reached, and the run is
    /// stopped at that point.
    target: u64,
}

/// Per core, because a core runs one process at a time and is told about it
/// before it starts.
static JOB: PerCpu<Job> =
    PerCpu::new(Job { root: 0, entry: 0, stack: 0, argument: 0, hz: 0, target: 0 });

/// What the core that ran a process found out, for the core that prepared it.
///
/// Everything here is in the running core's own shard and is read by the boot
/// processor only after that core has said it is finished — which it says with
/// a `Release` store, so these writes are visible to the `Acquire` that reads
/// it. `smp` owns that pair and argues for it.
#[derive(Clone, Copy)]
struct Outcome {
    /// [`KILLED`], [`EXITED`], or zero if the process never started.
    ended: u64,
    /// Timer ticks the running core took out of ring 3.
    ticks: u64,
    /// Capabilities still in its table when it ended, before it was cleared.
    held: usize,
    /// What crossed into the frame while it held the core.
    ///
    /// Here rather than in [`State`] because it has to cross a core boundary,
    /// and this is the structure that already does — published by the running
    /// core's `Release` store and read through the `Acquire` that answers it.
    entries: Entries,
    /// Why the core could not run it, if it could not.
    failed: Option<Error>,
}

/// Per core, and written only by the core that ran the process.
static OUTCOME: PerCpu<Outcome> =
    PerCpu::new(Outcome { ended: 0, ticks: 0, held: 0, entries: Entries::ZERO, failed: None });

/// What the frame is asking of one run of a process.
///
/// Five values that all say the same kind of thing — what this run is for —
/// and they are a struct because [`prepare`] otherwise takes eight arguments,
/// three of which are numbers of the same type. A call site that passes `hz`
/// where `target` goes is a boot that waits for a thousand ticks or arms a
/// timer at a hundred hertz, and nothing about either would look wrong.
#[derive(Clone, Copy)]
pub struct Plan {
    /// The program to run.
    ///
    /// Two of them exist. `arch::x86_64::probe` is the frame's own adversary,
    /// assembled into the kernel image, and it is what every `user=` and `cap=`
    /// boot runs — it has to be in the image, because a suite that could only
    /// test a component somebody supplied would be a suite that stops working
    /// when nobody supplies one. The other is whatever the loader placed in
    /// memory, which from E0-B10 is `user/init`.
    ///
    /// Making it a parameter rather than a switch is the whole of what the
    /// loader changed here: the frame now runs *a* program, and where it came
    /// from is the caller's business.
    pub program: &'static [u8],
    /// Which violation the process is to commit.
    pub provoke: Provoke,
    /// The physical address of the frame the state tree is published in.
    /// Unit: bytes, physical.
    pub tree: u64,
    /// How many timer ticks the frame takes out of ring 3 before it tells the
    /// process it has run long enough.
    pub wanted: u64,
    /// The rate the core running it arms its own timer at.
    pub hz: u32,
    /// How many ticks that timer asks for. A bound rather than a schedule: the
    /// process ends long before it is reached.
    pub target: u64,
    /// Which core is to run it.
    pub cpu: usize,
}

/// A process that has been built and not yet run, and the memory it will have
/// to give back.
///
/// Held by the core that prepared it rather than by the core that runs it, and
/// that split is the whole shape of this milestone. Allocating and freeing are
/// the frame allocator's, and the frame allocator belongs to one core; running
/// is a core taking a privilege-level transition, and that can be any core. So
/// the boot processor builds the process, another core runs it, and the boot
/// processor gives it back.
pub struct Prepared {
    space: paging::UserSpace,
    /// The pages this process is made of, in the order they were taken and
    /// every one of them owed back.
    ///
    /// Four for an ordinary process — text, stack, the frame behind its frame
    /// capability, and the region behind its untyped one. Four for a runtime
    /// too, and different ones: text, stack, its control ring and its work
    /// ring. The array is sized for the larger of the two shapes and
    /// [`Prepared::parts`] says how much of it is real, because a list whose
    /// length is a constant is a list that silently frees a frame nobody took
    /// the day the shapes stop agreeing.
    pages: [Frame; PARTS_MAX],
    /// How many of `pages` were taken. Unit: frames.
    parts: usize,
    /// The free count before any of it was taken.
    before: u64,
    /// How many capabilities the frame put in its table.
    granted: usize,
    /// The generation it granted them at.
    generation: u16,
    /// Which core is to run it.
    cpu: usize,
}

/// The most pages any process shape is made of.
///
/// Four, in both shapes. It is a maximum rather than the count so that a shape
/// with five is a change to one constant and a `parts` that is already carried,
/// rather than a change to every literal in this file — but it is the *real*
/// maximum and not a round number above it. It was six, with both shapes
/// padding the list with two null frames to fill it, so the constant, the
/// sentence beside it and the code disagreed about what was being reserved and
/// why. A ceiling nobody reaches teaches a reader the wrong number.
const PARTS_MAX: usize = 4;

/// Build a process on `cpu`'s behalf: an address space, four pages, a table of
/// capabilities and a job.
///
/// Nothing runs as a result of this. The core named by `cpu` is left holding
/// everything it needs and nothing has told it to start, which is
/// [`crate::smp::run_on`]'s to do.
///
/// `wanted` is how many timer ticks the frame takes out of ring 3 before it
/// tells the process it has run long enough.
///
/// # Why another core's shards are written here
///
/// Because they cannot be written by their owner. A core cannot build its own
/// first process for the same reason it cannot start itself: everything it
/// would need to do it with — the allocator, the kernel's address space, the
/// program — is reachable only from a core that is already running. `PerCpu::at`
/// exists for exactly this case and says so.
///
/// # Errors
///
/// [`Error`], every variant of which fails the boot.
///
/// # Safety
///
/// Call on the boot processor, with the kernel's address space in `CR3`,
/// `frames` rebound onto its direct map, and `cpu` a core that is up and idle.
/// The `&mut` on `frames` must not be used again until [`reap`]: its address is
/// handed to the running core, which reads through it while the process is
/// alive.
pub unsafe fn prepare(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    plan: Plan,
) -> Result<Prepared, Error> {
    let Plan { program, provoke, tree, wanted, hz, target, cpu } = plan;
    if program.is_empty() {
        return Err(Error::NoProgram);
    }
    if program.len() as u64 > FRAME_SIZE {
        return Err(Error::TooLarge);
    }

    let before = frames.free_count();

    // SAFETY: the caller's guarantee that the kernel's space is live and that
    // frames are addressable through its direct map.
    let mut space = unsafe { paging::user_space(frames, kernel) }.map_err(Error::Space)?;

    // Zeroed, both of them, and for two different reasons. The text frame is
    // about to be overwritten by a program that is shorter than a page, so
    // whatever the last owner left would be executable tail; the stack frame is
    // memory a component is being handed, which `mem::alloc_zeroed` documents as
    // the case that must never carry the previous owner's bytes.
    let text = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    let stack = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    // Two frames the process owns and neither of which is mapped for it. The
    // first is what its frame capability names — it becomes reachable only by
    // presenting that capability — and the second is the region behind its
    // untyped capability, which is what a retype mints out of. Both are zeroed
    // for the reason `mem::alloc_zeroed` documents: memory handed to a
    // component must never carry the previous owner's bytes.
    let granted = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    let untyped = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;

    let into = frames.virt(text);
    // SAFETY: `text` was just allocated and nothing else holds it; it is one
    // frame, addressable through the direct map, and the program is shorter
    // than one — checked above rather than assumed.
    unsafe { core::ptr::copy_nonoverlapping(program.as_ptr(), into, program.len()) };

    // SAFETY: as `user_space`, and `space` is not in `CR3` — it has never been.
    unsafe {
        paging::map_user(frames, &mut space, TEXT, text.addr(), paging::UserPage::Text, features)
    }
    .map_err(Error::Space)?;
    // SAFETY: as above.
    unsafe {
        paging::map_user(frames, &mut space, STACK, stack.addr(), paging::UserPage::Data, features)
    }
    .map_err(Error::Space)?;

    // The table, before the process exists to reach it. Four grants, in the
    // order the process is written against, and nothing else will ever be put
    // in from this side: everything the table holds after this line is
    // something the process derived.
    let table = crate::cap::of(cpu);
    // SAFETY: the table of a core that is idle, with no process running on it —
    // so neither the system-call path nor the fault path over there can be
    // holding a reference to it. This is the write `PerCpu::at` exists for.
    let held = unsafe { &mut *table };
    held.clear_all();
    let space_rights = rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE;
    // The first grant, and the one the process is told about: everything else it
    // holds follows from this handle by index. `f_abi::door::Entry` argues why
    // it is told rather than left to assume, and the answer is this function
    // running twice on one core — the second process finds the same slots at a
    // later generation.
    let first = held
        .grant(CapType::AddressSpace, space_rights, space.root(), 0)
        .map_err(|_| Error::NoSlot)?;
    // Deliberately without `WRITE`, and it is the whole of the rights half of
    // the negative suite: a process that could map this writable would have
    // exceeded what it was granted, and `cap=rights` is the run that tries.
    let frame_rights = rights::READ | rights::DERIVE | rights::REVOKE;
    held.grant(CapType::Frame, frame_rights, granted.addr(), FRAME_SIZE)
        .map_err(|_| Error::NoSlot)?;
    held.grant(CapType::Untyped, space_rights, untyped.addr(), FRAME_SIZE)
        .map_err(|_| Error::NoSlot)?;
    // The state tree, read-only and never freed. Not in `pages` for exactly
    // that reason: it is machine-wide and outlives every process that reads it,
    // and a tree a `reap` could give back would be a mapping a reader still
    // holds. Rights without `WRITE`, and `cap=state` is the boot that tries.
    held.grant(CapType::Frame, frame_rights, tree, FRAME_SIZE).map_err(|_| Error::NoSlot)?;
    let granted_count = held.used();

    let state = STATE.at(cpu);
    // SAFETY: the slot of an idle core, so neither the fault path nor the
    // system-call path over there can be holding it.
    unsafe {
        state.write(State {
            announced: false,
            refused: 0,
            death: Death::Running,
            wanted,
            // Filled in by the core that runs it, out of the timer window it
            // opens: a give-up bound is a deadline, and a deadline computed on
            // one core for a window that has not been opened on another is a
            // number about the wrong interval.
            giveup: 0,
            caps: Tally::ZERO,
            root: space.root(),
            // An address derived from the caller's `&mut`, which is not used
            // again until `reap` — so the borrow it came from is dormant for
            // exactly as long as the running core may use it. That is the whole
            // of why a process may reach the frame allocator at all, and why it
            // is put back to zero when the process ends.
            //
            // It crosses a core now, which the single-core version did not have
            // to say anything about. What makes it sound is that nothing on
            // either side *mutates* through it while the process is alive:
            // `map_user_live` and `unmap_user_live` both take a shared
            // reference, the allocator is untouched on this core from here
            // until `reap`, and the two cores are separated at both ends by the
            // release-acquire pair in `smp`.
            frames: core::ptr::from_ref::<FrameAllocator>(frames) as usize,
            features,
        });
    }

    let ticks = IN_RING3.at(cpu);
    // SAFETY: volatile through the raw pointer, into the slot of a core whose
    // timer handler — the only other writer — has nothing to count yet.
    unsafe { ticks.write_volatile(0) };
    // The other three crossings, zeroed beside it and for its reason: a
    // residency's count is about that residency.
    arm_entries(cpu);

    let outcome = OUTCOME.at(cpu);
    // SAFETY: as above; the core is idle and has not been given the job.
    unsafe {
        outcome.write(Outcome { ended: 0, ticks: 0, held: 0, entries: Entries::ZERO, failed: None })
    };

    let job = JOB.at(cpu);
    // SAFETY: as above. Written last of the three, and published to the running
    // core by the `Release` store `smp::run_on` makes after this returns.
    unsafe {
        job.write(Job {
            root: space.root(),
            entry: TEXT,
            stack: STACK_TOP,
            argument: door::Entry::new(provoke.selector(), first).bits(),
            hz,
            target,
        })
    };

    Ok(Prepared {
        space,
        pages: [text, stack, granted, untyped],
        parts: 4,
        before,
        granted: granted_count,
        generation: first.generation(),
        cpu,
    })
}

/// What the frame is asking of one run of a *runtime*.
///
/// Separate from [`Plan`] rather than a variant of it, because the two shapes
/// share almost nothing: a process is entered to be judged by a tally of
/// refusals, and a runtime is entered to be judged by an absence of crossings.
/// A single struct would have half its fields ignored on either path, which is
/// the arrangement that makes a call site passing the wrong one look correct.
#[derive(Clone, Copy)]
pub struct RuntimePlan {
    /// The component's image, out of the component file the loader carried.
    pub image: &'static [u8],
    /// Which of the component's lives the frame is asking for. It reaches the
    /// component in the low half of `f_abi::door::Entry`.
    /// Unit: none — a selector ordinal.
    pub selector: u32,
    /// The physical address of the frame the state tree is published in.
    /// Unit: bytes, physical.
    pub tree: u64,
    /// The rate the core running it arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks that timer asks for. A bound rather than a schedule: the
    /// runtime's load is what ends the run. Unit: timer ticks.
    pub target: u64,
    /// Which core the runtime is allocated. Unit: none — a core index.
    pub cpu: usize,
}

/// The kernel-visible addresses of a runtime's two rings.
///
/// Answered by [`prepare_runtime`] because the frame has to reach both before
/// the runtime does: the notices it already owes go onto the control ring
/// before the first instruction runs, and the work ring is what the frame reads
/// afterwards to see whether the runtime parked cleanly or abandoned its queue.
#[derive(Clone, Copy, Debug)]
pub struct Rings {
    /// The control ring, as the frame sees it. Unit: bytes, kernel-virtual.
    pub control: u64,
    /// The runtime's own work ring, as the frame sees it.
    /// Unit: bytes, kernel-virtual.
    pub work: u64,
}

/// Build a runtime on `cpu`'s behalf: an address space, four pages, a table
/// whose grants are notices it is owed, and two described rings.
///
/// # What is different from [`prepare`], and why it is a second function
///
/// A runtime is given **memory rather than authority**. Its table holds the
/// three capabilities naming what it was mapped, so that the frame owes it three
/// *granted* notices and its first polling point has something real to drain —
/// which is the half `component::demonstrate` could show published and could not
/// show acted on. What it is not given is anything to derive from, map with or
/// spend: a runtime that could enlarge its own address space could fault on a
/// page it made, and the hot-path count would be measuring the wrong thing.
///
/// The two rings are described here rather than by the runtime, because the
/// frame is the grantor: it zeroes the frames, writes the headers, and hands
/// over two addresses. The runtime adopts them and believes nothing —
/// `f_ring::adopt`, RFC 0037 — which is exactly what it would do if the peer
/// were hostile, and is why the same code drives a control ring the frame
/// produces onto and a work ring nobody else touches.
///
/// # Errors
///
/// [`Error`], every variant of which fails the boot.
///
/// # Safety
///
/// As [`prepare`].
pub unsafe fn prepare_runtime(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    plan: RuntimePlan,
) -> Result<(Prepared, Rings), Error> {
    let RuntimePlan { image, selector, tree, hz, target, cpu } = plan;
    if image.is_empty() {
        return Err(Error::NoProgram);
    }
    if image.len() as u64 > FRAME_SIZE {
        return Err(Error::TooLarge);
    }

    let before = frames.free_count();

    // SAFETY: the caller's guarantee that the kernel's space is live and that
    // frames are addressable through its direct map.
    let mut space = unsafe { paging::user_space(frames, kernel) }.map_err(Error::Space)?;

    let text = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    let stack = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    // Zeroed, and it is a real obligation rather than tidiness: `Mapping`'s
    // cursors, index ring and both entry arrays are reinterpreted in place, and
    // all-zero is the one bit pattern every one of those types is valid at.
    let control = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    let work = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;

    let into = frames.virt(text);
    // SAFETY: `text` was just allocated and nothing else holds it; it is one
    // frame, addressable through the direct map, and the image is shorter than
    // one — checked above rather than assumed.
    unsafe { core::ptr::copy_nonoverlapping(image.as_ptr(), into, image.len()) };

    for (virt, frame, kind) in [
        (TEXT, text, paging::UserPage::Text),
        (STACK, stack, paging::UserPage::Data),
        (RING, control, paging::UserPage::Data),
        (WORK, work, paging::UserPage::Data),
    ] {
        // SAFETY: as `user_space`, and `space` is not in `CR3` — it has never
        // been.
        unsafe { paging::map_user(frames, &mut space, virt, frame.addr(), kind, features) }
            .map_err(Error::Space)?;
    }

    let rings = Rings { control: frames.virt(control) as u64, work: frames.virt(work) as u64 };

    let table = crate::cap::of(cpu);
    // SAFETY: the table of a core that is idle, with no process running on it,
    // which is the write `PerCpu::at` exists for.
    let held = unsafe { &mut *table };
    held.clear_all();
    // A runtime has a control ring, so it is owed notices — and the rule that
    // follows is the one that makes the grants below worth making: a slot whose
    // notice field is not quiet is not refilled, so a runtime that never drains
    // runs out of table rather than out of memory. RFC 0008.
    held.owes_notices();
    let first = held
        .grant(CapType::AddressSpace, rights::READ | rights::WRITE, space.root(), 0)
        .map_err(|_| Error::NoSlot)?;
    // Read and write and nothing else, on both rings. Not `DERIVE`, because a
    // runtime that could hand its control ring on would be a runtime whose
    // supervisor no longer knows who is listening; not `REVOKE`, because a
    // component that could revoke its own control ring could make itself
    // unreachable and still be running.
    for object in [control.addr(), work.addr()] {
        held.grant(CapType::Frame, rights::READ | rights::WRITE, object, FRAME_SIZE)
            .map_err(|_| Error::NoSlot)?;
    }
    // The frame's published tree, read-only. RFC 0013's *read, never delivered*
    // is the whole of what a runtime can do with it, and it is observation
    // rather than authority — which is the only kind of capability a runtime
    // needs to hold.
    //
    // **Four grants, and the count is load-bearing rather than tidy.**
    // `f_abi::door::Entry::granted(nth)` computes the nth handle as the first
    // handle's index plus `nth`, *at the first handle's generation* — the frame
    // tells a component one handle and the rest follow — and that arithmetic is
    // sound only while every slot the frame filled carries the same generation.
    // Slots advance a generation each time they are cleared and refilled, so
    // *the same generation* holds exactly while every process shape this kernel
    // builds grants the same number. A runtime granted three left slot three a
    // generation behind slots zero to two on a core that then ran an ordinary
    // process, and the fourth handle that process was told it held resolved to
    // nothing. It presented as a component refusing to map the state tree,
    // which is about as far from the cause as a symptom gets.
    //
    // The structural fix is for `Table::clear_all` to raise every slot to the
    // table's generation floor, which makes the arithmetic a property instead
    // of an accident. It is not made here because it is not this task's to
    // make: `kernel/src/arch/x86_64/probe.rs` names `Handle::FIRST_GENERATION`
    // as a literal in `cap=unowned` and `cap=forge` counts refusals by
    // generation, so raising the floor moves what E0-P08's negative suite is
    // asserting. *Reversal:* that change, with those two fixtures moved with
    // it, at which point this paragraph and this fourth grant both go.
    held.grant(CapType::Frame, rights::READ, tree, FRAME_SIZE).map_err(|_| Error::NoSlot)?;
    let granted_count = held.used();

    let state = STATE.at(cpu);
    // SAFETY: the slot of an idle core, so neither the fault path nor the
    // system-call path over there can be holding it.
    unsafe {
        state.write(State {
            announced: false,
            refused: 0,
            death: Death::Running,
            // A runtime never asks. `PROGRESS` is the call RFC 0008 replaces
            // with a blocking wait on a ring, and a runtime that polled the
            // frame for permission to keep working would be crossing the
            // boundary once per quantum — which is the measurement, inverted.
            wanted: 0,
            giveup: 0,
            caps: Tally::ZERO,
            root: space.root(),
            // Deliberately zero, unlike `prepare`. A runtime holds no capability
            // it could map with, so a capability call arriving from one is a
            // refusal rather than a walk through a borrow — and the borrow on
            // `frames` stays this core's for the whole residency.
            frames: 0,
            features,
        });
    }

    let ticks = IN_RING3.at(cpu);
    // SAFETY: volatile through the raw pointer, into the slot of a core whose
    // timer handler — the only other writer — has nothing to count yet.
    unsafe { ticks.write_volatile(0) };
    arm_entries(cpu);

    let outcome = OUTCOME.at(cpu);
    // SAFETY: as above; the core is idle and has not been given the job.
    unsafe {
        outcome.write(Outcome { ended: 0, ticks: 0, held: 0, entries: Entries::ZERO, failed: None })
    };

    let job = JOB.at(cpu);
    // SAFETY: as above. Written last of the three, and published to the running
    // core by the `Release` store `smp::run_on` makes after this returns.
    unsafe {
        job.write(Job {
            root: space.root(),
            entry: TEXT,
            stack: STACK_TOP,
            argument: door::Entry::new(selector, first).bits(),
            hz,
            target,
        })
    };

    Ok((
        Prepared {
            space,
            pages: [text, stack, control, work],
            parts: 4,
            before,
            granted: granted_count,
            generation: first.generation(),
            cpu,
        },
        rings,
    ))
}

/// What the frame is asking of one run of a *driver*.
///
/// A third plan beside [`Plan`] and [`RuntimePlan`], for [`RuntimePlan`]'s own
/// reason: the three shapes share almost nothing. A process is judged by a
/// tally of refusals, a runtime by an absence of crossings, and a driver by
/// what happened in its client's memory and in a remapping unit's fault
/// registers. The fields below are the ones a driver has and the other two do
/// not — a device's registers, a device's queue memory, and a ring whose far
/// end somebody else holds.
#[derive(Clone, Copy)]
pub struct DriverPlan {
    /// The component's image, out of the component file the loader carried.
    /// May be longer than one page — up to [`TEXT_PAGES`].
    pub image: &'static [u8],
    /// Which of the component's lives the frame is asking for. It reaches the
    /// component in the low half of `f_abi::door::Entry`.
    /// Unit: none — a selector ordinal.
    pub selector: u32,
    /// The physical address of the frame the state tree is published in.
    /// Unit: bytes, physical.
    pub tree: u64,
    /// The rate the core running it arms its own timer at. Unit: hertz.
    pub hz: u32,
    /// How many ticks that timer asks for. Unit: timer ticks.
    pub target: u64,
    /// Which core the driver is allocated. Unit: none — a core index.
    pub cpu: usize,
    /// The first of the device's register pages, physical.
    ///
    /// **Not a frame this allocator owns**, which is why it is an address and
    /// not a [`Frame`]: it is a base-address register the firmware placed, and
    /// [`reap`] must not hand it back to anybody. A `Frame` here would be a
    /// type inviting exactly that.
    /// Unit: bytes, physical.
    pub registers: u64,
    /// The driver's queue memory, physical.
    ///
    /// The caller's to free, for the reason above inverted: it *is* an
    /// allocation, and it is the caller's rather than this function's because
    /// the same region has to be in the device's IOMMU domain before the
    /// component runs and out of it after — which is the supervisor's act and
    /// not the loader's.
    /// Unit: bytes, physical.
    pub queues: u64,
    /// How many bytes of it. Unit: bytes.
    pub queue_bytes: u64,
    /// The frame holding the ring this driver serves its client on, physical.
    ///
    /// The caller's, because the caller holds the other end of it.
    /// Unit: bytes, physical.
    pub data: u64,
}

/// The kernel-visible addresses of a driver's two frame-owned pages.
///
/// The data ring is not here: the caller allocated it, holds the client end of
/// it, and already knows where it is.
#[derive(Clone, Copy, Debug)]
pub struct DriverPages {
    /// The control ring, as the frame sees it. Unit: bytes, kernel-virtual.
    pub control: u64,
    /// The page that says where everything else is, as the frame sees it.
    /// Unit: bytes, kernel-virtual.
    pub board: u64,
}

/// Build a driver on `cpu`'s behalf: an address space, a text reservation, a
/// stack, two rings, a board, its device's registers and its queue memory.
///
/// # What is different from [`prepare_runtime`], and why it is a third function
///
/// Two things, and both of them are the difference between a component that
/// computes and a component that drives a device.
///
/// **Its text is more than a page.** Every other shape here is one page and the
/// build refuses an image that is not; a driver with its own polling loop is
/// three. [`TEXT_PAGES`] is the reservation and the argument for its size.
///
/// **It is mapped things the frame does not own.** A device's registers are a
/// base-address register the firmware placed, and its queue memory is an
/// allocation the *supervisor* made and put in a remapping domain before this
/// call and takes out after it. So they arrive as addresses rather than as
/// frames, and [`reap`] never sees them: [`Prepared`] holds only what this
/// function allocated, which is the property that makes the free count come
/// back.
///
/// # Errors
///
/// [`Error`], every variant of which fails the boot.
///
/// # Safety
///
/// As [`prepare`], and `plan.registers` must name [`BLK_REGISTER_PAGES`] pages
/// of a device's register space that nothing else is driving, `plan.queues`
/// `plan.queue_bytes` of memory the caller allocated and holds, and `plan.data`
/// one frame the caller allocated and holds the far end of.
pub unsafe fn prepare_driver(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    plan: DriverPlan,
) -> Result<(Prepared, DriverPages), Error> {
    let DriverPlan { image, selector, tree, hz, target, cpu, .. } = plan;
    if image.is_empty() {
        return Err(Error::NoProgram);
    }
    let text_pages = (image.len() as u64).div_ceil(FRAME_SIZE) as usize;
    if text_pages > TEXT_PAGES {
        return Err(Error::TooLarge);
    }
    if plan.queue_bytes == 0 || !plan.queue_bytes.is_multiple_of(FRAME_SIZE) {
        return Err(Error::TooLarge);
    }

    let before = frames.free_count();

    // SAFETY: the caller's guarantee that the kernel's space is live and that
    // frames are addressable through its direct map.
    let mut space = unsafe { paging::user_space(frames, kernel) }.map_err(Error::Space)?;

    // One block and not `text_pages` separate frames, because a flat image is
    // contiguous by definition — the frame copies it in one `memcpy` and the
    // component's own `call`s are relative. The order rounds up, which is the
    // allocator's own arithmetic and is why the slack is visible in the free
    // count rather than hidden in a loop.
    let order = Order::new(
        u8::try_from(text_pages.next_power_of_two().trailing_zeros()).unwrap_or(u8::MAX),
    )
    .ok_or(Error::NoFrames)?;
    let text = frames.alloc_zeroed(order).ok_or(Error::NoFrames)?;
    let stack = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    // Zeroed, and it is an obligation rather than tidiness: a channel's cursors
    // and entry arrays are reinterpreted in place and all-zero is the one bit
    // pattern every one of those types is valid at.
    let control = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;
    let board = frames.alloc_zeroed(Order::FRAME).ok_or(Error::NoFrames)?;

    let into = frames.virt(text);
    // SAFETY: `text` was just allocated at an order covering `text_pages`,
    // nothing else holds it, it is addressable through the direct map, and the
    // image is no longer than the block — `text_pages` is computed from its
    // length and the order rounds up.
    unsafe { core::ptr::copy_nonoverlapping(image.as_ptr(), into, image.len()) };

    for page in 0..text_pages as u64 {
        let at = text.addr().wrapping_add(page * FRAME_SIZE);
        // SAFETY: as `user_space`, and `space` is not in `CR3` — it has never
        // been.
        unsafe {
            paging::map_user(
                frames,
                &mut space,
                TEXT + page * FRAME_SIZE,
                at,
                paging::UserPage::Text,
                features,
            )
        }
        .map_err(Error::Space)?;
    }

    for (virt, at, kind) in [
        (SPAWN_STACK, stack.addr(), paging::UserPage::Data),
        (SPAWN_CONTROL, control.addr(), paging::UserPage::Data),
        (BLK_DATA, plan.data, paging::UserPage::Data),
        (BLK_BOARD, board.addr(), paging::UserPage::Data),
    ] {
        // SAFETY: as above.
        unsafe { paging::map_user(frames, &mut space, virt, at, kind, features) }
            .map_err(Error::Space)?;
    }

    for page in 0..BLK_REGISTER_PAGES as u64 {
        // SAFETY: as above, and the caller's guarantee that `registers` names
        // this many pages of a device's register space.
        unsafe {
            paging::map_user(
                frames,
                &mut space,
                BLK_REGISTERS + page * FRAME_SIZE,
                plan.registers.wrapping_add(page * FRAME_SIZE),
                paging::UserPage::Device,
                features,
            )
        }
        .map_err(Error::Space)?;
    }

    for page in 0..plan.queue_bytes / FRAME_SIZE {
        // SAFETY: as above, and the caller's guarantee that `queues` names
        // `queue_bytes` of memory it holds.
        unsafe {
            paging::map_user(
                frames,
                &mut space,
                BLK_QUEUES + page * FRAME_SIZE,
                plan.queues.wrapping_add(page * FRAME_SIZE),
                paging::UserPage::Data,
                features,
            )
        }
        .map_err(Error::Space)?;
    }

    let pages =
        DriverPages { control: frames.virt(control) as u64, board: frames.virt(board) as u64 };

    let table = crate::cap::of(cpu);
    // SAFETY: the table of a core that is idle, with no process running on it,
    // which is the write `PerCpu::at` exists for.
    let held = unsafe { &mut *table };
    held.clear_all();
    // A driver has a control ring, so it is owed notices — and a slot whose
    // notice field is not quiet is not refilled, so a driver that never drains
    // runs out of table rather than out of memory. RFC 0008.
    held.owes_notices();
    let first = held
        .grant(CapType::AddressSpace, rights::READ | rights::WRITE, space.root(), 0)
        .map_err(|_| Error::NoSlot)?;
    // Four grants, in the same order and at the same count as every other shape
    // this kernel builds. The count is load-bearing and `prepare_runtime` says
    // why at length: `door::Entry::granted(nth)` is arithmetic over one
    // generation, and it is sound only while every shape fills the same number
    // of slots.
    for object in [control.addr(), plan.data] {
        held.grant(CapType::Frame, rights::READ | rights::WRITE, object, FRAME_SIZE)
            .map_err(|_| Error::NoSlot)?;
    }
    held.grant(CapType::Frame, rights::READ, tree, FRAME_SIZE).map_err(|_| Error::NoSlot)?;
    let granted_count = held.used();

    let state = STATE.at(cpu);
    // SAFETY: the slot of an idle core, so neither the fault path nor the
    // system-call path over there can be holding it.
    unsafe {
        state.write(State {
            announced: false,
            refused: 0,
            death: Death::Running,
            // A driver never asks. Its loop ends when the frame tells it to
            // stop, on the ring, which is the answer RFC 0008 replaces
            // `PROGRESS` with.
            wanted: 0,
            giveup: 0,
            caps: Tally::ZERO,
            root: space.root(),
            // Zero, as a runtime's is: a driver holds no capability it could
            // map with, so a capability call arriving from one is refused
            // before it reaches an address space.
            frames: 0,
            features,
        });
    }

    let ticks = IN_RING3.at(cpu);
    // SAFETY: volatile through the raw pointer, into the slot of a core whose
    // timer handler — the only other writer — has nothing to count yet.
    unsafe { ticks.write_volatile(0) };
    arm_entries(cpu);

    let outcome = OUTCOME.at(cpu);
    // SAFETY: as above; the core is idle and has not been given the job.
    unsafe {
        outcome.write(Outcome { ended: 0, ticks: 0, held: 0, entries: Entries::ZERO, failed: None })
    };

    let job = JOB.at(cpu);
    // SAFETY: as above. Written last, and published to the running core by the
    // `Release` store `smp::run_on` makes after this returns.
    unsafe {
        job.write(Job {
            root: space.root(),
            entry: TEXT,
            stack: SPAWN_STACK_TOP,
            argument: door::Entry::new(selector, first).bits(),
            hz,
            target,
        })
    };

    Ok((
        Prepared {
            space,
            // Four, and every one of them allocated here. The register pages,
            // the queue memory and the data ring are the caller's and are
            // deliberately absent: a list that held them would free memory this
            // function never took, which is a corruption rather than a leak.
            pages: [text, stack, control, board],
            parts: 4,
            before,
            granted: granted_count,
            generation: first.generation(),
            cpu,
        },
        pages,
    ))
}

/// Run the process this core was given, and record what happened.
///
/// # Why this core arms its own timer
///
/// Because the frame answers "have you run long enough?" by counting ticks
/// taken *out of ring 3*, and only this core's timer can take one out of this
/// core's ring 3. Before there was a second core the answer came from the same
/// timer the milestone was measuring, and the two questions were one; they are
/// two now, and keeping them one would mean a process on this core waiting for
/// ticks that are interrupting a different one.
///
/// The two timers are independent, which is the point of the exit criterion:
/// core 0's jitter measurement runs to its own schedule while this core holds
/// a process at ring 3, and neither is a term in the other.
///
/// It prints nothing. See `smp::arrive`.
///
/// # Safety
///
/// Call on a core [`prepare`] has been called for, with interrupts disabled,
/// the kernel's address space in `CR3`, and `kernel_root` that address space's
/// top-level table. `ring3::init` must have run on this core.
pub unsafe fn execute(kernel_root: u64) {
    let slot = OUTCOME.mine();
    // SAFETY: this core's slot, with no process running on it.
    let job = unsafe { JOB.mine().read() };

    // SAFETY: this core was brought up and adopted the boot processor's clocks,
    // `idt::init` has installed the timer's vector on it, and interrupts are
    // disabled on entry — `start` enables them and `stop` disables them again.
    let window = match unsafe { apic::start(job.hz, job.target) } {
        Ok(window) => window,
        Err(_) => {
            // SAFETY: this core's slot, no process running.
            unsafe {
                slot.write(Outcome {
                    ended: 0,
                    ticks: 0,
                    held: 0,
                    entries: Entries::ZERO,
                    failed: Some(Error::NoTimer),
                })
            };
            return;
        }
    };

    let state = STATE.mine();
    // SAFETY: this core's slot. Interrupts are enabled, but the only handler
    // that touches this shard is the fault path, which cannot run before a
    // process exists — and one does not yet.
    let mut observed = unsafe { state.read() };
    observed.giveup = window.giveup();
    // SAFETY: as above.
    unsafe { state.write(observed) };

    // SAFETY: `job.root` carries a copy of the kernel's upper half, so the
    // instruction after this one, the stack under it and every kernel mapping
    // this core's timer handler needs are all still there.
    unsafe { paging::switch(job.root) };

    // SAFETY: the address space in `CR3` is the process's, both addresses are
    // mapped in it for ring 3, `ring3::init` ran on this core at bring-up, and
    // interrupts are enabled with the timer armed. No process is running: this
    // core has never entered ring 3, or it forgot the last one below.
    let ended = unsafe { ring3::enter(job.entry, job.stack, job.argument) };

    // SAFETY: the kernel's own space, whose kernel window maps this very
    // instruction — which is what makes the switch survivable in both
    // directions. `kernel_root` is the root this core arrived in.
    unsafe { paging::switch(kernel_root) };
    // SAFETY: on the core that entered, after `enter` returned.
    unsafe { ring3::forget() };

    // SAFETY: on the core `start` was called on, once per `start`. Returns with
    // interrupts disabled, which is what the caller expects.
    let _ = unsafe { apic::stop(&window) };

    // SAFETY: this core's slot; the process is over, so nothing can be writing.
    let observed = unsafe { state.read() };
    // The address of the allocator goes away with the process that was allowed
    // to reach it. A capability call arriving after this — which would be a bug
    // in the frame rather than in a process — is refused rather than answered
    // through a pointer to a borrow that has ended.
    // SAFETY: as above.
    unsafe { state.write(State { frames: 0, ..observed }) };
    // SAFETY: volatile, as `IN_RING3` requires; the handler has nothing left to
    // count.
    let ticks = unsafe { IN_RING3.mine().read_volatile() };

    // SAFETY: this core's table, with the process over.
    let table = unsafe { &mut *crate::cap::mine() };
    let held = table.used();
    // Everything the process was given and everything it derived, forgotten in
    // one step. Generations survive it, so a handle this process held cannot
    // resolve in the next one — which is the boundary the generation exists for
    // and the one it would be most tempting to reset at.
    table.clear_all();

    // SAFETY: on the core that ran it, with the process over, so no writer is
    // left for these shards.
    let entries = unsafe { entries_here(ticks) };
    // SAFETY: this core's slot, with the process over.
    unsafe { slot.write(Outcome { ended, ticks, held, entries, failed: None }) };
}

/// Give a finished process's memory back, and say what it did.
///
/// # Errors
///
/// [`Error`], every variant of which fails the boot. There is nothing to fall
/// back to: a process whose frames do not come back is a leak that is cheaper
/// to find now than after there are thousands of them.
///
/// # Safety
///
/// Call on the core that called [`prepare`], after the core it was prepared for
/// has reported that it is finished — which is what makes reading that core's
/// shards sound, and what makes the `&mut` on `frames` live again.
pub unsafe fn reap(frames: &mut FrameAllocator, prepared: Prepared) -> Result<Report, Error> {
    let cpu = prepared.cpu;

    // SAFETY: the slot of a core that has finished and said so, which is what
    // the caller has guaranteed. Read by value; nothing over there is writing.
    let outcome = unsafe { OUTCOME.at(cpu).read() };
    if let Some(failed) = outcome.failed {
        return Err(failed);
    }
    // SAFETY: as above.
    let observed = unsafe { STATE.at(cpu).read() };

    // The two paths agree, or the frame is lying to itself about one of them.
    let death = match (outcome.ended, observed.death) {
        (KILLED, death @ Death::Killed { .. }) | (EXITED, death @ Death::Exited(_)) => death,
        _ => return Err(Error::NoDeath),
    };

    let mut count = 0;
    // The two granted frames go back with the rest, and the fact that the list
    // is still this short is the evidence that the live mapping path allocated
    // nothing: a process that could enlarge its own address space would leave
    // tables here that this loop has never heard of, and the free count would
    // not come back.
    let pages = prepared.pages.get(..prepared.parts).unwrap_or(&[]);
    for frame in prepared.space.tables().iter().copied().chain(pages.iter().copied()) {
        // SAFETY: every one of these came from this allocator in `prepare`, the
        // address space they described is no longer in `CR3` on any core — the
        // core that ran it switched back before it reported finished — and that
        // switch flushed the non-global entries that reached them.
        unsafe { frames.free(frame) };
        count += 1;
    }

    if frames.free_count() != prepared.before {
        return Err(Error::Leaked);
    }

    Ok(Report {
        cpu,
        root: prepared.space.root(),
        shared_slots: prepared.space.shared_slots(),
        frames: count,
        announced: observed.announced,
        refused: observed.refused,
        ticks: outcome.ticks,
        death,
        granted: prepared.granted,
        generation: prepared.generation,
        caps: observed.caps,
        held: outcome.held,
        entries: outcome.entries,
    })
}

/// Where a system call from a process is answered.
///
/// Seven calls and a refusal. Three are M3's and RFC 0014 is the argument for
/// them; four are M4's and RFC 0015 is the argument for those — the short
/// version being that a ring is named by a `Channel` capability, so the
/// capability table has to work before there is any ring to work it through.
/// Every one of the seven names the opcode that retires it. Adding an eighth
/// means arguing against both documents in writing, which is the intended cost.
pub fn syscall(number: u64, first: u64, second: u64) -> Answer {
    let slot = STATE.mine();
    // SAFETY: this core's slot. A system call runs with interrupts masked by
    // `IA32_FMASK`, so the timer handler — the only other code that can reach
    // this core's process state — cannot interleave with it, and a process
    // cannot make two calls at once.
    let mut state = unsafe { slot.read() };

    // Counted before the call is dispatched and before it is even known to be
    // one this build implements, because what is being counted is the crossing
    // rather than the work: a refused call cost exactly as much boundary as an
    // accepted one. `EXIT` is separated here and nowhere else, because it is the
    // one call that *is* the allocation boundary rather than a crossing inside
    // it — RFC 0038.
    count(if number == SYS_EXIT { &BOUNDARY_CALLS } else { &HOT_CALLS });

    let answer = match number {
        SYS_ANNOUNCE => {
            state.announced = true;
            Answer::Reply(0)
        }
        SYS_PROGRESS => Answer::Reply(progress(&state)),
        SYS_EXIT => {
            state.death = Death::Exited(first);
            Answer::Ended(EXITED)
        }
        SYS_CAP_INSPECT | SYS_CAP_DERIVE | SYS_CAP_REVOKE | SYS_CAP_MAP => {
            let reply = capability(number, first, second, &state);
            // Counted here rather than at each of the four, so that a call the
            // suite forgot to count is impossible rather than unlikely.
            state.caps.record(reply);
            Answer::Reply(reply)
        }
        _ => {
            // Refused in the project's own error space rather than with a
            // number invented here. RFC 0010: an error names a domain, and a
            // call that does not exist is an argument error rather than a
            // failure of authority — the process was allowed to ask.
            state.refused += 1;
            let packed = error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE);
            Answer::Reply(packed as i64 as u64)
        }
    };

    // SAFETY: as the read above, and no reference to the slot is live across it.
    unsafe { slot.write(state) };
    answer
}

/// Answer one of the four capability calls, as a word for `rax`.
///
/// A packed error is sign-extended into the register, which is what makes a
/// refusal distinguishable from an answer without a second output: every
/// success here is a small non-negative number and every failure is negative.
/// The same convention the ring will use in `Cqe::result`, one milestone early,
/// and deliberately so — a process written against this one does not have to
/// learn a second one at M5.
fn capability(number: u64, first: u64, second: u64, state: &State) -> u64 {
    let result = match number {
        SYS_CAP_INSPECT => inspect(Handle::from_bits(first as u32)),
        SYS_CAP_DERIVE => derive(Handle::from_bits(first as u32), second, state),
        SYS_CAP_REVOKE => revoke(Handle::from_bits(first as u32), state),
        SYS_CAP_MAP => map_frame(first, second, state),
        // Unreachable from `syscall`, which matches on exactly these four. Not
        // a panic: a frame that cannot be provoked into faulting by ring 3 is
        // the property under test, and it would be an odd place to keep an
        // exception to it.
        _ => Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE)),
    };
    match result {
        Ok(value) => value,
        Err(packed) => packed as i64 as u64,
    }
}

/// This core's capability table, as a shared reference.
///
/// # Safety
///
/// Call only from the system-call path, which runs with interrupts masked by
/// `IA32_FMASK` — so the timer handler, the only other code on this core that
/// could reach the same slot, cannot interleave with the reference's life.
unsafe fn table() -> &'static Table {
    // SAFETY: the caller's guarantee. The pointer is this core's slot, which is
    // a `.bss` static and does not move.
    unsafe { &*crate::cap::mine() }
}

/// This core's capability table, mutably.
///
/// # Safety
///
/// As [`table`], and no other reference to the table may be live.
unsafe fn table_mut() -> &'static mut Table {
    // SAFETY: the caller's guarantee.
    unsafe { &mut *crate::cap::mine() }
}

/// What a handle names: the kind in the high byte, the rights in the low one.
///
/// Packed into one word because the door returns one, and it is the shape that
/// goes away at M5 — a completion has room for the object and the extent too,
/// and a process that wants those today does not get them. Deliberate: a
/// process at M4 has no use for a physical address it cannot map without
/// presenting the capability anyway, and the narrower answer is the one that
/// does not have to be taken back later.
fn inspect(handle: Handle) -> Result<u64, i32> {
    // SAFETY: the system-call path, per `table`.
    let found = unsafe { table() }.inspect(handle)?;
    Ok((u64::from(found.kind.to_wire()) << 8) | u64::from(found.rights))
}

/// Mint a weaker capability, or a copy, and answer with its handle.
///
/// # Why this is the call that can spend money
///
/// Because it is the only one a component makes that needs a slot it has not
/// already got. RFC 0008 makes the capability table an object retyped from the
/// component's own `Untyped`, and E1-B13 puts the purchase exactly where the
/// need appears: a derive with nowhere to put its child buys a page and tries
/// again, and a component with nothing left to spend is refused
/// `RESOURCE/QUOTA_EXHAUSTED` rather than served from anything the frame keeps
/// back. `cap=flood` is the run that buys and `cap=quota` the run that cannot.
fn derive(handle: Handle, asked: u64, state: &State) -> Result<u64, i32> {
    let asked = u8::try_from(asked)
        .map_err(|_| error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG))?;
    if state.frames == 0 {
        // No process is running, so there is no borrow of the allocator to
        // reach a bought page through. Unreachable from ring 3 — a call arrives
        // only while a process is entered — and refused rather than asserted,
        // for the reason `map_frame` gives at the same check.
        return Err(error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED));
    }
    // SAFETY: `state.frames` is the address of the `&mut FrameAllocator` that
    // `run` holds, dormant for the whole life of the process — the same
    // argument `map_frame` makes and for the same duration. Shared, never
    // mutable: growing a table allocates nothing, it only translates a frame
    // the process has already paid for.
    let frames = unsafe { &*(state.frames as *const FrameAllocator) };
    // SAFETY: every `Untyped` in this table names a frame `prepare` allocated
    // out of this allocator for this process and gave to nobody else, and the
    // watermark means no frame inside one is ever handed out twice — so the
    // frame a growth charges for is owned by this table alone. `frames` is
    // rebound onto the direct map of the address space in `CR3`.
    let mut ground = unsafe { Direct::new(frames) };
    // SAFETY: as `inspect`, and no other reference to the table is live: this
    // is the only one taken in this call.
    let minted = unsafe { table_mut() }.derive(handle, asked, &mut ground)?;
    Ok(u64::from(minted.bits()))
}

/// Withdraw everything derived from a capability, and answer with how many.
///
/// # What this does that it did not do before E0-B10
///
/// It takes the memory back as well as the name.
///
/// Until there was a second core, revoking a frame capability that had been
/// mapped withdrew the capability and left the mapping standing. It was the
/// largest gap in the capability system and it was stated in four places rather
/// than buried, because it is the sentence somebody would otherwise assume the
/// other way round: a component whose authority had been revoked went on
/// reading the page through a translation nobody could take away.
///
/// Undoing a mapping needs an unmap, an unmap needs a shootdown, and a
/// shootdown needs somebody to shoot down *to*. That is the whole of why this
/// waited: not that one core made it hard, but that one core made it
/// unfalsifiable — a kernel that skipped the interrupt would have passed every
/// test it could have been given.
/// # Why it is three steps here and one call before E1-B13
///
/// Because a table is as many slots as its holder has paid for, so the list of
/// mappings a revocation withdraws is no longer bounded by anything that fits
/// in a return value. The table condemns, this function drains the addresses
/// one at a time and unmaps each, and the table sweeps last — which is also the
/// order that fails safely: authority is withdrawn only after every translation
/// behind it is gone, and a drain that could not finish leaves a table that
/// still names what it still maps.
fn revoke(handle: Handle, state: &State) -> Result<u64, i32> {
    // SAFETY: as `derive`.
    let table = unsafe { table_mut() };
    let mut condemned = table.condemn(handle)?;
    while let Some(page) = table.next_mapping(&mut condemned) {
        withdraw(state, page)?;
    }
    Ok(u64::from(table.sweep(&condemned)))
}

/// Take one mapping out of the running process's address space and tell every
/// other core.
///
/// # Why a failure here ends the machine
///
/// Because there is nothing smaller to do about it. A shootdown that is not
/// acknowledged means some core may still hold a translation to a page whose
/// authority has been withdrawn, and the frame has no way to find out whether
/// it does. Returning an error to the process would be answering "the authority
/// is gone" when it is not, which is the one lie a capability system cannot
/// tell. So it says what happened and stops.
///
/// The unmap failing is different in cause and the same in consequence: the
/// tables and the table of capabilities disagree about what is mapped, which is
/// a bug in this file rather than in the process, and continuing would mean
/// building on it.
fn withdraw(state: &State, page: u64) -> Result<(), i32> {
    if state.frames == 0 || state.root == 0 {
        // No process is running, so there is no address space to edit. Reaching
        // here is a frame bug rather than a process one — a capability call
        // cannot arrive without a process — and refusing is the smallest
        // truthful answer.
        return Err(error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS));
    }

    // SAFETY: the address is one this core wrote into `State` while it held the
    // caller's `&mut FrameAllocator`, and that borrow is dormant until the
    // process ends — the same argument `map_frame` makes, and for the same
    // duration. Shared, never mutable: an unmap frees nothing.
    let frames = unsafe { &*(state.frames as *const FrameAllocator) };

    // SAFETY: `state.root` is the top-level table this kernel built for the
    // running process, `frames` is rebound onto the direct map of the space in
    // `CR3`, and that space is the one `root` describes.
    let result = unsafe { paging::unmap_user_live(frames, state.root, page) };
    if let Err(why) = result {
        kprintln!();
        kprintln!("FAIL: a revoked capability's mapping could not be withdrawn: {}", why.message());
        crate::arch::x86_64::exit_qemu(crate::arch::x86_64::Exit::Failure);
    }

    // SAFETY: the entry has been cleared and this core's own translation
    // invalidated by the call above, and every other running core has
    // interrupts enabled — the boot processor holds a timer window open across
    // the whole of a process's life, and a started core enables them before it
    // reports ready.
    if let Err(cpu) = unsafe { crate::smp::shootdown(page) } {
        kprintln!();
        kprintln!("FAIL: core {cpu} did not acknowledge that a revoked page was unmapped");
        crate::arch::x86_64::exit_qemu(crate::arch::x86_64::Exit::Failure);
    }

    Ok(())
}

/// Map a frame into an address space, on the two capabilities that name them.
///
/// `first` carries the frame handle in its low half and the address space
/// handle in its high half; `second` is a page-aligned address with the
/// requested rights in the twelve bits alignment leaves free. See
/// [`SYS_CAP_MAP`] on why they are packed and what unpacks them at M5.
///
/// # The order the checks are in, which is the whole of what this is testing
///
/// Authority first, then the argument, then the page tables. A frame that
/// checked the address before the capability would refuse an overlapping
/// mapping with an argument error whether or not the process was entitled to
/// make it — and the negative suite would pass while proving nothing, because
/// every attempt would be refused for a reason that has nothing to do with
/// authority.
fn map_frame(first: u64, second: u64, state: &State) -> Result<u64, i32> {
    let frame = Handle::from_bits(first as u32);
    let space = Handle::from_bits((first >> 32) as u32);
    let virt = second & !(FRAME_SIZE - 1);
    let low = second & (FRAME_SIZE - 1);

    let asked = u8::try_from(low)
        .map_err(|_| error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG))?;
    // Only the three that describe a mapping. A rights bit that is meaningful
    // on a capability and meaningless on a mapping — `DERIVE`, say — is refused
    // rather than ignored, because ignoring it is how a caller comes to believe
    // it asked for something it did not.
    if asked & !(rights::READ | rights::WRITE | rights::EXECUTE) != 0 {
        return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
    }

    // SAFETY: the system-call path, per `table`. Both lookups are through the
    // same shared reference, which is why they are taken together.
    let held = unsafe { table() };
    let object = held.invoke(frame, CapType::Frame, asked)?;
    let target = held.invoke(space, CapType::AddressSpace, rights::WRITE)?;

    let kind = match (asked & rights::WRITE != 0, asked & rights::EXECUTE != 0) {
        // Write-exclusive-or-execute, enforced where the authority is checked
        // rather than only where the page tables are built. A capability may
        // legitimately carry both rights — the frame it names can be used for
        // either — and it is the *mapping* that may not have both at once.
        (true, true) => {
            return Err(error::pack(error::ARGUMENT, error::argument::RIGHTS_CONFLICT));
        }
        (true, false) => paging::UserPage::Data,
        (false, true) => paging::UserPage::Text,
        (false, false) => paging::UserPage::ReadOnly,
    };
    if asked & rights::READ == 0 {
        // A mapping nothing may read is not something this file can express,
        // and pretending otherwise would produce a page that is present and
        // unreadable — which is not what the caller asked for.
        return Err(error::pack(error::ARGUMENT, error::argument::RIGHTS_CONFLICT));
    }

    if state.frames == 0 {
        // No process is running, so there is no borrow of the allocator to
        // reach through. Unreachable from ring 3 — a call arrives only while a
        // process is entered — and refused rather than asserted, because the
        // one thing this path may never do is fault.
        return Err(error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED));
    }
    // SAFETY: `state.frames` is the address of the `&mut FrameAllocator` that
    // `run` holds. `run` is blocked inside `ring3::enter` for the whole life of
    // the process and does not touch that borrow until it returns, so a shared
    // reborrow of it is live only while the borrow it came from is dormant.
    // `run` zeroes this field before it uses the allocator again, and the check
    // above is what makes the zero mean "not now".
    let frames = unsafe { &*(state.frames as *const FrameAllocator) };

    // SAFETY: `target.object` is a top-level table this kernel built for a
    // process and put in the table itself — a process cannot place one — and
    // `frames` is rebound onto the direct map of the space in `CR3`, whose
    // upper half is a copy of the kernel's. `object.object` is a frame the
    // frame allocator gave this process and nothing else holds.
    unsafe {
        paging::map_user_live(frames, target.object, virt, object.object, kind, state.features)
    }
    .map_err(|_| error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS))?;

    // After the mapping exists and not before. A slot that recorded an address
    // the tables do not have would make the next revoke unmap a page this
    // capability never authorised — and the reverse order is the one that looks
    // tidier, which is why it is worth a sentence.
    //
    // The `&mut` is taken here rather than at the top of this function because
    // the shared reference above is still live until the mapping is made: two
    // references to one table, one of them mutable, is the aliasing this file
    // avoids by sequencing rather than by hoping.
    // SAFETY: the system-call path, per `table`, and the shared reference taken
    // above is dead — its last use was the mapping.
    unsafe { table_mut() }.note_mapping(frame, virt)?;
    Ok(0)
}

/// Whether the process has run long enough, and how the frame knows.
fn progress(state: &State) -> u64 {
    let ticks = IN_RING3.mine();
    // SAFETY: volatile through the raw pointer, because the timer handler
    // writes the same location. See [`IN_RING3`].
    let taken = unsafe { ticks.read_volatile() };
    if taken >= state.wanted {
        return ENOUGH;
    }
    if read_tsc() > state.giveup {
        return GAVE_UP;
    }
    KEEP_GOING
}

/// Count a timer tick that was taken out of ring 3.
///
/// # Safety
///
/// Call from the timer's handler, on the core the timer was armed on, having
/// established from the saved code selector that the interrupted code was at
/// ring 3.
pub unsafe fn tick_from_ring3() {
    let ticks = IN_RING3.mine();
    // SAFETY: volatile through the raw pointer, for the reason [`IN_RING3`]
    // gives. The handler cannot interrupt itself — its gate is an interrupt
    // gate — so this read-modify-write is not racing another copy of itself.
    let taken = unsafe { ticks.read_volatile() };
    // SAFETY: as above.
    unsafe { ticks.write_volatile(taken + 1) };

    // The frame reaching a core it has given away, and the only way it can.
    // An interrupt happened here and a preemption did not: nothing below
    // redirects the interrupted instruction stream or ends anything, it writes
    // a completion entry into a ring the runtime will read when it next chooses
    // to look. `kernel/src/runtime.rs` argues why that distinction is the whole
    // model rather than a detail of it.
    // SAFETY: the timer handler on the core ring 3 is holding, with the tick
    // count it has just taken, which is exactly what this asks for.
    unsafe { crate::runtime::on_ring3_tick(taken + 1) };
}

/// Count an interrupt other than the timer that was taken out of ring 3.
///
/// A shootdown, a doorbell or the spurious vector. None of them is on the hot
/// path — every one is the frame or another core reaching a core this one gave
/// away, which is [`Entries::ticks`]'s argument and not a weaker one — and all
/// of them are kernel entries, so a bucket is what they get. Counting is the
/// whole of it: nothing here decides anything, and the handler that called it
/// goes on to do whatever the vector is for.
///
/// # Safety
///
/// Call from the interrupt dispatcher on the core the interrupt was delivered
/// to, having established from the saved code selector that the interrupted
/// code was at ring 3.
pub unsafe fn frame_interrupt_from_ring3() {
    count(&FRAME_INTERRUPTS);
}

/// End the process because of a fault it took.
///
/// Returns false when there was no process to end, which leaves the caller to
/// treat the fault as the kernel's own — the only honest thing to do with an
/// exception that claims to come from ring 3 on a core that never went there.
///
/// # Safety
///
/// Call from the interrupt dispatcher with the frame it is about to restore
/// from, having established that the fault was taken at ring 3.
#[must_use]
pub unsafe fn kill(frame: &mut crate::arch::x86_64::idt::Frame, address: u64) -> bool {
    // A fault is a crossing the code at ring 3 did not choose, and it is on the
    // hot path for exactly that reason: the claim is that a runtime's work never
    // reaches the frame, and a page fault is that claim failing in the way that
    // is hardest to notice from above. Counted before anything else, so that a
    // kill which then goes wrong still leaves the count behind.
    count(&RING3_FAULTS);

    let slot = STATE.mine();
    // SAFETY: this core's slot. The gate is an interrupt gate, so this handler
    // cannot interrupt itself, and the system-call path it could otherwise
    // interleave with runs with interrupts masked.
    let mut state = unsafe { slot.read() };
    state.death =
        Death::Killed { vector: frame.vector, error: frame.error, address, rip: frame.rip };
    // SAFETY: as above.
    unsafe { slot.write(state) };

    // SAFETY: this is the frame an interrupt stub is about to restore from, and
    // the caller has established the fault was taken at ring 3.
    unsafe { ring3::resume(frame, KILLED) }
}

/// Check the arithmetic and the selector layout before a process depends on
/// either.
///
/// Two things, and the second is the one that cannot be debugged afterwards.
/// The layout: text, guard and stack are three consecutive pages in the lower
/// half, inside one two-mebibyte region, which is what makes a process's
/// address space four tables rather than six. And the selectors: `sysret`
/// computes both of the ones it loads by adding fixed offsets to a field of
/// `IA32_STAR`, so a table laid out any other way returns to ring 3 through
/// whatever descriptor happened to be there — which is not a fault, it is a
/// process running with the wrong segment, or the kernel's.
///
/// # Errors
///
/// A sentence naming what does not hold.
pub fn self_test() -> Result<(), &'static str> {
    use crate::arch::x86_64::gdt;

    if !TEXT.is_multiple_of(FRAME_SIZE) || GUARD != TEXT + FRAME_SIZE || STACK != GUARD + FRAME_SIZE
    {
        return Err("the process layout is not three consecutive pages");
    }
    if STACK_TOP != STACK + FRAME_SIZE {
        return Err("the process's stack pointer is not one past its stack");
    }
    if STACK_TOP >= 1 << 47 {
        return Err("the process layout is not in the lower half");
    }
    // One page table covers two mebibytes. Text and stack sharing one is the
    // reason `MAX_USER_TABLES` is four rather than six.
    if TEXT >> 21 != STACK >> 21 {
        return Err("the process's text and stack need two page tables, not one");
    }

    // What `syscall` loads: the field, and the segment eight bytes after it.
    let kernel_cs = ((gdt::STAR >> 32) & 0xFFFF) as u16;
    if kernel_cs != gdt::KERNEL_CODE || kernel_cs + 8 != gdt::KERNEL_DATA {
        return Err("IA32_STAR does not name the kernel's code and stack segments");
    }

    // What `sysret` loads: the other field, plus sixteen for code and eight for
    // the stack, each with the requested privilege level forced to three.
    let user_base = ((gdt::STAR >> 48) & 0xFFFF) as u16;
    if (user_base + 16) | 3 != gdt::USER_CODE || (user_base + 8) | 3 != gdt::USER_DATA {
        return Err("IA32_STAR does not name the ring-3 segments sysret would load");
    }

    if probe::program().len() as u64 > FRAME_SIZE {
        return Err("the process's program does not fit in one page");
    }

    Ok(())
}
