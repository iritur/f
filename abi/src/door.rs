// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The door, from the outside: the call numbers and the instruction that makes
//! one.
//!
//! # Why the stub is here and not in the component that uses it
//!
//! Because a component may not contain `unsafe`, and making a system call is
//! an instruction. `user/init` inherits `unsafe_code = "forbid"` from the
//! workspace and the whole point of that crate is that the property is enforced
//! rather than asserted — so the two instructions that cross the boundary live
//! on this side of it, in a crate that is part of the frame and is reviewed as
//! one.
//!
//! That is not a workaround. It is the same division every system makes: the
//! calling convention is the platform's, not the application's, and a component
//! that hand-rolled it would be a component that could get it wrong in a way
//! only the kernel could detect. Here there is one implementation, in the crate
//! whose entire subject is what crosses a trust boundary.
//!
//! # Why these numbers are here and not only in the kernel
//!
//! They are wire. A call number is exactly the kind of thing this crate exists
//! to hold: a fixed-width value with the same meaning on both sides of a
//! boundary, where a disagreement is silent. `kernel::process` names each of
//! them again with the kernel's own reasoning attached, and defines them *as*
//! these — so the two cannot drift.
//!
//! RFC 0014 is the argument for the door being this narrow, and RFC 0015 for
//! the four capability calls being behind it at all. Both name what retires
//! each call, which is an opcode on a ring. When that happens this module
//! shrinks to nothing rather than growing.
//!
//! # What is now true about that, and what is not
//!
//! **The ring exists and carries operations.** RFC 0037 made a channel
//! adoptable in safe code and RFC 0047 showed a component using one in both
//! directions on a real device: `user/virtio-blk` submits
//! `f_abi::control::op::DEVICE_MAP` on its control ring and the frame answers it
//! from a polling loop. So the sentence that used to defer this — *a component
//! cannot drive a ring* — is not the reason any more.
//!
//! **The four capability opcodes are still names.** `control::op::INSPECT`,
//! `DERIVE`, `REVOKE` and `MAP` are defined, `op::known` admits them, and
//! nothing executes them; the two that *are* executed are the two a driver
//! could not do without. Until something executes them there is nowhere for
//! [`CAP_INSPECT`] and its three neighbours to go, and moving them would be
//! moving a call to a refusal.
//!
//! **[`ANNOUNCE`] and [`PROGRESS`] are waiting on something else again.**
//! RFC 0014 retires them when a component is *started with a channel and told
//! on it*. A component is still started by the frame writing a job into a
//! per-core slot and jumping to a fixed address, so there is nothing for an
//! announcement to tell anybody that the frame did not already know, and
//! `PROGRESS` still asks a question the frame answers out of a per-core tick
//! count rather than off a ring.
//!
//! None of that is a plan. `cargo xtask lint-owed` holds all three as a
//! declared set and goes red the day one is paid, which is the day this
//! paragraph is wrong and has to be rewritten rather than quietly left.
//!
//! # What a component may assume about registers
//!
//! Nothing beyond the C ABI. The frame preserves what the System V convention
//! calls callee-saved — `rbx`, `rbp` and `r12` through `r15` — because its
//! dispatcher is an ordinary Rust function; everything else is destroyed, and
//! [`call`] declares it so. A component that assumed more would be reading a
//! value the next kernel change takes away.

/// "I am here." Takes nothing, answers nothing, and the frame records that it
/// happened.
pub const ANNOUNCE: u64 = 0;

/// "Have I run long enough?" Answers [`KEEP_GOING`], [`ENOUGH`] or [`GAVE_UP`].
pub const PROGRESS: u64 = 1;

/// "I am done." The first argument is a status.
pub const EXIT: u64 = 2;

/// "What is this handle?" Answers a packed kind and rights, or an authority
/// error.
pub const CAP_INSPECT: u64 = 3;

/// "Mint me a weaker one." Takes a handle and a rights bitmap, answers a
/// handle.
pub const CAP_DERIVE: u64 = 4;

/// "Take back everything I handed on from this." Answers how many capabilities
/// were withdrawn.
pub const CAP_REVOKE: u64 = 5;

/// "Map this frame into this address space."
///
/// Two registers for four values: the first argument carries the frame handle
/// in its low half and the address space handle in its high half, and the
/// second is a page-aligned address with the requested rights in the twelve
/// bits alignment leaves free. [`map_operands`] and [`map_address`] build them,
/// so that no component has to know the packing and no two components can
/// disagree about it.
pub const CAP_MAP: u64 = 6;

/// The answer to [`PROGRESS`] while the component should carry on.
pub const KEEP_GOING: i64 = 0;

/// The answer once the frame has taken as many ticks out of ring 3 as it
/// wanted.
pub const ENOUGH: i64 = 1;

/// The answer when the frame has given up waiting for those ticks, which is a
/// reason to stop rather than a reason to keep asking.
pub const GAVE_UP: i64 = 2;

/// Pack the two handles [`CAP_MAP`] takes into its first argument.
#[inline]
#[must_use]
pub const fn map_operands(space: crate::cap::Handle, frame: crate::cap::Handle) -> u64 {
    ((space.bits() as u64) << 32) | (frame.bits() as u64)
}

/// Pack the address and the rights [`CAP_MAP`] takes into its second argument.
///
/// The address is truncated to a page boundary rather than refused, because the
/// frame does the same and a component that disagreed about which page it had
/// asked for would be surprised by the answer rather than by the refusal.
#[inline]
#[must_use]
pub const fn map_address(virt: u64, rights: u8) -> u64 {
    (virt & !0xFFF) | (rights as u64)
}

/// What the frame tells a component on entry, in one register.
///
/// # Why a component is told rather than entitled to know
///
/// It used to be entitled. The frame grants a fixed set of capabilities in a
/// fixed order into a cleared table, so the first three handles were the first
/// three slots at the first generation, and a component could write them down.
///
/// That stopped being true the moment a core ran a second process. Generations
/// are not reset between processes — that is the whole point of them, and
/// `kernel::cap::Table::clear_all` says so at the one boundary where resetting
/// would be most tempting — so the second component on a core finds its
/// capabilities at the same *indices* and a later *generation*. A component
/// that assumed otherwise is refused, correctly, for a reason that looks
/// nothing like the mistake.
///
/// So the frame tells it. One register, because the door hands over one: the
/// low half is whatever the frame wanted this run to do, and the high half is
/// the first handle it was granted, from which the rest follow by index.
///
/// This is the smallest possible version of something a component will
/// eventually be *sent*. At M5 a component is started with a channel and its
/// initial capabilities arrive as messages on it, at which point this type is
/// retired along with the door.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Entry(u64);

impl Entry {
    /// Build the word the frame passes.
    #[inline]
    #[must_use]
    pub const fn new(selector: u32, first: crate::cap::Handle) -> Self {
        Self(((first.bits() as u64) << 32) | selector as u64)
    }

    /// From the register.
    #[inline]
    #[must_use]
    pub const fn from_bits(bits: u64) -> Self {
        Self(bits)
    }

    /// To the register.
    #[inline]
    #[must_use]
    pub const fn bits(self) -> u64 {
        self.0
    }

    /// What the frame wanted this run to do. Meaningless to a component that
    /// only has one thing to do, which is why it is the half that is ignored.
    #[inline]
    #[must_use]
    pub const fn selector(self) -> u32 {
        self.0 as u32
    }

    /// The `nth` capability the frame granted, counting from the first.
    ///
    /// The grants go into consecutive slots of a cleared table in a fixed
    /// order, so knowing the first is knowing all of them. That is a promise
    /// about how the frame fills a table rather than about what it grants, and
    /// `kernel::process::GRANTS` is where the order is written down.
    #[inline]
    #[must_use]
    pub const fn granted(self, nth: u16) -> crate::cap::Handle {
        let first = crate::cap::Handle::from_bits((self.0 >> 32) as u32);
        crate::cap::Handle::new(first.index().wrapping_add(nth), first.generation())
    }
}

/// Make one call.
///
/// A negative answer is a packed error — [`crate::error::unpack`] takes it
/// apart — and a non-negative one is whatever the call returns.
///
/// # What this clobbers
///
/// Everything the System V convention lets a call clobber, which is more than
/// the instruction itself touches. `syscall` destroys `rcx` and `r11` by
/// architecture; the frame's dispatcher destroys the rest of the caller-saved
/// set because it is an ordinary function. Declaring the wider set is what
/// makes this correct against a dispatcher that changes, rather than correct
/// against the one that exists today.
#[cfg(target_arch = "x86_64")]
#[inline]
#[must_use]
pub fn call(number: u64, first: u64, second: u64) -> i64 {
    let answer: i64;
    // SAFETY: `syscall` is unprivileged. It transfers to the frame's entry
    // point, which returns through `sysret` with the answer in `rax` and every
    // callee-saved register as it was — the module comment states that contract
    // and `kernel::arch::x86_64::ring3` implements it. The operands below name
    // every register the frame may destroy, so nothing the compiler is holding
    // survives across it by accident. `nostack` because neither the instruction
    // nor the frame uses the caller's stack: the entry switches to a kernel one
    // before it touches anything.
    unsafe {
        core::arch::asm!(
            "syscall",
            inlateout("rax") number => answer,
            inlateout("rdi") first => _,
            inlateout("rsi") second => _,
            lateout("rcx") _,
            lateout("rdx") _,
            lateout("r8") _,
            lateout("r9") _,
            lateout("r10") _,
            lateout("r11") _,
            options(nostack),
        );
    }
    answer
}

/// [`call`] with no arguments.
#[cfg(target_arch = "x86_64")]
#[inline]
#[must_use]
pub fn call0(number: u64) -> i64 {
    call(number, 0, 0)
}
