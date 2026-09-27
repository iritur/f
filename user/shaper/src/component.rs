// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The shaper's entry, and the honest statement of how little it does.
//!
//! # Why an entry exists at all
//!
//! RFC 0082 says the shaper is *built into a component image of its own*, and an
//! image is linked by `user/init/link.ld`, which places the section this
//! function is compiled into at the first byte. Without an entry there is no
//! image to link, and without an image there is nothing to measure — and the
//! measurement is the finding: `cargo xtask shaper` links this crate the way every
//! component is linked and reports how large the result is against what the
//! frame maps for a spawned component.
//!
//! # Why it serves nothing
//!
//! Because no frame in this tree can spawn it. `kernel::process` maps sixteen
//! pages of text for a spawned component and copies a flat image into them;
//! the shim and the import it reaches are several times that. `UNSPAWNABLE` in
//! `xtask/src/main.rs` holds the reason as a checked row, with the build that
//! would reverse it: a loader that reads an image's headers (`E5`), or a shaper
//! that fits. A ring loop written here today would be code no boot can run, so
//! it is not written. `crate::serve` is what the loop would call, and what runs
//! of it is a host test: `user/shaper/tests/run.rs`, in one process, on x86-64
//! Linux and — once CI's AArch64 job has run it — on AArch64 Linux. This file is
//! compiled and linked into the image `cargo xtask test` measures, and no frame
//! has executed it.
//!
//! So the entry does two things. It takes [`crate::serve`]'s address through an
//! opaque barrier, which is what keeps `--gc-sections` from discarding the
//! import — the image then carries what a serving shaper carries, and the
//! measurement is of that and not of an empty function. And it exits with
//! [`NOT_SERVING`], so that a spawn somebody forced would end with a word saying
//! why rather than park.
//!
//! # Nothing here parks
//!
//! A panic ends in `ud2` — an invalid-opcode fault at ring 3, which the frame
//! records with a vector, an address and an instruction pointer — and not in a
//! spin and not in `door::EXIT`. Not a spin, because a component that parks
//! tells nobody anything, and the heap refusing an allocation is a panic:
//! a face that made the import ask for more than the manifest's `heap` would
//! otherwise hold its core forever. Not `EXIT`, because `docs/manifest.md`'s
//! `on_fault`, which this component's `[restart]` says, respawns after a fault
//! and *not* after an exit; an exit would leave the place empty with nobody
//! told why. `ud2` is also what every other component's panic already is — they
//! are built with `panic=immediate-abort`, which this one cannot be
//! (`xtask/src/shaper.rs` says why), so it writes the same instruction by hand.
//! No host test can run a handler compiled for the image only, so
//! `tests/run.rs` holds this file's text to it; in the linked image
//! `rust_begin_unwind` is the one instruction `ud2` (RFC 0141).

use f_abi::door;

/// The status this entry exits with: *built, and not serving*.
/// Unit: none — an exit status.
pub const NOT_SERVING: u64 = 1;

/// The entry. `user/init/link.ld` places it at the image's first byte.
pub fn start(_argument: u64) -> ! {
    core::hint::black_box(
        crate::serve as fn(&[[u8; 32]], &[u8; f_abi::shape::Request::BYTES], &[u8], &mut [u8]) -> _,
    );
    let _ = door::call(door::EXIT, NOT_SERVING, 0);
    // `EXIT` does not come back. If a frame ever let it, a fault says so.
    core::intrinsics::abort()
}

/// A panic, a refused allocation among them: a fault, for the module's reason.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    core::intrinsics::abort()
}
