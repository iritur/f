// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Leaving ring 0, and the two ways of coming back.
//!
//! # The three transitions
//!
//! **Out**, with `iretq`. There is no instruction for "start running at
//! privilege level three"; there is only the instruction for returning to where
//! an interrupt came from, and a frame that says it came from there. [`enter`]
//! builds that frame by hand. Everything about the transition is in those five
//! quadwords, which is why they are pushed in one place and never assembled
//! anywhere else.
//!
//! **Back, on purpose**, with `syscall` and `sysret`. The design document is
//! explicit that this entry exists for channel setup and never for a hot path —
//! `docs/design/ring-scene-boot.html` section 15, M3 — so it is deliberately
//! plain: no register save area, no per-call bookkeeping, and a dispatch that
//! is a `match`. Making it fast would be optimising the path the architecture
//! exists to avoid using.
//!
//! **Back, involuntarily**, through the interrupt table. A fault taken at ring
//! 3 arrives at exactly the same stubs as a fault taken at ring 0, and the only
//! difference is one bit of the saved code selector. That bit is the whole of
//! the difference between a process that dies and a kernel that dies, and
//! [`resume`] is where it is acted on.
//!
//! # `GS`, and why there is no alternative
//!
//! `syscall` does not switch stacks. On entry the kernel is executing at ring 0
//! on a stack the process chose, with every register still holding the
//! process's values and nowhere to put one. `swapgs` is the architecture's
//! answer: one instruction that exchanges `GS.base` with a value the kernel
//! parked in a model-specific register, giving a segment-relative address to a
//! per-core scratch area without needing a free register to compute it.
//!
//! So `IA32_KERNEL_GS_BASE` holds this core's [`Entry`] block whenever ring 3
//! is running, and `GS.base` holds zero — the value a process would see if it
//! read it. The stub swaps them on the way in and back on the way out. Nothing
//! else in this kernel touches `GS`, which is what makes the invariant checkable
//! by reading one file.
//!
//! # The stack a process is entered from is the stack it is killed on
//!
//! [`enter`] records its own stack pointer three times: in the task state
//! segment, so an interrupt from ring 3 has somewhere to land; in the [`Entry`]
//! block, so `syscall` has somewhere to land; and as the place execution
//! resumes when the process ends. All three are the same address because all
//! three are the same claim — everything below that address is free, and
//! everything above it is the kernel call that is waiting for the process to be
//! over. `gdt::kernel_stack_slot` says what goes wrong if a fixed stack is used
//! instead.

use core::arch::global_asm;

use super::{gdt, idt, read_msr, write_msr};
use crate::percpu::PerCpu;

/// `IA32_EFER`. Bit 0 is the switch that makes `syscall` a system call rather
/// than an invalid opcode.
const IA32_EFER: u32 = 0xC000_0080;

/// `IA32_STAR`: the segment selectors `syscall` and `sysret` load.
const IA32_STAR: u32 = 0xC000_0081;

/// `IA32_LSTAR`: where `syscall` jumps.
const IA32_LSTAR: u32 = 0xC000_0082;

/// `IA32_FMASK`: the flags `syscall` clears on entry.
const IA32_FMASK: u32 = 0xC000_0084;

/// `IA32_KERNEL_GS_BASE`: the value `swapgs` brings in.
const IA32_KERNEL_GS_BASE: u32 = 0xC000_0102;

/// `IA32_GS_BASE`: the value a process sees, and the value `swapgs` puts back.
const IA32_GS_BASE: u32 = 0xC000_0101;

/// `EFER.SCE`, system call extensions.
const EFER_SCE: u64 = 1 << 0;

/// Flags cleared by the processor on entry to [`syscall_entry`].
///
/// The interrupt flag is the one that is not a preference. `syscall` does not
/// switch stacks, so between the entry point and the two instructions that move
/// to the kernel's stack the processor is at ring 0 with `rsp` pointing at
/// memory the process chose. An interrupt delivered in that window would push a
/// frame there. Clearing the flag here closes the window in hardware rather
/// than in a comment.
///
/// The rest are hygiene the kernel would otherwise have to perform: the
/// direction flag, because everything below assumes it is clear and the
/// interrupt stubs already say so with a `cld`; the trap flag, so a process
/// cannot single-step the kernel; nested task and alignment check, because
/// neither means anything here and both are settable from ring 3.
const FMASK: u64 = (1 << 8) | (1 << 9) | (1 << 10) | (1 << 14) | (1 << 18);

/// The flags a process starts with: the reserved bit, and interrupts enabled.
///
/// Enabled deliberately. A process that ran with interrupts off would hold the
/// core against the timer, and the whole claim this milestone has to support is
/// that it does not.
const RFLAGS_USER: u64 = 0x202;

/// The flags the kernel is resumed with when a process ends.
///
/// Interrupts off, and then on again at one known instruction inside
/// [`enter`] rather than as a side effect of an `iretq`. The transition back
/// changes the stack, the privilege level and the address space's usefulness
/// all at once, and an interrupt landing in the middle of that would be
/// delivered against a state that is briefly nobody's.
const RFLAGS_KERNEL: u64 = 0x002;

/// Per-core scratch for the two entries that cannot use a stack to find one.
///
/// Reached two ways, and that is the point: the assembly stubs address it
/// through `GS` because they have no register to spare, and Rust addresses it
/// through [`PerCpu`] because it has. Both reach the same bytes.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct Entry {
    /// The kernel stack a `syscall` from ring 3 switches to, and the stack the
    /// kernel resumes on when the process ends. One address, because it is one
    /// claim — see the module comment.
    kernel_rsp: u64,
    /// Where the stub parks the process's stack pointer for the duration of a
    /// call, because `sysret` has to put it back and there is nowhere else.
    user_rsp: u64,
    /// The instruction the kernel resumes at when the process ends, whichever
    /// way it ends. Zero when no process is running, which is what makes
    /// [`resume`] able to refuse.
    resume_rip: u64,
}

/// This core's entry block.
static ENTRY: PerCpu<Entry> = PerCpu::new(Entry { kernel_rsp: 0, user_rsp: 0, resume_rip: 0 });

unsafe extern "C" {
    /// Where `syscall` lands. A symbol rather than a function, because nothing
    /// in Rust may call it: it arrives with the process's registers live and
    /// leaves through `sysret`.
    static syscall_entry: u8;

    /// Build an interrupt frame that says ring 3 and return through it.
    ///
    /// Returns the value the resume path left in `rax`, which is how the
    /// process ended.
    fn enter_user(rip: u64, rsp: u64, argument: u64, rsp0_slot: *mut u64, block: *mut Entry)
    -> u64;
}

/// Make `syscall` mean something on this core.
///
/// # Safety
///
/// Call once per core, on that core, after [`gdt::init`] on the same core —
/// every selector written here names a descriptor that installs — and before
/// anything enters ring 3 on it.
pub unsafe fn init() {
    // SAFETY: `IA32_EFER` exists on every processor that can be in long mode,
    // which this one is, or none of the code around this would be running.
    let efer = unsafe { read_msr(IA32_EFER) };
    // Read, modify, write. The register also holds long-mode enable and the
    // no-execute switch `paging::enable_features` set, and assigning it would
    // turn both off — which is a triple fault on the next instruction fetch.
    // SAFETY: as above, and the value differs from what was read in one defined
    // bit.
    unsafe { write_msr(IA32_EFER, efer | EFER_SCE) };

    // SAFETY: the four system-call registers exist wherever `EFER.SCE` does.
    // `STAR` is a whole-register field pair with nothing else in it, so it is
    // assigned rather than merged.
    unsafe { write_msr(IA32_STAR, gdt::STAR) };
    // SAFETY: as above. The address is a symbol in the kernel window, which is
    // mapped in every address space a `syscall` can arrive from — including a
    // process's, whose upper half is a copy of the kernel's.
    unsafe { write_msr(IA32_LSTAR, (&raw const syscall_entry) as u64) };
    // SAFETY: as above.
    unsafe { write_msr(IA32_FMASK, FMASK) };

    // The value `swapgs` brings in. It is written to the *kernel* half because
    // that is the half that is inactive while ring 0 runs: the swap on entry to
    // a system call is what makes it current.
    // SAFETY: as above. The address is this core's own slot in a `.bss` static,
    // which does not move.
    unsafe { write_msr(IA32_KERNEL_GS_BASE, ENTRY.mine() as u64) };
    // What a process sees if it reads its own `GS.base`, and what the stub puts
    // back on the way out. Zero rather than left as found, so that a process
    // cannot be handed a kernel address by an omission.
    // SAFETY: as above.
    unsafe { write_msr(IA32_GS_BASE, 0) };
}

/// Run a process until it stops, and return how it stopped.
///
/// `argument` is the one word the process is told on entry; every other
/// register is zeroed, because a register left holding a kernel value is a
/// kernel address handed to ring 3 by accident rather than by grant.
///
/// # Safety
///
/// The address space in `CR3` must be the process's, `rip` and `rsp` must be
/// mapped in it at ring 3, [`init`] must have run on this core, and interrupts
/// must be enabled — this returns with them enabled, and a caller that had them
/// disabled would silently get them back on. There must be no process already
/// running on this core: the entry block holds one resume point, and a second
/// entry would overwrite the first.
pub unsafe fn enter(rip: u64, rsp: u64, argument: u64) -> u64 {
    let slot = gdt::kernel_stack_slot();
    // SAFETY: the caller's guarantees. The assembly writes this core's task
    // state segment and this core's entry block, and reads neither afterwards
    // except through the paths this module owns.
    unsafe { enter_user(rip, rsp, argument, slot, ENTRY.mine()) }
}

/// Point an interrupt frame back at the kernel, so that `iretq` ends the
/// process instead of resuming it.
///
/// Returns false when there is no process to end, which is a kernel bug rather
/// than a condition: it means a fault arrived carrying a ring-3 code selector
/// on a core that never entered ring 3.
///
/// # Safety
///
/// `frame` must be the frame an interrupt stub is about to restore from, and
/// the fault it describes must have been taken at ring 3. Rewriting a ring-0
/// frame with this would return the kernel to a stack it is already using.
#[must_use]
pub unsafe fn resume(frame: &mut idt::Frame, outcome: u64) -> bool {
    // SAFETY: this core's slot, read by value. The only writer is the assembly
    // in this file, which runs on this core with no interrupt able to interleave
    // between its two stores and this read — they are separated by the whole
    // life of a process.
    let block = unsafe { ENTRY.mine().read() };
    if block.resume_rip == 0 {
        return false;
    }

    frame.rip = block.resume_rip;
    frame.rsp = block.kernel_rsp;
    frame.cs = u64::from(gdt::KERNEL_CODE);
    frame.ss = u64::from(gdt::KERNEL_DATA);
    frame.rflags = RFLAGS_KERNEL;
    frame.rax = outcome;
    true
}

/// End the process from inside a system call, without returning to it.
///
/// The other half of [`resume`], for the path where nothing went wrong. It
/// cannot be written as a return value: the stub's only way out is `sysret`,
/// and `sysret` goes back to ring 3.
///
/// # Safety
///
/// Call only from [`syscall_dispatch`], on a core with a process running.
/// Everything after this point on the current stack is abandoned.
unsafe fn leave(outcome: u64) -> ! {
    // SAFETY: as [`resume`], and this runs with interrupts masked by `FMASK`,
    // so no handler can be between the two stores either.
    let block = unsafe { ENTRY.mine().read() };

    // SAFETY: `swapgs` restores the invariant the module comment states before
    // control leaves the system-call path; the stack pointer is one the
    // assembly in this file recorded, and the address jumped to is the label
    // immediately after the `iretq` that started the process. Interrupts stay
    // masked across all three, and are enabled again at that label.
    unsafe {
        core::arch::asm!(
            "swapgs",
            "mov rsp, {rsp}",
            "jmp {rip}",
            rsp = in(reg) block.kernel_rsp,
            rip = in(reg) block.resume_rip,
            in("rax") outcome,
            options(noreturn),
        );
    }
}

/// Forget the process this core was running.
///
/// Called after a process has ended, so that a later fault carrying a ring-3
/// selector — which would be a kernel bug — cannot be answered by returning to
/// a stack frame that no longer exists.
///
/// # Safety
///
/// Call on the core that entered ring 3, after [`enter`] has returned.
pub unsafe fn forget() {
    let block = ENTRY.mine();
    // SAFETY: this core's slot, with no process running, so the assembly that
    // is the only other writer cannot be executing.
    unsafe { block.write(Entry { kernel_rsp: 0, user_rsp: 0, resume_rip: 0 }) };
}

/// Where a system call from ring 3 arrives in Rust.
///
/// # Safety
///
/// Called only from [`syscall_entry`], with the process's arguments already
/// shuffled into the C argument registers. Not to be called from Rust: it may
/// not return.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn syscall_dispatch(number: u64, first: u64, second: u64) -> u64 {
    match crate::process::syscall(number, first, second) {
        crate::process::Answer::Reply(value) => value,
        // SAFETY: reached only from the stub, which is only reached from a
        // process, so there is one to end.
        crate::process::Answer::Ended(outcome) => unsafe { leave(outcome) },
    }
}

global_asm!(
    r#"
    .section .text, "ax", @progbits
    .code64

    // ---- the deliberate way back ------------------------------------------
    //
    // Arrives with: rcx the process's next instruction, r11 its flags, rax the
    // call number, rdi and rsi its two arguments. Everything else is the
    // process's and is left alone; the C ABI's callee-saved registers survive
    // because `syscall_dispatch` is an ordinary function and preserves them,
    // which is the only reason a process may keep anything across a call.
    .globl syscall_entry
syscall_entry:
    swapgs
    movq %rsp, %gs:{user_rsp}
    movq %gs:{kernel_rsp}, %rsp

    // The two registers `sysret` needs back, and the only two the processor
    // destroyed on the way in.
    pushq %rcx
    pushq %r11

    // (rax, rdi, rsi) -> (rdi, rsi, rdx). In that order: every register is read
    // before the move that overwrites it.
    movq %rsi, %rdx
    movq %rdi, %rsi
    movq %rax, %rdi
    call syscall_dispatch

    popq %r11
    popq %rcx
    movq %gs:{user_rsp}, %rsp
    swapgs
    sysretq

    // ---- the way out ------------------------------------------------------
    //
    // rdi the entry point, rsi the process's stack, rdx the one word it is
    // told, rcx the task state segment slot, r8 this core's entry block.
    .globl enter_user
enter_user:
    pushq %rbp
    pushq %rbx
    pushq %r12
    pushq %r13
    pushq %r14
    pushq %r15
    // Six pushes leave the stack eight bytes out. The kernel re-enters here
    // with an interrupt frame, and the processor aligns one to sixteen.
    subq $8, %rsp

    movq %rsp, {kernel_rsp}(%r8)
    // Where an interrupt taken at ring 3 will be delivered. Written last of the
    // three because it is the one the hardware reads without being asked.
    movq %rsp, (%rcx)
    leaq 1f(%rip), %rax
    movq %rax, {resume_rip}(%r8)

    // From here to the `iretq` the machine is between two privilege levels:
    // the task state segment already names this stack and the frame that would
    // use it is still being built.
    cli
    pushq ${user_ss}
    pushq %rsi
    pushq ${user_flags}
    pushq ${user_cs}
    pushq %rdi

    // The one word ring 3 is told, and then every other register cleared. A
    // register still holding a kernel value on the far side of this instruction
    // is an address the process was never granted.
    movq %rdx, %rdi
    xorl %eax, %eax
    xorl %esi, %esi
    xorl %edx, %edx
    xorl %ecx, %ecx
    xorl %ebx, %ebx
    xorl %ebp, %ebp
    xorl %r8d, %r8d
    xorl %r9d, %r9d
    xorl %r10d, %r10d
    xorl %r11d, %r11d
    xorl %r12d, %r12d
    xorl %r13d, %r13d
    xorl %r14d, %r14d
    xorl %r15d, %r15d
    iretq

    // Where the process ends, both ways: the fault path arrives by `iretq` out
    // of an interrupt stub, the exit path by a plain jump. Both have already
    // put the stack back.
1:
    sti
    addq $8, %rsp
    popq %r15
    popq %r14
    popq %r13
    popq %r12
    popq %rbx
    popq %rbp
    ret
"#,
    kernel_rsp = const core::mem::offset_of!(Entry, kernel_rsp),
    user_rsp = const core::mem::offset_of!(Entry, user_rsp),
    resume_rip = const core::mem::offset_of!(Entry, resume_rip),
    user_ss = const gdt::USER_DATA as u32,
    user_cs = const gdt::USER_CODE as u32,
    user_flags = const RFLAGS_USER as u32,
    options(att_syntax)
);
