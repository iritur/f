// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A process: an address space, two pages, and a way of ending.
//!
//! # What a process is at M3
//!
//! Less than the word usually means, and the gap is the honest part. There is
//! no scheduler, no capability table, no ring, and no second process. What
//! exists is the thing all four of those need first: a body of code running at
//! privilege level three, in an address space of its own, that the frame can
//! start and can end — including when the process would rather it did not.
//!
//! One at a time, on the core that starts it. `run` takes the core for the
//! process's whole life and returns when it is over. That is not a design; it
//! is what a system with no scheduler can honestly say, and E0-B10 is where a
//! second core makes the question real.
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

use f_abi::error;

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::{paging, probe, read_tsc, ring3};
use crate::mem::{FRAME_SIZE, FrameAllocator, Order};
use crate::percpu::PerCpu;

/// Where a process's text is mapped.
///
/// Four mebibytes up, which is a long way from the null page and inside the
/// first two-mebibyte region a page table covers — so text, guard and stack are
/// one table between them. The address is fixed because there is nothing yet to
/// choose it with: E0-B10 loads a real component and E0-B11 gives it an address
/// space capability, and the layout stops being a constant then.
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

/// "I am here." Takes nothing, and the frame records that it happened.
///
/// It is the whole of what a process can say before there is a channel to say
/// it on. At M5 this is what channel setup replaces: the same handshake,
/// carrying a ring rather than carrying nothing. RFC 0014.
const SYS_ANNOUNCE: u64 = 0;

/// "Have I run long enough?" Answers [`KEEP_GOING`], [`ENOUGH`] or
/// [`GAVE_UP`].
///
/// Replaced at M5 by a blocking wait on a ring, which is the same question
/// asked of something that can answer it without being polled.
const SYS_PROGRESS: u64 = 1;

/// "I am done." The first argument is a status.
///
/// The one of the three with no successor named, because it is the one a
/// process genuinely cannot do through a ring: submitting "I no longer exist"
/// and then waiting for the completion is not a sequence a process can finish.
const SYS_EXIT: u64 = 2;

/// The answer to [`SYS_PROGRESS`] while the process should carry on.
const KEEP_GOING: u64 = 0;

/// The answer once the frame has taken as many ticks from ring 3 as it wanted.
const ENOUGH: u64 = 1;

/// The answer when the frame has given up waiting for those ticks.
///
/// Bounded in time rather than in calls, and read from the one clock the kernel
/// is allowed to read directly. A process that polls forever because the timer
/// stopped is a machine that hangs in boot with no output, which is the failure
/// `apic::wait` already refuses to have and this refuses for the same reason.
const GAVE_UP: u64 = 2;

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
}

/// Per core, because a process runs on one and its faults arrive on that one.
static STATE: PerCpu<State> = PerCpu::new(State {
    announced: false,
    refused: 0,
    death: Death::Running,
    wanted: 0,
    giveup: 0,
});

/// Timer ticks taken while ring 3 held the core.
///
/// The one piece of process state two paths touch: the interrupt handler writes
/// it and a system call reads it. It lives outside [`State`] and every access is
/// volatile through the raw pointer, for exactly the reason `apic::TICKS` does —
/// a reference here would be a claim that the handler and the code it
/// interrupted are not both looking at it, and they are.
static IN_RING3: PerCpu<u64> = PerCpu::new(0);

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
}

/// What a provocation is supposed to produce.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Expect {
    /// A fault at this vector, and the process killed.
    Fault(u64),
    /// A clean exit, with this many calls refused on the way.
    Exit(u32),
}

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
        ] {
            if boot.has_parameter(parameter) {
                return provoke;
            }
        }
        Self::Kernel
    }

    /// The word the process is handed on entry.
    #[must_use]
    pub const fn selector(self) -> u64 {
        match self {
            Self::Kernel => probe::PROVOKE_KERNEL,
            Self::Null => probe::PROVOKE_NULL,
            Self::Text => probe::PROVOKE_TEXT,
            Self::Stack => probe::PROVOKE_STACK,
            Self::Privileged => probe::PROVOKE_PRIV,
            Self::Call => probe::PROVOKE_CALL,
            Self::Exit => probe::PROVOKE_EXIT,
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
        }
    }

    /// What has to happen for the provocation to have been provoked.
    #[must_use]
    pub const fn expects(self) -> Expect {
        match self {
            Self::Kernel | Self::Null | Self::Text | Self::Stack => Expect::Fault(PAGE_FAULT),
            Self::Privileged => Expect::Fault(GENERAL_PROTECTION),
            Self::Call => Expect::Exit(1),
            Self::Exit => Expect::Exit(0),
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
    /// The program does not fit in one page. It is assembled into the kernel
    /// image, so this is a build-time fact discovered at boot — and a program
    /// that outgrows a page needs a second mapping rather than a larger
    /// constant.
    TooLarge,
    /// The process ended and nothing recorded how, which is a bug in the frame
    /// rather than in the process.
    NoDeath,
    /// The free count did not come back. Something the process owned was not
    /// given back, and continuing would hide it.
    Leaked,
}

impl Error {
    /// A sentence for the serial log.
    #[must_use]
    pub fn message(self) -> &'static str {
        match self {
            Self::Space(inner) => inner.message(),
            Self::NoFrames => "no frame for the process's text or stack",
            Self::TooLarge => "the process's program does not fit in one page",
            Self::NoDeath => "the process ended and the frame did not record how",
            Self::Leaked => "a process's frames were not all given back",
        }
    }
}

/// What one run of a process produced.
#[derive(Clone, Copy)]
pub struct Report {
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

/// Build a process, run it until it ends, and give its memory back.
///
/// `wanted` is how many timer ticks the frame takes out of ring 3 before it
/// tells the process it has run long enough, and `giveup` is a timestamp-counter
/// value past which it stops waiting for them.
///
/// # Errors
///
/// [`Error`], every variant of which fails the boot. There is nothing to fall
/// back to: a process that cannot be built is a milestone that has not been
/// reached, and one whose frames do not come back is a leak that is cheaper to
/// find now than after there are thousands of them.
///
/// # Safety
///
/// Call on the boot processor, with the kernel's address space in `CR3`,
/// `frames` rebound onto its direct map, [`ring3::init`] done on this core, and
/// interrupts enabled with the timer armed — the whole point is that it keeps
/// ticking while this runs.
pub unsafe fn run(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: paging::Features,
    provoke: Provoke,
    wanted: u64,
    giveup: u64,
) -> Result<Report, Error> {
    let program = probe::program();
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

    let state = STATE.mine();
    // SAFETY: this core's slot, with no process running, so neither the fault
    // path nor the system-call path can be holding it.
    unsafe {
        state.write(State { announced: false, refused: 0, death: Death::Running, wanted, giveup });
    }
    let ticks = IN_RING3.mine();
    // SAFETY: volatile through the raw pointer, before the handler that is the
    // only other writer can have anything to count.
    unsafe { ticks.write_volatile(0) };

    // SAFETY: `space` carries a copy of the kernel's upper half, so the
    // instruction after this one, the stack under it and every kernel mapping
    // the timer's handler needs are all still there.
    unsafe { paging::switch(space.root()) };

    // SAFETY: the address space in `CR3` is the process's, both addresses are
    // mapped in it for ring 3, `ring3::init` ran on this core at boot, and the
    // caller has guaranteed interrupts are enabled. No process is running: the
    // previous one, if any, was forgotten below.
    let outcome = unsafe { ring3::enter(TEXT, STACK_TOP, provoke.selector()) };

    // SAFETY: the kernel's own space, whose kernel window maps this very
    // instruction — which is what makes the switch survivable in both
    // directions.
    unsafe { paging::activate(kernel) };
    // SAFETY: on the core that entered, after `enter` returned.
    unsafe { ring3::forget() };

    // SAFETY: this core's slot; the process is over, so nothing can be writing.
    let observed = unsafe { state.read() };
    // SAFETY: volatile, as above; the handler has nothing left to count.
    let in_ring3 = unsafe { ticks.read_volatile() };

    // The two paths agree, or the frame is lying to itself about one of them.
    let death = match (outcome, observed.death) {
        (KILLED, death @ Death::Killed { .. }) | (EXITED, death @ Death::Exited(_)) => death,
        _ => return Err(Error::NoDeath),
    };

    let mut count = 0;
    for frame in space.tables().iter().copied().chain([text, stack]) {
        // SAFETY: every one of these came from this allocator a few lines
        // above, the address space they described is no longer in `CR3`, and
        // the switch that took it out flushed the non-global entries that
        // reached them. Nothing refers to any of them.
        unsafe { frames.free(frame) };
        count += 1;
    }

    if frames.free_count() != before {
        return Err(Error::Leaked);
    }

    Ok(Report {
        root: space.root(),
        shared_slots: space.shared_slots(),
        frames: count,
        announced: observed.announced,
        refused: observed.refused,
        ticks: in_ring3,
        death,
    })
}

/// Where a system call from a process is answered.
///
/// Three calls and a refusal, which is the whole interface a process has at M3.
/// It is deliberately not growing, and RFC 0014 is the argument: the entry is a
/// door rather than an interface, a call may exist only if it cannot be an
/// opcode on a ring, and each of these three names the thing that replaces it.
/// Adding a fourth means arguing against that in writing, which is the intended
/// cost.
pub fn syscall(number: u64, first: u64, _second: u64) -> Answer {
    let slot = STATE.mine();
    // SAFETY: this core's slot. A system call runs with interrupts masked by
    // `IA32_FMASK`, so the timer handler — the only other code that can reach
    // this core's process state — cannot interleave with it, and a process
    // cannot make two calls at once.
    let mut state = unsafe { slot.read() };

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
