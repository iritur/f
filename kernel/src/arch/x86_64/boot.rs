// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The boot stub: multiboot header, the walk from 32-bit protected mode to
//! 64-bit long mode, and the move to the higher half.
//!
//! # Why multiboot 1 and not Limine or multiboot 2
//!
//! QEMU implements multiboot 1 in its own `-kernel` loader, so this handoff
//! costs nothing but the header and this stub: no vendored bootloader binary,
//! no ISO assembly step, no third-party licence boundary to draw at M0, and
//! `cargo xtask run` stays one command. Multiboot 2 would mean GRUB and an
//! ISO; Limine would mean a binary in the tree.
//!
//! It buys what M1 needs — a memory map — and one thing more, which arrived
//! later and is worth separating from the protocol it is usually blamed on.
//! **Multiboot 1 does define a video request**, at flag bit 2 and four fields
//! in the header, and a loader that implements it hands back a linear
//! framebuffer's address, pitch, geometry and channel layout. What does not
//! implement it is QEMU's own `-kernel` loader, which prints *multiboot knows
//! VBE. we don't.* and carries on. So the sentence that used to be here —
//! *this protocol does not give a framebuffer* — was true of the emulator and
//! false of the protocol, and the difference is the whole of `intent/0011`
//! step 1: the request is made unconditionally, both loaders answer, and
//! [`crate::arch::x86_64::multiboot`] reports which one answered with
//! something. It is still a BIOS-era protocol and still cannot carry the ACPI
//! root pointer under UEFI, which is the reason E5-D03 exists and which this
//! header does not touch.
//!
//! Nothing above [`crate::kmain`] can tell the difference: the handoff is one
//! pointer wide.
//!
//! # Two halves at two addresses
//!
//! This file is linked where it is *loaded* — low, at 1 MiB — because it runs
//! before there is a mapping, so a symbol in it has to be a physical address.
//! The rest of the kernel is linked high and loaded low, and the last thing the
//! stub does is jump there. Everything after that point runs at
//! `KERNEL_VMA + physical`, which is where user space stops being able to reach
//! it.
//!
//! # What the stub does, in order
//!
//! 1. Save the two registers the protocol hands over, `eax` and `ebx`.
//! 2. Zero its own `.bss.boot`, which is where the transitional tables live.
//! 3. Map the first gigabyte twice: identity, so the low code it is currently
//!    executing keeps working, and again at `KERNEL_VMA`, which is where it is
//!    going.
//! 4. Enable PAE, set the long-mode enable bit, turn paging on.
//! 5. Load a flat 64-bit descriptor table and far jump into 64-bit code.
//! 6. Jump to the high half, reload the descriptor table at its high address,
//!    take a high stack, zero the high `.bss`, and call [`crate::kmain`].
//!
//! Everything the stub builds is transitional. The kernel replaces all of it in
//! [`super::paging`] with tables built from real frames, and the identity half
//! disappears at that point — which is the whole reason step 6 exists, because
//! a kernel still executing at a low address could not survive its own page
//! tables being replaced.
//!
//! The stub is `global_asm!` rather than a `#[naked]` function because it is one
//! contiguous piece of position-dependent code with its own data, and splitting
//! it across Rust items would buy nothing but a chance to get the calling
//! convention wrong. It is written in AT&T syntax: a far jump with an immediate
//! selector is unambiguous there, and this is the one place in the tree that
//! needs one.

use core::arch::global_asm;

global_asm!(
    r#"
    // ---------------------------------------------------------------- header
    .section .multiboot, "a", @progbits
    .align 4
    .long 0x1BADB002                        // magic
    .long 0x00000007                        // flags: page-align modules, memory info, video mode
    .long -(0x1BADB002 + 0x00000007)        // checksum: the three must sum to zero

    // Offsets 12..31: the address fields, which belong to flag bit 16 and are
    // not requested. They are emitted as zeroes because the video request below
    // sits at *fixed* offsets 32..47 — a header that skipped these five words
    // would put `mode_type` where `header_addr` belongs, and a loader that read
    // it would set a video mode from an address. Padding the protocol requires,
    // rather than an address plan.
    .long 0, 0, 0, 0, 0

    // Offsets 32..47: the video request. Every field of this was zero at first
    // — which the protocol reads as *you choose* — and the answer a real
    // machine gave was 1024 by 768, because that is the mode a firmware is
    // already sitting in when it hands over. That is the right request for
    // finding out whether there is an answer at all, and the wrong one for
    // reading a boot log: at eight by sixteen it is 128 columns by 48 rows, and
    // this kernel prints lines longer than 128 columns.
    //
    // So a size is named, and naming one is safe for a reason worth writing
    // down rather than trusting. GRUB turns this request into the mode string
    // `1920x1080x32,1920x1080,auto` — its own construction, not this header's —
    // and tries those in order. A firmware that cannot do 1920 by 1080 at
    // thirty-two bits falls to that size at any depth, and then to `auto`,
    // which is precisely the mode the all-zero request would have produced.
    // **No arrangement of this header ends with no framebuffer where the empty
    // one would have got one**, which is the only property that matters here:
    // the failure to avoid is a machine that boots to a black screen because
    // its kernel was ambitious about pixels.
    //
    // What it costs is the choice. GRUB writes this request into `gfxpayload`
    // itself, so `GRUB_GFXMODE` no longer decides what the payload gets. A
    // machine that wants a different mode changes the two numbers below, and
    // `docs/booting-on-hardware.md` says so beside the menu entry it documents.
    // Zeroing them restores the firmware's own answer and hands the choice back
    // to the GRUB configuration.
    //
    // 1080p rather than something larger: it is the mode almost every panel and
    // every virtual machine of this era can set, and at eight by sixteen it is
    // 240 columns by 67 rows — a full terminal, and the same grid the same
    // panel shows under Linux. A 4K request would name a mode many firmwares do
    // not offer before an operating system has loaded a driver for the display,
    // so it would fall back more often than it would land.
    .long 0                                 // mode_type: 0 linear graphics, 1 EGA text
    .long 1920                              // width,  in pixels
    .long 1080                              // height, in pixels
    .long 32                                // depth,  in bits per pixel

    // ------------------------------------------------------- low data + code
    // Linked where it is loaded, so every symbol here is a physical address.
    .section .data.boot, "aw", @progbits
    .align 4
boot_magic:
    .long 0
boot_info:
    .long 0

    .align 8
boot_gdt:
    .quad 0                                 // null descriptor
    .quad 0x00AF9A000000FFFF                // 64-bit code: present, exec/read, L=1
    .quad 0x00CF92000000FFFF                // data: present, read/write
boot_gdt_end:
    // Two pointers to one table. The 32-bit form carries a 32-bit base and is
    // loaded while the identity mapping is live; the 64-bit form carries the
    // same table's high address and is loaded once the kernel is running there,
    // because a descriptor table reachable only through a mapping that is about
    // to be deleted is a fault waiting for the first interrupt.
boot_gdt_ptr:
    .word boot_gdt_end - boot_gdt - 1
    .long boot_gdt

    .section .bss.boot, "aw", @nobits
    .align 4096
boot_pml4:
    .skip 4096
boot_pdpt:
    .skip 4096
boot_pdpt_high:
    .skip 4096
boot_pd:
    .skip 4096
    .align 16
boot_stack:
    .skip 4096
boot_stack_top:

    .section .text.boot, "ax", @progbits
    .code32
    .globl _start
    .type _start, @function
_start:
    cli
    cld

    // The protocol's whole payload: a magic number saying who loaded us and a
    // pointer to everything else. Stored before anything is clobbered.
    movl %eax, boot_magic
    movl %ebx, boot_info

    // Zero this section's own bss. The high one is zeroed later, from the high
    // half, where its addresses are the ones that will still work.
    movl $__bootbss_start, %edi
    movl $__bootbss_end, %ecx
    subl %edi, %ecx
    shrl $2, %ecx
    xorl %eax, %eax
    rep stosl

    movl $boot_stack_top, %esp

    // PML4[0] -> PDPT, and PDPT[0] -> PD: the identity window, which is what
    // this code is executing out of and cannot do without yet.
    movl $boot_pdpt, %eax
    orl $0x3, %eax
    movl %eax, boot_pml4

    movl $boot_pd, %eax
    orl $0x3, %eax
    movl %eax, boot_pdpt

    // PML4[511] -> high PDPT, and high PDPT[510] -> the same PD: the
    // -2 GiB window, pointing at the same first gigabyte of physical memory.
    // Two virtual addresses, one set of pages, which is what makes the jump
    // between them survivable.
    movl $boot_pdpt_high, %eax
    orl $0x3, %eax
    movl %eax, boot_pml4 + 511 * 8

    movl $boot_pd, %eax
    orl $0x3, %eax
    movl %eax, boot_pdpt_high + 510 * 8

    // PD[i] = i * 2 MiB, present | writable | huge. 512 entries: one gigabyte.
    xorl %ecx, %ecx
    xorl %eax, %eax
1:
    movl %eax, %edx
    orl $0x83, %edx
    movl %edx, boot_pd(, %ecx, 8)
    movl $0, boot_pd + 4(, %ecx, 8)
    addl $0x200000, %eax
    incl %ecx
    cmpl $512, %ecx
    jb 1b

    movl $boot_pml4, %eax
    movl %eax, %cr3

    // CR4.PAE — long mode requires it, and it must precede setting LME.
    movl %cr4, %eax
    orl $0x20, %eax
    movl %eax, %cr4

    // EFER.LME, in the extended feature MSR.
    movl $0xC0000080, %ecx
    rdmsr
    orl $0x100, %eax
    wrmsr

    // CR0.PG. The processor is in compatibility mode from here until the far
    // jump loads a descriptor with the long-mode bit set.
    movl %cr0, %eax
    orl $0x80000000, %eax
    movl %eax, %cr0

    lgdt boot_gdt_ptr
    ljmp $0x08, $2f

    // Still executing low, now in 64-bit mode. The only job left down here is
    // to leave: an absolute jump, because the distance to the high half is far
    // past what a relative one can express.
    .code64
2:
    movw $0x10, %ax
    movw %ax, %ds
    movw %ax, %es
    movw %ax, %ss
    movw %ax, %fs
    movw %ax, %gs

    movabs $3f, %rax
    jmp *%rax

    // ------------------------------------------------------------- high half
    // Linked high, loaded low, and reached only through the -2 GiB window.
    .section .text, "ax", @progbits
    .code64
3:
    // A stack in the high half, before anything that could need one. The
    // linker script places it with an unmapped page below it, so an overflow
    // faults on the guard rather than quietly eating whatever is underneath.
    movabs $__kernel_stack_top, %rsp
    xorq %rbp, %rbp

    // The descriptor table again, at an address that survives the identity
    // window being deleted.
    movabs $boot_gdt_ptr64, %rax
    lgdt (%rax)

    // Zero the high .bss. Nothing has been put there yet, and the loader does
    // not promise to have done it.
    movabs $__bss_start, %rdi
    movabs $__bss_end, %rcx
    subq %rdi, %rcx
    shrq $3, %rcx
    xorq %rax, %rax
    rep stosq

    // The handoff, read back from where the 32-bit half left it. Still reached
    // through the identity window, which is still live.
    movabs $boot_magic, %rax
    movl (%rax), %edi
    movabs $boot_info, %rax
    movl (%rax), %esi

    call kmain

    // kmain is `-> !`. If it ever returns, that is a bug in the frame and the
    // only honest response is to stop rather than execute whatever follows.
4:
    cli
    hlt
    jmp 4b

    // ------------------------------------------------------------- high data
    .section .rodata, "a", @progbits
    .align 8
boot_gdt_ptr64:
    .word boot_gdt_end - boot_gdt - 1
    .quad boot_gdt

"#,
    options(att_syntax)
);

/// The value a multiboot 1 loader leaves in `eax`.
///
/// Anything else means the kernel was entered by something that does not speak
/// this protocol, and every other register — including the info pointer — is
/// then meaningless rather than merely wrong.
pub const MULTIBOOT_MAGIC: u32 = 0x2BAD_B002;
