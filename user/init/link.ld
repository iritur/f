/* SPDX-License-Identifier: Apache-2.0 OR MIT
 *
 * The `init` image: one flat, read-only, executable blob.
 *
 * # Why a linker script rather than an attribute
 *
 * Because the crate this links cannot write one. Naming an entry point in Rust
 * means `#[unsafe(no_mangle)]` or `#[unsafe(link_section)]`, both of which are
 * unsafe attributes in this edition, and `user/init` forbids unsafe code — the
 * lint does not distinguish an attribute whose hazard is a duplicate symbol
 * from a dereference of a wild pointer. `src/component.rs` says more.
 *
 * So the placement happens here. `component::start` is compiled into a section
 * of its own — one section per function is the default for this target — and
 * the pattern below is what puts that section at the image's first byte.
 * `KEEP()` is what stops `--gc-sections` discarding it: nothing in this image
 * refers to it, because the thing that calls it is the kernel.
 *
 * The pattern matches both mangling schemes on purpose. The legacy name is
 * `_ZN6f_init9component5start17h<hash>E` and the v0 name is
 * `_RNvNtCs<hash>_6f_init9component5start`; both contain `9component5start`,
 * which is the path this file is really naming. `cargo xtask init` checks that
 * the symbol which actually landed at the first byte is that one, so a
 * toolchain that changes the encoding breaks the build with a sentence rather
 * than producing an image that starts in the middle of something.
 *
 * # Why the address is fixed
 *
 * Because `kernel::process::TEXT` is. A component's text goes at four mebibytes
 * up and the frame maps exactly one page there; both numbers are the frame's
 * and this file has to agree with them. When a component can pay for its own
 * address space — E1 — this becomes a position-independent image and the
 * agreement stops being needed.
 */

ENTRY(__image_start)

SECTIONS {
    /* kernel::process::TEXT. Stated as a number because the two trees are
     * linked separately and there is nothing to share a constant through. */
    . = 0x00400000;
    __image_start = .;

    .text : {
        KEEP(*(.text.*9component5start*))
        *(.text .text.*)
    }

    /* Read-only data lands in the same page as the text and is therefore
     * mapped executable. That is not ideal and it is bounded: the image is one
     * page, the frame maps it read-only, and write-exclusive-or-execute holds.
     * Splitting them would need a second page and a second mapping, which is
     * what a real loader does — E5. */
    .rodata : { *(.rodata .rodata.*) }

    /* Kept rather than discarded, and empty rather than absent. A component
     * with writable data would have it mapped read-only and would fault on the
     * first write; `cargo xtask init` refuses to build an image where either of
     * these has any content, so the failure is a sentence at build time instead
     * of a page fault at boot. */
    .data : { *(.data .data.*) *(.data.rel.ro .data.rel.ro*) }
    .got  : { *(.got) *(.got.plt) }
    .bss (NOLOAD) : { *(.bss .bss.*) *(COMMON) }

    __image_end = .;

    /DISCARD/ : {
        *(.eh_frame)
        *(.eh_frame_hdr)
        *(.note*)
        *(.comment)
        *(.debug*)
    }
}
