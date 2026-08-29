/* SPDX-License-Identifier: Apache-2.0 OR MIT
 *
 * The kernel image, in two halves that live at different addresses.
 *
 * `.boot` is linked and loaded at the same physical address, because it runs
 * before paging exists and a symbol in it must be a physical address. Every
 * other section is linked high and loaded low: the loader places it at 1 MiB,
 * and it runs at KERNEL_VMA + 1 MiB once the boot stub has built a mapping that
 * says so. `AT()` is what expresses that difference to the loader.
 *
 * Why higher-half at all: the lower half of the address space belongs to user
 * space from M3 onward. A kernel that lives down there has to move before the
 * first user process exists, and moving it later means moving it while
 * something depends on where it is.
 *
 * -2 GiB specifically, because it is within reach of the kernel code model:
 * `rip`-relative addressing and 32-bit sign-extended displacements both work,
 * and no relocation in the image needs a full 64-bit absolute.
 *
 * # The boundaries are exported, and they are load-bearing
 *
 * Every `__*_start` / `__*_end` pair below is read by `arch::x86_64::paging`,
 * which maps each range with the permissions its contents deserve: text is
 * executable and not writable, constants are neither, and everything else is
 * writable and not executable. That is only expressible because these
 * boundaries are page-aligned — a section that shares a page with another gets
 * the union of their permissions, which is the permission nobody wanted.
 */

ENTRY(_start)

KERNEL_VMA = 0xFFFFFFFF80000000;

/* One page, unmapped, below each stack. A stack that grows past its end hits
 * nothing instead of hitting whatever was allocated underneath it. */
GUARD_SIZE  = 4K;
STACK_SIZE  = 64K;
FAULT_STACK_SIZE = 16K;

SECTIONS {
    /* Below 1 MiB is firmware, the legacy device hole and the real-mode
     * interrupt table. No loader will place an image there. */
    . = 1M;
    __kernel_phys_start = .;

    /* Runs with paging off, or with the boot stub's own identity map. Linked
     * where it is loaded, so `$symbol` in the stub is a physical address and
     * needs no arithmetic. The multiboot header must be within the first 8 KiB
     * of the file, and KEEP() is what stops the linker discarding a section
     * nothing references.
     *
     * This half is deliberately *absent* from the address space the kernel
     * builds for itself: nothing here runs again after the jump to the higher
     * half, so nothing here should remain reachable. */
    .boot : ALIGN(4) {
        KEEP(*(.multiboot))
        *(.text.boot)
        *(.data.boot)
    }
    .bootbss (NOLOAD) : ALIGN(4096) {
        __bootbss_start = .;
        *(.bss.boot)
        . = ALIGN(4);
        __bootbss_end = .;
    }

    . = ALIGN(4096);
    __kernel_high_phys = .;

    /* Everything from here is linked high and loaded low. */
    . = . + KERNEL_VMA;
    __kernel_start = .;

    .text ALIGN(4096) : AT(ADDR(.text) - KERNEL_VMA) {
        __text_start = .;
        *(.text .text.*)
        . = ALIGN(4096);
        __text_end = .;
    }

    .rodata ALIGN(4096) : AT(ADDR(.rodata) - KERNEL_VMA) {
        __rodata_start = .;
        *(.rodata .rodata.*)
        . = ALIGN(4096);
        __rodata_end = .;
    }

    /* Writable, and never executable. `.got` is named explicitly because the
     * linker places a section nobody mentions wherever it likes — and wherever
     * it likes turned out to be next to `.boot`, at a low address, where an
     * indirect call through it faulted the instant the identity window was
     * dropped. */
    .data ALIGN(4096) : AT(ADDR(.data) - KERNEL_VMA) {
        __rwdata_start = .;
        *(.data .data.*)
        *(.data.rel.ro .data.rel.ro.*)
    }

    .got ALIGN(8) : AT(ADDR(.got) - KERNEL_VMA) {
        *(.got)
        *(.got.plt)
        *(.igot .igot.plt)
    }

    /* Zeroed by the boot stub after it reaches the high half, not by the
     * loader: multiboot does not promise to do it for an ELF image. */
    .bss ALIGN(4096) (NOLOAD) : AT(ADDR(.bss) - KERNEL_VMA) {
        __bss_start = .;
        *(COMMON)
        *(.bss .bss.*)
        . = ALIGN(8);
        __bss_end = .;
    }

    /* Stacks, each with an unmapped page below it.
     *
     * They live in their own section rather than in `.bss` for one reason: a
     * guard page has to be somewhere the mapper can skip, and skipping a page
     * in the middle of `.bss` would mean skipping whatever else happened to
     * share it. Before this section existed, a stack overflow grew down through
     * `.bss` into the descriptor tables — corrupting the machinery that would
     * have reported the overflow, and turning a diagnosable fault into a silent
     * reset. */
    .stacks ALIGN(4096) (NOLOAD) : AT(ADDR(.stacks) - KERNEL_VMA) {
        __fault_stack_guard = .;
        . += GUARD_SIZE;
        __fault_stack_bottom = .;
        . += FAULT_STACK_SIZE;
        __fault_stack_top = .;

        __kernel_stack_guard = .;
        . += GUARD_SIZE;
        __kernel_stack_bottom = .;
        . += STACK_SIZE;
        __kernel_stack_top = .;
    }

    . = ALIGN(4096);
    __rwdata_end = .;
    __kernel_end = .;

    /* The physical extent, so the frame allocator can reserve the memory the
     * kernel is running out of without doing address arithmetic in Rust. */
    __kernel_phys_end = . - KERNEL_VMA;

    /* Exported so that one number defines the split, in one file. */
    __kernel_vma = KERNEL_VMA;

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note.GNU-stack)
        *(.comment)
    }
}
