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
//! What it does not do yet is take the memory back with the name. Revoking a
//! frame capability withdraws the capability and leaves the mapping standing;
//! `arch::x86_64::paging` says why, and the second core is what would fix it.
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

use f_abi::cap::{CapType, Handle, rights};
use f_abi::error;

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::{paging, probe, read_tsc, ring3};
use crate::cap::{TABLE_SLOTS, Table};
use crate::mem::{FRAME_SIZE, FrameAllocator, Order};
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

/// "What is this handle?" Answers a packed kind and rights, or an authority
/// error.
///
/// The first of four capability calls, and RFC 0015 is the argument for why
/// they are calls at all when RFC 0014 says the door does not accumulate an
/// interface. In short: a ring is named by a `Channel` capability, so the table
/// has to work before there is any ring to work it through, and each of the
/// four names the opcode that retires it. This one becomes an opcode on the
/// component's control ring at M5.
pub(crate) const SYS_CAP_INSPECT: u64 = 3;

/// "Mint me a weaker one." Takes a handle and a rights bitmap, answers a
/// handle.
///
/// Copy is the identity case — the same rights — and is a derivation like any
/// other, so that revoking the source reaches it. `kernel/src/cap.rs` argues
/// that against seL4. Retired by the same control-ring opcode at M5.
pub(crate) const SYS_CAP_DERIVE: u64 = 4;

/// "Take back everything I handed on from this." Answers how many capabilities
/// were withdrawn. Retired at M5 with the rest.
pub(crate) const SYS_CAP_REVOKE: u64 = 5;

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
pub(crate) const SYS_CAP_MAP: u64 = 6;

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
/// Its address space, one frame, and one untyped region — in that order, so
/// their handles are the first three slots and the process can be written
/// against them. Three, and every one of them is something the process needs
/// and nothing more: there is no capability here for the frame it is running
/// out of, because a process that could remap its own text is a process for
/// which write-exclusive-or-execute is advisory.
pub const GRANTS: usize = 3;

/// Capability calls every process makes before it does whatever it was told to.
///
/// Inspect the frame capability, derive a copy of it, map the copy. Three, and
/// they are the positive path: a process that cannot use a capability correctly
/// cannot meaningfully fail to abuse one, and a suite of nothing but refusals
/// passes on a frame that refuses everything.
const PREAMBLE_OK: u32 = 3;

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

/// How many capabilities a flooding process mints before the table is full.
///
/// Every slot that is not already spoken for. That this is a number at all is
/// the point: the table has a bound, the bound is reached with an error rather
/// than a fault, and the error names a resource.
const FLOOD_MINTS: u32 = TABLE_SLOTS as u32 - SWEEP_LIVE;

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
            Self::Grant => probe::PROVOKE_CAP_GRANT,
            Self::Unowned => probe::PROVOKE_CAP_UNOWNED,
            Self::Forge => probe::PROVOKE_CAP_FORGE,
            Self::Stale => probe::PROVOKE_CAP_STALE,
            Self::Rights => probe::PROVOKE_CAP_RIGHTS,
            Self::Mistyped => probe::PROVOKE_CAP_TYPE,
            Self::Flood => probe::PROVOKE_CAP_FLOOD,
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
        }
    }

    /// What has to happen for the provocation to have been provoked.
    #[must_use]
    pub const fn expects(self) -> Expect {
        match self {
            Self::Kernel | Self::Null | Self::Text | Self::Stack => Expect::Fault(PAGE_FAULT),
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
            | Self::Flood => Expect::Exit(0),
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
    #[must_use]
    pub const fn expects_caps(self) -> Tally {
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
            // Four handles are live at that point — the three grants and the
            // one the preamble derived — so exactly four of the hundred and
            // thirty-two resolve.
            Self::Forge => tally(base + SWEEP_LIVE, SWEEP_REFUSED, 0, 0, 0, 0),

            // Derive a grandchild, revoke the root, then use both leaves.
            Self::Stale => tally(base + 2, 0, 0, 2, 0, 0),

            // Widen by derivation, then map more permissively than the
            // capability allows.
            Self::Rights => tally(base, 0, 2, 0, 0, 0),

            // A space where a frame belongs, then a frame where a space does.
            Self::Mistyped => tally(base, 0, 0, 0, 2, 0),

            // Every free slot, then the refusal.
            Self::Flood => tally(base + FLOOD_MINTS, 0, 0, 0, 0, 1),
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
    /// The capability table had no room for the frame's own grants. A bug in
    /// the frame — it grants three into an empty table of thirty-two — and
    /// reported rather than ignored, because a process that starts without one
    /// of its capabilities fails later and somewhere else.
    NoSlot,
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
            Self::NoSlot => "the capability table had no room for a process's own grants",
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
    /// How many capabilities the frame granted it.
    pub granted: usize,
    /// What its capability calls were answered with.
    pub caps: Tally,
    /// Capabilities still in its table when it ended, before the table was
    /// cleared. Everything it derived and did not have revoked.
    pub held: usize,
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
        let expected = provoke.expects_caps();
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

    // The table, before the process exists to reach it. Three grants, in the
    // order the process is written against, and nothing else will ever be put
    // in from this side: everything the table holds after this line is
    // something the process derived.
    let table = crate::cap::mine();
    // SAFETY: this core's table, with no process running — so neither the
    // system-call path nor the fault path can be holding a reference to it.
    let held = unsafe { &mut *table };
    held.clear_all();
    let space_rights = rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE;
    held.grant(CapType::AddressSpace, space_rights, space.root(), 0).map_err(|_| Error::NoSlot)?;
    // Deliberately without `WRITE`, and it is the whole of the rights half of
    // the negative suite: a process that could map this writable would have
    // exceeded what it was granted, and `cap=rights` is the run that tries.
    let frame_rights = rights::READ | rights::DERIVE | rights::REVOKE;
    held.grant(CapType::Frame, frame_rights, granted.addr(), FRAME_SIZE)
        .map_err(|_| Error::NoSlot)?;
    held.grant(CapType::Untyped, space_rights, untyped.addr(), FRAME_SIZE)
        .map_err(|_| Error::NoSlot)?;
    let granted_count = held.used();

    let state = STATE.mine();
    // SAFETY: this core's slot, with no process running, so neither the fault
    // path nor the system-call path can be holding it.
    unsafe {
        state.write(State {
            announced: false,
            refused: 0,
            death: Death::Running,
            wanted,
            giveup,
            caps: Tally::ZERO,
            // An address derived from the caller's `&mut`, which is not used
            // again until `enter` has returned — so the borrow it came from is
            // dormant for exactly as long as the capability calls may use it.
            // That is the whole of why a process may reach the frame allocator
            // at all, and why it is put back to zero below.
            frames: core::ptr::from_ref::<FrameAllocator>(frames) as usize,
            features,
        });
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
    // The address of the allocator goes away with the process that was allowed
    // to reach it. A capability call arriving after this — which would be a bug
    // in the frame rather than in a process — is refused rather than answered
    // through a pointer to a borrow that has ended.
    // SAFETY: as above.
    unsafe { state.write(State { frames: 0, ..observed }) };
    // SAFETY: volatile, as above; the handler has nothing left to count.
    let in_ring3 = unsafe { ticks.read_volatile() };

    // SAFETY: this core's table, with the process over.
    let table = unsafe { &mut *crate::cap::mine() };
    let still_held = table.used();
    // Everything the process was given and everything it derived, forgotten in
    // one step. Generations survive it, so a handle this process held cannot
    // resolve in the next one — which is the boundary the generation exists for
    // and the one it would be most tempting to reset at.
    table.clear_all();

    // The two paths agree, or the frame is lying to itself about one of them.
    let death = match (outcome, observed.death) {
        (KILLED, death @ Death::Killed { .. }) | (EXITED, death @ Death::Exited(_)) => death,
        _ => return Err(Error::NoDeath),
    };

    let mut count = 0;
    // The two granted frames go back with the rest, and the fact that the list
    // is still this short is the evidence that the live mapping path allocated
    // nothing: a process that could enlarge its own address space would leave
    // tables here that this loop has never heard of, and the free count would
    // not come back.
    for frame in space.tables().iter().copied().chain([text, stack, granted, untyped]) {
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
        granted: granted_count,
        caps: observed.caps,
        held: still_held,
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
        SYS_CAP_DERIVE => derive(Handle::from_bits(first as u32), second),
        SYS_CAP_REVOKE => revoke(Handle::from_bits(first as u32)),
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
fn derive(handle: Handle, asked: u64) -> Result<u64, i32> {
    let asked = u8::try_from(asked)
        .map_err(|_| error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG))?;
    // SAFETY: as `inspect`, and no other reference to the table is live: this
    // is the only one taken in this call.
    let minted = unsafe { table_mut() }.derive(handle, asked)?;
    Ok(u64::from(minted.bits()))
}

/// Withdraw everything derived from a capability, and answer with how many.
fn revoke(handle: Handle) -> Result<u64, i32> {
    // SAFETY: as `derive`.
    let cleared = unsafe { table_mut() }.revoke(handle)?;
    Ok(u64::from(cleared))
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
