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

use core::arch::asm;

use crate::percpu::PerCpu;

/// Selector for the kernel code segment.
pub const KERNEL_CODE: u16 = 0x08;

/// Selector for the kernel data segment.
pub const KERNEL_DATA: u16 = 0x10;

/// Selector for the task state segment.
const TSS_SELECTOR: u16 = 0x18;

/// The interrupt stack table slot the double-fault handler switches to.
///
/// One-based, because that is how the descriptor encodes it: zero means "do not
/// switch stacks", which is the behaviour this exists to avoid.
pub const DOUBLE_FAULT_IST: u8 = 1;

/// Kernel code: present, executable, readable, 64-bit.
const CODE_DESCRIPTOR: u64 = 0x00AF_9A00_0000_FFFF;

/// Kernel data: present, writable.
const DATA_DESCRIPTOR: u64 = 0x00CF_9200_0000_FFFF;

/// A 64-bit task state segment.
///
/// Most of it is for privilege-level stack switching, which arrives with user
/// space at M3. The part in use now is the interrupt stack table.
#[repr(C, packed)]
#[derive(Clone, Copy)]
struct Tss {
    _reserved0: u32,
    /// Stacks for entry from a lower privilege level. M3.
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

/// Null, kernel code, kernel data, and two slots for the system descriptor
/// that names the task state segment — system descriptors are sixteen bytes.
///
/// Per core, because the fourth and fifth slots name *this* core's task state
/// segment, and a segment is not something two cores can share.
static GDT: PerCpu<[u64; 5]> = PerCpu::new([0; 5]);

/// Per core, which is what forces the table above to be: this is where the
/// stack pointers live.
///
/// Sharded here and not yet everywhere it needs to be. The double-fault stack
/// each of these names comes from the linker script, and there is exactly one
/// of it — so on a second core this table would be private and the stack it
/// points at would not be. Sharding the stacks belongs with the code that
/// starts the second core (E0-B10), because a stack needs a guard page under
/// it and a guard page needs the mapper, which does not exist this early.
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
    // The stack the double-fault handler switches to. The linker script names
    // its far end, because a stack grows down from there.
    let fault_stack_top = (&raw const __fault_stack_top) as u64;

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
    for (index, descriptor) in
        [(0, 0), (1, CODE_DESCRIPTOR), (2, DATA_DESCRIPTOR), (3, low), (4, high)]
    {
        let at = gdt.wrapping_add(index);
        // SAFETY: this core's own table, before anything can observe it partly
        // built, and the index is one of the five slots it has.
        unsafe { at.write(descriptor) };
    }

    let pointer = DescriptorPointer {
        limit: (core::mem::size_of::<[u64; 5]>() - 1) as u16,
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
