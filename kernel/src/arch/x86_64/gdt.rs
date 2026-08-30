// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Descriptor tables the kernel owns.
//!
//! # Why this is not optional
//!
//! The boot stub's descriptor table lives in `.data.boot`, at a low physical
//! address, and the kernel's own address space does not map low memory. From
//! the moment `CR3` changed, `GDTR` has pointed at memory that is not there.
//! Nothing has noticed because nothing has reloaded a segment or taken an
//! interrupt — and the first thing to do either would have been the exception
//! handler this file exists to make possible.
//!
//! That is the shape of the bug this milestone removes: a fault whose handler
//! faults, which on x86 is not an error report but a reset.
//!
//! # The task state segment, and the stack that survives a broken stack
//!
//! A double fault is what happens when handling a fault goes wrong, and the
//! most common way for it to go wrong is that the stack is the problem — it
//! overflowed, or it is unmapped. Pushing an exception frame onto a broken
//! stack fails, and a fault while delivering a fault is a triple fault, which
//! is a machine reset with no output at all.
//!
//! The interrupt stack table is the hardware's answer: a descriptor can name a
//! stack to switch to unconditionally, whatever `rsp` currently holds. The
//! double-fault handler uses one. It is the difference between a kernel that
//! reports a stack overflow and a kernel that vanishes.
//!
//! # The three ring-3 descriptors, and why their order is fixed
//!
//! M3 added three more slots. Two of them do something — a code segment and a
//! data segment at privilege level three — and the third exists only because
//! `sysret` computes the selectors it loads by adding fixed offsets to one
//! number in `IA32_STAR`. Positions here are an interface with the processor,
//! not a layout choice, and [`USER_BASE`] says so where somebody would
//! otherwise tidy the gap away.
//!
//! Segments carry almost none of the isolation on this architecture: flat is
//! flat, and the only field that matters in a long-mode descriptor is the
//! privilege level. What keeps a process out of the kernel is `paging::USER`.

use core::arch::asm;

use crate::percpu::PerCpu;

/// Selector for the kernel code segment.
pub const KERNEL_CODE: u16 = 0x08;

/// Selector for the kernel data segment.
pub const KERNEL_DATA: u16 = 0x10;

/// Selector for the task state segment.
const TSS_SELECTOR: u16 = 0x18;

/// Where the three ring-3 descriptors start.
///
/// The order of the three is not a choice. `sysret` computes both selectors it
/// loads from this one number: the stack segment is `USER_BASE + 8` and the
/// 64-bit code segment is `USER_BASE + 16`, each with its requested privilege
/// level forced to three. So the slot at `USER_BASE` itself has to be the
/// 32-bit code segment, which this kernel never loads and cannot omit — a gap
/// there would move the other two and `sysret` would land in whatever
/// followed. `syscall` reads the other half of the same register and requires
/// the same adjacency of the kernel pair, which slots one and two already have.
const USER_BASE: u16 = 0x28;

/// Selector for the user data segment, which is also the ring-3 stack segment.
pub const USER_DATA: u16 = (USER_BASE + 8) | 3;

/// Selector for the 64-bit user code segment.
pub const USER_CODE: u16 = (USER_BASE + 16) | 3;

/// What `IA32_STAR` holds: the two segment bases `syscall` and `sysret` use.
///
/// Bits 47:32 are the kernel pair, bits 63:48 the user pair. The low half of
/// the register is the 32-bit entry point and is meaningless in long mode.
pub const STAR: u64 = ((USER_BASE as u64) << 48) | ((KERNEL_CODE as u64) << 32);

/// The interrupt stack table slot the double-fault handler switches to.
///
/// One-based, because that is how the descriptor encodes it: zero means "do not
/// switch stacks", which is the behaviour this exists to avoid.
pub const DOUBLE_FAULT_IST: u8 = 1;

/// Kernel code: present, executable, readable, 64-bit.
const CODE_DESCRIPTOR: u64 = 0x00AF_9A00_0000_FFFF;

/// Kernel data: present, writable.
const DATA_DESCRIPTOR: u64 = 0x00CF_9200_0000_FFFF;

/// User code, 32-bit. Never loaded by anything this kernel runs.
///
/// It is here because `sysret` names it by position — see [`USER_BASE`] — and a
/// slot that must exist is better filled with the descriptor the architecture
/// says belongs there than with a zero that would be a fault waiting for the
/// first `sysret` to a compatibility-mode process.
const USER_CODE32_DESCRIPTOR: u64 = 0x00CF_FA00_0000_FFFF;

/// User data, ring 3: present, writable. Also the ring-3 stack segment.
const USER_DATA_DESCRIPTOR: u64 = 0x00CF_F200_0000_FFFF;

/// User code, ring 3: present, executable, readable, 64-bit.
///
/// The only difference from [`CODE_DESCRIPTOR`] is two bits of privilege level.
/// That is the whole of what a segment contributes to isolation on this
/// architecture — the rest is in the page tables, which is why `paging::USER`
/// and not this constant is where a mistake would actually cost something.
const USER_CODE_DESCRIPTOR: u64 = 0x00AF_FA00_0000_FFFF;

/// A 64-bit task state segment.
///
/// Two of its fields are live. The interrupt stack table is what makes a double
/// fault reportable; `privilege_stacks[0]` is what makes an interrupt taken
/// from ring 3 land somewhere the kernel owns, and it arrived with user space
/// at M3.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Tss {
    _reserved0: u32,
    /// Stacks for entry from a lower privilege level. Only the first is used:
    /// there are three privilege levels below zero on this architecture and
    /// this kernel has exactly two, ring 0 and ring 3.
    privilege_stacks: [u64; 3],
    _reserved1: u64,
    /// Stacks a descriptor may name unconditionally.
    interrupt_stacks: [u64; 7],
    _reserved2: u64,
    _reserved3: u16,
    /// Past the end of the segment, which is how the absence of an I/O
    /// permission bitmap is spelled.
    iomap_base: u16,
}

/// What the processor loads from `GDTR` and `IDTR`.
#[repr(C, packed)]
pub struct DescriptorPointer {
    /// One less than the table's size in bytes.
    pub limit: u16,
    /// Where the table is.
    pub base: u64,
}

/// How many descriptors the table holds.
///
/// Null, kernel code, kernel data, two slots for the system descriptor that
/// names the task state segment — system descriptors are sixteen bytes — and
/// then the three ring-3 descriptors at [`USER_BASE`], in the order `sysret`
/// requires.
const SLOTS: usize = 8;

/// The table.
///
/// Per core, because the fourth and fifth slots name *this* core's task state
/// segment, and a segment is not something two cores can share.
static GDT: PerCpu<[u64; SLOTS]> = PerCpu::new([0; SLOTS]);

/// Per core, which is what forces the table above to be: this is where the
/// stack pointers live.
///
/// Sharded all the way down, since E0-B10. It used to say the opposite: the
/// double-fault stack every one of these named was the single one the linker
/// script reserved, so the table was private and the stack under it was not,
/// and two cores taking a double fault would have written their exception
/// frames to one address — corrupting the report each was trying to make. The
/// stacks are now a block per core, reserved by the same linker script for the
/// reason it gives there, and [`init`] picks the block belonging to the core it
/// is called on.
static TSS: PerCpu<Tss> = PerCpu::new(Tss {
    _reserved0: 0,
    privilege_stacks: [0; 3],
    _reserved1: 0,
    interrupt_stacks: [0; 7],
    _reserved2: 0,
    _reserved3: 0,
    iomap_base: core::mem::size_of::<Tss>() as u16,
});

unsafe extern "C" {
    /// The far end of the double-fault stack, from the linker script.
    ///
    /// It lives there rather than in a `static` here so that it can have an
    /// unmapped page below it. A handler whose own stack overflows silently is
    /// worse than no handler, because it looks like the fault it was reporting.
    static __fault_stack_top: u8;
}

/// Install the kernel's descriptor tables and start using them.
///
/// # Safety
///
/// Call once per core, on that core, before enabling interrupts on it. It
/// installs the calling core's own tables and no other core's — which is what
/// makes it the right shape for the application processors, and why the
/// obligation is once *per core* rather than once. Reloading the segment
/// registers and the code segment mid-flight is only sound because the
/// descriptors being installed describe the same flat address space the caller
/// is already running in.
pub unsafe fn init() {
    // The stack the double-fault handler switches to, and it has to be this
    // core's. The boot processor's is the one the linker script names outright;
    // every core that can be started has a block of its own beside it. A shared
    // one would be two cores writing an exception frame to one address at the
    // moment each is trying to report why it cannot use its own stack.
    let fault_stack_top = super::ap::fault_stack_top(super::current_cpu())
        .unwrap_or((&raw const __fault_stack_top) as u64);

    let tss = TSS.mine();
    // SAFETY: this core's slot, on the boot path, and nothing else refers to
    // the task state segment until it is installed below.
    unsafe {
        tss.write(Tss {
            _reserved0: 0,
            privilege_stacks: [0; 3],
            _reserved1: 0,
            interrupt_stacks: [fault_stack_top, 0, 0, 0, 0, 0, 0],
            _reserved2: 0,
            _reserved3: 0,
            iomap_base: core::mem::size_of::<Tss>() as u16,
        });
    }

    let tss_base = tss as u64;
    let (low, high) = tss_descriptor(tss_base, core::mem::size_of::<Tss>() as u32 - 1);

    // One write per `unsafe` block, with the offset arithmetic outside it:
    // computing a pointer is not the dangerous act, and a block covering five
    // writes has a SAFETY comment covering whichever one the reader thinks of.
    let gdt = GDT.mine().cast::<u64>();
    for (index, descriptor) in [
        (0, 0),
        (1, CODE_DESCRIPTOR),
        (2, DATA_DESCRIPTOR),
        (3, low),
        (4, high),
        (usize::from(USER_BASE / 8), USER_CODE32_DESCRIPTOR),
        (usize::from(USER_BASE / 8) + 1, USER_DATA_DESCRIPTOR),
        (usize::from(USER_BASE / 8) + 2, USER_CODE_DESCRIPTOR),
    ] {
        let at = gdt.wrapping_add(index);
        // SAFETY: this core's own table, before anything can observe it partly
        // built, and the index is one of the five slots it has.
        unsafe { at.write(descriptor) };
    }

    let pointer = DescriptorPointer {
        limit: (core::mem::size_of::<[u64; SLOTS]>() - 1) as u16,
        base: gdt as u64,
    };

    // SAFETY: the pointer describes the table built immediately above, at an
    // address in the kernel window, which is mapped in the address space the
    // caller is running in — unlike the one this replaces.
    unsafe {
        asm!(
            "lgdt [{ptr}]",
            ptr = in(reg) &pointer,
            options(readonly, nostack, preserves_flags),
        );
    }

    // SAFETY: the code selector names the descriptor written above. A far
    // return is how a 64-bit code segment is reloaded: there is no `mov cs`.
    unsafe {
        asm!(
            "push {code:r}",
            "lea {tmp}, [rip + 2f]",
            "push {tmp}",
            "retfq",
            "2:",
            code = in(reg) u64::from(KERNEL_CODE),
            tmp = lateout(reg) _,
            options(preserves_flags),
        );
    }

    // SAFETY: the data selector names the descriptor written above, and every
    // data segment in long mode is flat regardless of what it says.
    unsafe {
        asm!(
            "mov ds, {sel:x}",
            "mov es, {sel:x}",
            "mov ss, {sel:x}",
            "mov fs, {sel:x}",
            "mov gs, {sel:x}",
            sel = in(reg) KERNEL_DATA,
            options(nostack, preserves_flags),
        );
    }

    // SAFETY: the selector names the system descriptor written above, which
    // describes a task state segment that is now fully initialised.
    unsafe {
        asm!("ltr {sel:x}", sel = in(reg) TSS_SELECTOR, options(nostack, preserves_flags));
    }
}

/// Where the processor is to put an interrupt frame taken from ring 3.
///
/// # Why the caller supplies it rather than this module owning a stack
///
/// The obvious arrangement is a per-core stack of its own, named here and
/// pointed at once. It is wrong for the same reason a per-thread kernel stack
/// exists in every kernel that has threads: this address is where the processor
/// starts writing when it leaves ring 3, so it must be below everything the
/// kernel is currently using and above nothing. The only code that knows that
/// address is the code that is about to enter ring 3, and it knows it exactly
/// once — as its own stack pointer at the moment of the transition.
///
/// A fixed stack top here would be *above* the live kernel frames, and the
/// first interrupt from ring 3 would overwrite them. That failure is silent,
/// arrives on the first tick, and presents as corruption in whatever the kernel
/// was doing rather than as anything to do with user space.
///
/// The pointer is handed out rather than the write performed, because the
/// caller is assembly that has to do it after its own last push and cannot call
/// back into Rust to do so. See `ring3::enter`.
#[must_use]
pub fn kernel_stack_slot() -> *mut u64 {
    let tss = TSS.mine();
    // SAFETY: no dereference happens — this computes the address of a field of
    // this core's own task state segment. The slot is written by assembly that
    // uses an unaligned store, which is what the packed layout requires and
    // what the processor itself does when it reads the field back.
    unsafe { &raw mut (*tss).privilege_stacks[0] }
}

/// Build the two halves of a system descriptor for a task state segment.
///
/// Sixteen bytes rather than eight, because a 64-bit base does not fit in the
/// layout inherited from 1985.
fn tss_descriptor(base: u64, limit: u32) -> (u64, u64) {
    let mut low: u64 = 0;
    low |= u64::from(limit & 0xFFFF);
    low |= (base & 0xFF_FFFF) << 16;
    // Type 0b1001: an available 64-bit task state segment. Present.
    low |= 0b1000_1001 << 40;
    low |= ((base >> 24) & 0xFF) << 56;

    let high = base >> 32;
    (low, high)
}
