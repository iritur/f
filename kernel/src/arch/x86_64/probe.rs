// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The program that is not the kernel.
//!
//! Sixty-odd instructions of x86-64, assembled into `.rodata` and copied into a
//! frame the process owns.
//!
//! It was here rather than in `user/init` because there was no loader: a
//! process needed *something* to run, and the honest smallest something is a
//! flat blob with no relocations and no dependency on any crate. E0-B10 brought
//! the loader, and this stayed — which is the part worth reading. A suite that
//! could only test a component somebody supplied would be a suite that stops
//! working when nobody supplies one, and more to the point this program is
//! written in assembly *so that it can attempt things Rust will not express*:
//! forging a handle out of arithmetic, executing its own stack, running an
//! instruction only ring 0 may. `user/init` is what a component looks like;
//! this is what the frame has to survive.
//!
//! Both run on every boot, the component first. `kernel::process::Plan` carries
//! whichever program a run is for.
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
//! One word arrives in `rdi`, and it is [`f_abi::door::Entry`]: which violation
//! to commit in its low half, and the first capability the frame granted in its
//! high half. `rax` carries the call
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

use f_abi::cap::{Handle, rights};

use crate::cap::TABLE_SLOTS;

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

/// Use the capabilities it was given, correctly. The positive control.
pub const PROVOKE_CAP_GRANT: u64 = 7;

/// Name a slot the frame never filled.
pub const PROVOKE_CAP_UNOWNED: u64 = 8;

/// Sweep the handle space, in range and out of it.
pub const PROVOKE_CAP_FORGE: u64 = 9;

/// Use a capability after the tree it hangs from was revoked.
pub const PROVOKE_CAP_STALE: u64 = 10;

/// Ask for rights the capability does not carry.
pub const PROVOKE_CAP_RIGHTS: u64 = 11;

/// Present a capability of the wrong kind for the operand.
pub const PROVOKE_CAP_TYPE: u64 = 12;

/// Derive until the table is full.
pub const PROVOKE_CAP_FLOOD: u64 = 13;

/// Read a mapping after the capability it was made through has been revoked.
pub const PROVOKE_CAP_UNMAP: u64 = 14;

/// Store into the state tree, which was granted read-only. E0-B14.
pub const PROVOKE_CAP_STATE: u64 = 15;

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
    // The one word the frame hands over. Its low half is which violation to
    // commit; its high half is the first capability the frame granted, from
    // which the other two follow by index. `f_abi::door::Entry` is the packing
    // and argues why a component is told rather than entitled to know: a second
    // process on this core finds its capabilities at a later generation, and one
    // that assumed otherwise would be refused for a reason that looks nothing
    // like the mistake.
    //
    // Kept in the three registers a call preserves, because everything else is
    // destroyed by `syscall`.
    movq %rdi, %r13
    shrq $32, %r13              // the address space capability
    movl %r13d, %r12d
    incl %r12d                  // the frame capability, one slot along
    movl %edi, %ebx             // which violation, zero-extended

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

    // ---- the capability preamble -----------------------------------------
    //
    // Run by every process, whatever it was told to provoke, and the reason is
    // the same one `user=exit` exists for: a suite made only of refusals passes
    // against a frame that refuses everything. So the first thing any process
    // does is use its capabilities correctly — inspect one, derive a copy of
    // it, and map the copy — and the frame counts all three.
    //
    // The three handles are the first three slots at the first generation,
    // because the frame fills the lowest free slot and grants in a fixed order.
    // A process is entitled to know its own starting handles; it is not
    // entitled to compute any others, which is what the sweep below tries.
.Lprobe_provoke:
    // "What is this?" — the one call that asks rather than acts.
    movl ${sys_inspect}, %eax
    movl %r12d, %edi
    xorl %esi, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived

    // A copy: the same rights, which makes it a child of the capability it came
    // from rather than a peer of it.
    movl ${sys_derive}, %eax
    movl %r12d, %edi
    movl ${right_rdv}, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived
    movq %rax, %r14

    // And the mapping the copy authorises. Read-only, because the capability
    // this was derived from carries no write right — `cap=rights` is the run
    // that tries to have it both ways.
    movl ${sys_map}, %eax
    movq %r13, %rdi
    shlq $32, %rdi
    orq %r14, %rdi
    movl ${grant_read}, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived

    // Touch it. A mapping the frame reported as made and the processor refuses
    // is the failure this line exists to turn into a fault rather than into a
    // number that looks right.
    movl ${grant_page}, %eax
    movq (%rax), %rax

    // And the fourth: the frame's state tree, mapped read-only from the handle
    // it was granted at rather than from a copy. E0-B14. The derive above
    // exists to be exercised and doing it twice would exercise it twice, while
    // making every live-handle count in `process.rs` one larger for nothing.
    //
    // The handle is two slots along from the address space: space, frame,
    // untyped, tree, in the order `process::prepare` grants them.
    movl %r13d, %r15d
    addl $3, %r15d
    movl ${sys_map}, %eax
    movq %r13, %rdi
    shlq $32, %rdi
    orq %r15, %rdi
    movl ${tree_read}, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived

    // Touch that too, for the same reason.
    movl ${tree_page}, %eax
    movq (%rax), %rax

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
    cmpq $7, %rbx
    je .Lcap_grant
    cmpq $8, %rbx
    je .Lcap_unowned
    cmpq $9, %rbx
    je .Lcap_forge
    cmpq $10, %rbx
    je .Lcap_stale
    cmpq $11, %rbx
    je .Lcap_rights
    cmpq $12, %rbx
    je .Lcap_mistyped
    cmpq $13, %rbx
    je .Lcap_flood
    cmpq $14, %rbx
    je .Lcap_unmap
    cmpq $15, %rbx
    je .Lcap_state
    jmp .Lprobe_exit

    // The state tree, which the preamble mapped read-only and read. Writing to
    // it must fault. The read happened first and it succeeded, so a fault here
    // is about the *permission* and not about the address — a provocation that
    // faulted because the page was absent would report the write protection
    // holding when nothing had tested it, which is E0-B12's forged-slot-zero
    // lesson in a new place.
.Lcap_state:
    movl ${tree_page}, %eax
    movq $0, (%rax)
    jmp .Lprobe_survived

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
    // Explicit, not a fall-through. It used to be one — `.Lprobe_exit` was the
    // next label in the file — and M4 put seven blocks in between, so `user=call`
    // silently started running the capability control as well. A jump that
    // depends on what is written underneath it is a jump that moves when
    // somebody edits above.
    jmp .Lprobe_exit

    // ---- the capability escapes -------------------------------------------
    //
    // Seven, and only the first of them is supposed to succeed. Every one of
    // the other six is refused, and the *reason* it is refused is the content
    // of the test: the frame counts refusals by code, so a run that is turned
    // down the right number of times for the wrong reasons fails.
    //
    // None of them checks its own result. The process is not the judge here —
    // it cannot be, since a compromised one would lie — so it makes the
    // attempts and the frame says what happened. That is also why these blocks
    // are short: everything interesting about them is in `process::expects_caps`.

    // Narrow, then read back what came out. The control run.
.Lcap_grant:
    movl ${sys_derive}, %eax
    movq %r14, %rdi
    movl ${right_r}, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived
    movq %rax, %r15
    movl ${sys_inspect}, %eax
    movq %r15, %rdi
    xorl %esi, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived
    jmp .Lprobe_exit

    // A slot in range, at the generation slots are issued at, that the frame
    // never filled. The handle is well-formed in every respect except the one
    // that matters.
.Lcap_unowned:
    movl ${sys_inspect}, %eax
    movl ${cap_unowned}, %edi
    xorl %esi, %esi
    syscall
    movl ${sys_map}, %eax
    movq %r13, %rdi
    shlq $32, %rdi
    orq ${cap_unowned}, %rdi
    movl ${grant2_read}, %esi
    syscall
    jmp .Lprobe_exit

    // Every slot, four generations each, then four words nobody could have
    // issued. `rbp` and `r15` carry the loop because they are two of the six
    // registers a call preserves; everything else is destroyed by `syscall`.
.Lcap_forge:
    movl $1, %ebp
.Lcap_forge_gen:
    xorl %r15d, %r15d
.Lcap_forge_slot:
    movl ${sys_inspect}, %eax
    movl %ebp, %edi
    shll $16, %edi
    orl %r15d, %edi
    xorl %esi, %esi
    syscall
    incl %r15d
    cmpl ${slots}, %r15d
    jb .Lcap_forge_slot
    incl %ebp
    cmpl ${sweep_generations}, %ebp
    jbe .Lcap_forge_gen

    // The zero word, one past the last slot, the largest index there is, and
    // all ones. The first is the one that matters most: a handle field that was
    // never written must not name slot zero.
    movl ${sys_inspect}, %eax
    xorl %edi, %edi
    xorl %esi, %esi
    syscall
    movl ${sys_inspect}, %eax
    movl ${cap_past_end}, %edi
    xorl %esi, %esi
    syscall
    movl ${sys_inspect}, %eax
    movl ${cap_last_index}, %edi
    xorl %esi, %esi
    syscall
    movl ${sys_inspect}, %eax
    movl $0xffffffff, %edi
    xorl %esi, %esi
    syscall
    jmp .Lprobe_exit

    // Three deep, then withdraw the root and keep using the leaves. A revoke
    // that stopped at the children would leave the grandchild working, which is
    // the mistake that looks like it worked.
.Lcap_stale:
    movl ${sys_derive}, %eax
    movq %r14, %rdi
    movl ${right_rdv}, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived
    movq %rax, %r15
    movl ${sys_revoke}, %eax
    movq %r12, %rdi
    xorl %esi, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived
    movl ${sys_inspect}, %eax
    movq %r14, %rdi
    xorl %esi, %esi
    syscall
    movl ${sys_map}, %eax
    movq %r13, %rdi
    shlq $32, %rdi
    orq %r15, %rdi
    movl ${grant2_read}, %esi
    syscall
    jmp .Lprobe_exit

    // Widen by derivation, then map more permissively than the capability
    // allows. Two different ways to acquire a right nobody granted.
.Lcap_rights:
    movl ${sys_derive}, %eax
    movq %r12, %rdi
    movl ${right_rwdv}, %esi
    syscall
    movl ${sys_map}, %eax
    movq %r13, %rdi
    shlq $32, %rdi
    orq %r12, %rdi
    movl ${grant2_write}, %esi
    syscall
    jmp .Lprobe_exit

    // An address space where a frame belongs, then a frame where an address
    // space belongs. Both capabilities are held; neither names what the operand
    // is for.
.Lcap_mistyped:
    movl ${sys_map}, %eax
    movq %r13, %rdi
    shlq $32, %rdi
    orq %r13, %rdi
    movl ${grant2_read}, %esi
    syscall
    movl ${sys_map}, %eax
    movq %r12, %rdi
    shlq $32, %rdi
    orq %r12, %rdi
    movl ${grant2_read}, %esi
    syscall
    jmp .Lprobe_exit

    // Derive until it stops working. The table has a bound and reaching it is
    // an error rather than an event.
.Lcap_flood:
    movl ${sys_derive}, %eax
    movq %r12, %rdi
    movl ${right_r}, %esi
    syscall
    testq %rax, %rax
    jns .Lcap_flood
    jmp .Lprobe_exit

    // The one escape the frame does not answer. Everything above is refused by
    // the capability table and the process carries on; this asks for something
    // it is entitled to — revoking a capability it holds the right to revoke —
    // and then uses a page whose authority that revoke withdrew.
    //
    // The preamble mapped the derived copy at `grant_page` and read it, six
    // instructions ago, successfully. So the fault here is not "this page was
    // never there": it is the same page, in the same address space, on the same
    // core, after the name behind it was taken back. Until E0-B10 this read
    // succeeded, and the boot log said the capability had been revoked.
.Lcap_unmap:
    movl ${sys_revoke}, %eax
    movq %r12, %rdi
    xorl %esi, %esi
    syscall
    testq %rax, %rax
    js .Lprobe_survived
    movl ${grant_page}, %eax
    movq (%rax), %rax
    jmp .Lprobe_survived

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
    // The call numbers, so that the program and the frame cannot disagree about
    // them silently. A constant repeated in assembly is a constant that goes
    // stale, and the failure would be a process calling something it did not
    // mean to.
    sys_inspect = const crate::process::SYS_CAP_INSPECT as u32,
    sys_derive = const crate::process::SYS_CAP_DERIVE as u32,
    sys_revoke = const crate::process::SYS_CAP_REVOKE as u32,
    sys_map = const crate::process::SYS_CAP_MAP as u32,

    // Handles that name nothing. In range and never filled; one past the last
    // slot; and the largest index the packing can express.
    cap_unowned = const Handle::new(UNOWNED_SLOT, Handle::FIRST_GENERATION).bits(),
    cap_past_end = const Handle::new(TABLE_SLOTS as u16, Handle::FIRST_GENERATION).bits(),
    cap_last_index = const Handle::new(u16::MAX, Handle::FIRST_GENERATION).bits(),

    slots = const TABLE_SLOTS as u32,
    sweep_generations = const crate::process::SWEEP_GENERATIONS,

    right_r = const rights::READ as u32,
    right_rdv = const (rights::READ | rights::DERIVE | rights::REVOKE) as u32,
    right_rwdv = const (rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE) as u32,

    // A page-aligned address with the rights in the twelve bits alignment
    // leaves free, which is the packing `SYS_CAP_MAP` documents.
    grant_page = const crate::process::GRANT as u32,
    grant_read = const (crate::process::GRANT as u32) | rights::READ as u32,
    grant2_read = const (crate::process::GRANT_SECOND as u32) | rights::READ as u32,
    grant2_write =
        const (crate::process::GRANT_SECOND as u32) | (rights::READ | rights::WRITE) as u32,
    tree_page = const crate::process::TREE as u32,
    tree_read = const (crate::process::TREE as u32) | rights::READ as u32,
    options(att_syntax)
);

/// A slot in range, at the generation slots are issued at, that the frame never
/// fills.
///
/// Nine: past the four the frame grants and past anything the preamble
/// derives, so it is empty on every run. The point of choosing a plausible slot
/// rather than a wild one is that this handle is well-formed in every respect
/// except the one that matters — it is what a component would produce by
/// getting its own bookkeeping wrong, rather than by attacking anything.
const UNOWNED_SLOT: u16 = 9;

const _: () = assert!((UNOWNED_SLOT as usize) < TABLE_SLOTS);
const _: () = assert!(UNOWNED_SLOT as usize > crate::process::GRANTS);
