// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The program that is not the kernel.
//!
//! Sixty-odd instructions of x86-64, assembled into `.rodata` and copied into a
//! frame the process owns. It is here rather than in `user/init` because
//! E0-B10 is what loads a real component from a boot module: until then, a
//! process needs *something* to run, and the honest smallest something is a
//! flat blob with no loader, no relocations and no dependency on any crate.
//!
//! # What it is for
//!
//! Two things, and both are properties of the frame rather than of the program.
//!
//! **That ring 3 runs, and can come back.** It announces itself through
//! `syscall`, then asks the frame — repeatedly — whether it has run long
//! enough. The answer is the frame's, which is what makes the boot log a
//! fixture: a process that measured its own lifetime in instructions would run
//! for a different number of timer ticks on every machine, and the number of
//! ticks is exactly what this milestone has to be able to state.
//!
//! **That ring 3 cannot do what it is not permitted to.** Then it provokes,
//! deliberately, whichever violation it was told to. Each one is a protection
//! that is otherwise only asserted: a mapping the kernel believes is
//! unreachable, a page the kernel believes is not writable, an instruction the
//! kernel believes ring 3 cannot execute. `E0-B19` put the same argument the
//! other way round — *a protection nothing tries to violate is a protection
//! nobody has checked* — and these are the ring-3 half of it.
//!
//! # The calling convention, such as it is
//!
//! One word arrives in `rdi`: which violation to commit. `rax` carries the call
//! number into `syscall` and the answer out of it; `rdi` and `rsi` are the two
//! arguments. The frame preserves what the C ABI calls callee-saved — `rbx`,
//! `rbp` and `r12` through `r15` — because the dispatcher is an ordinary
//! function and preserves them; everything else is destroyed. That is a
//! consequence of how the entry is written rather than a promise the design
//! makes, and it is written down here because the program depends on it: it
//! keeps its one argument in `rbx` across every call it makes.
//!
//! It is not a wire format and it does not need one. The interface a component
//! is meant to use is the ring (M5), and this is what exists before it.

/// Read the direct map. Present in this address space, and not ring 3's.
pub const PROVOKE_KERNEL: u64 = 0;

/// Write to the page at address zero, which nothing maps.
pub const PROVOKE_NULL: u64 = 1;

/// Write to its own text, which is executable and therefore not writable.
pub const PROVOKE_TEXT: u64 = 2;

/// Execute its own stack, which is writable and therefore not executable.
pub const PROVOKE_STACK: u64 = 3;

/// Execute an instruction only ring 0 may.
pub const PROVOKE_PRIV: u64 = 4;

/// Make a call the frame does not have, then end normally.
pub const PROVOKE_CALL: u64 = 5;

/// Provoke nothing and ask to end.
pub const PROVOKE_EXIT: u64 = 6;

unsafe extern "C" {
    /// First byte of the program.
    static user_probe_start: u8;
    /// One past its last byte.
    static user_probe_end: u8;
}

/// The program, as bytes to be copied into a frame.
///
/// A slice of `.rodata`, which is mapped read-only and never executable in the
/// kernel window — so the copy is the only way these bytes ever get executed,
/// and they get executed at ring 3 or not at all.
#[must_use]
pub fn program() -> &'static [u8] {
    let start = (&raw const user_probe_start).cast::<u8>();
    let end = (&raw const user_probe_end).cast::<u8>();
    let len = (end as usize) - (start as usize);
    // SAFETY: both symbols are emitted by the assembly below, in that order, in
    // one section, so the region between them is exactly the program. It is in
    // `.rodata` and immutable for the life of the kernel, which is what makes a
    // `'static` shared slice of it sound.
    unsafe { core::slice::from_raw_parts(start, len) }
}

core::arch::global_asm!(
    r#"
    .section .rodata, "a", @progbits
    .balign 16
    .globl user_probe_start
user_probe_start:
    // Position-independent throughout: this runs from a completely different
    // address than the one it is assembled at, so every branch is relative and
    // every reference to itself is relative to the instruction pointer. The
    // only absolute address in the program is the kernel one it is not allowed
    // to read, and that is an immediate rather than a reference.
    movq %rdi, %rbx

    // "I am here." The frame answers by recording that it happened; it prints
    // nothing until the process is over, because the timer is running and a
    // serial port inside a tick interval is a jitter measurement of the serial
    // port.
    xorl %eax, %eax
    syscall

    // "Have I run long enough?" Zero means keep going. Anything else means
    // stop, and the frame knows which of the two reasons it was.
.Lprobe_ask:
    movl $1, %eax
    syscall
    testq %rax, %rax
    jnz .Lprobe_provoke
    movl $4096, %ecx
.Lprobe_spin:
    decl %ecx
    jnz .Lprobe_spin
    jmp .Lprobe_ask

.Lprobe_provoke:
    cmpq $0, %rbx
    je .Lprobe_read_kernel
    cmpq $1, %rbx
    je .Lprobe_write_null
    cmpq $2, %rbx
    je .Lprobe_write_text
    cmpq $3, %rbx
    je .Lprobe_exec_stack
    cmpq $4, %rbx
    je .Lprobe_privileged
    cmpq $5, %rbx
    je .Lprobe_bad_call
    jmp .Lprobe_exit

    // The direct map. The page is there, it is not marked for ring 3, and the
    // fault that follows says "protection violation" rather than "not present"
    // — which is the stronger of the two statements, because it proves the
    // mapping was reachable and refused rather than merely absent.
.Lprobe_read_kernel:
    movabsq $0xffff800000000000, %rax
    movq (%rax), %rax
    jmp .Lprobe_survived

.Lprobe_write_null:
    xorl %eax, %eax
    movq %rax, (%rax)
    jmp .Lprobe_survived

    // Write-exclusive-or-execute, from the side that can only be tested by a
    // process: this is the page the next instruction would have come from.
.Lprobe_write_text:
    leaq .Lprobe_write_text(%rip), %rax
    movb $0x90, (%rax)
    jmp .Lprobe_survived

    // The other side of the same rule. The byte written here is a `ret`, so
    // that a machine which executed it would carry on rather than wander —
    // making the failure a clean return to the survival path instead of an
    // unrelated crash somewhere else.
.Lprobe_exec_stack:
    leaq -16(%rsp), %rax
    movb $0xc3, (%rax)
    jmp *%rax

.Lprobe_privileged:
    cli
    jmp .Lprobe_survived

    // Not a violation: a call the frame does not have. It must be refused and
    // must not be fatal, so the program carries straight on into asking to end.
.Lprobe_bad_call:
    movl $99, %eax
    syscall

.Lprobe_exit:
    movl $2, %eax
    xorl %edi, %edi
    syscall

    // Every provocation above is supposed to end the process. Reaching here
    // means one of them did not, and the only useful thing left is to say so:
    // the frame reads the status, and a boot where a protection did not hold
    // fails rather than passing quietly.
.Lprobe_survived:
    movl $2, %eax
    movl $1, %edi
    syscall
    jmp .Lprobe_survived

    .globl user_probe_end
user_probe_end:
"#,
    options(att_syntax)
);
