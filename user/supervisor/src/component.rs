// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `supervisor` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to.
//!
//! # Why there is no attribute on [`start`]
//!
//! Because there cannot be, and `user/init/src/component.rs` records the scar at
//! length: naming an entry point means `#[unsafe(no_mangle)]` or
//! `#[unsafe(link_section)]`, both unsafe attributes in this edition, and a
//! crate that forbids unsafe code cannot write one. `user/init/link.ld` places
//! the section this function compiles into at the image's first byte, and
//! `cargo xtask component` checks that the symbol which actually landed there is
//! this one. The path `component::start` is load-bearing across every component
//! crate: one linker script, one placement rule, one check.
//!
//! # What this does, and the honest size of it
//!
//! It announces itself and ends. That is the whole body, and it is the same
//! body `user/store` had before it acquired a second life, because **this
//! component has never executed and cannot until an occupant is handed a core**
//! — `kernel/src/runtime.rs` is where the tree says so and `HEAP_GAP` is where
//! the build says it.
//!
//! What is deliberately *not* written here is a loop that adopts a control ring
//! and submits `control::op::SPAWN`. Writing it would be writing against a
//! machine state no boot reaches, and this tree has a name for code like that:
//! a promised layer with no owner. The frame answers those opcodes now
//! (RFC 0073), the ring exists, and what is missing is the core — so the next
//! increment is the join, and the submission loop lands in the same diff that
//! first makes it run.

use f_abi::door;

/// A run that did what it meant to.
pub const DONE: u64 = 0;

/// Where the frame starts this component.
///
/// The image is flat and the frame jumps to its first byte, so this has to be
/// the first thing in `.text`. The module comment says how that is arranged and
/// how it is checked.
///
/// It never returns: [`door::EXIT`] does not come back, and the loop after it is
/// what happens if the frame ever lets it.
pub fn start(_argument: u64) -> ! {
    // The argument is a selector, and this component has one life rather than
    // `user/store`'s two. It is taken and ignored rather than absent from the
    // signature, because the frame's spawn passes one to every component and a
    // parameter that disappears is a protocol two sides can stop agreeing about
    // without either of them changing.

    // The heap this component's manifest declares, used rather than only asked
    // for. A supervisor decides, and deciding needs somewhere to put a decision;
    // this is the smallest honest version of that, and it is what makes the
    // `heap` need a claim the boot can check instead of a line in a file.
    //
    // What it proves is not that a box works. It is that a `#[global_allocator]`
    // in a crate that forbids `unsafe`, over a region the frame granted and
    // described before this instruction, hands out memory that is there — and
    // the frame reads the region's own prologue afterwards, so the evidence is a
    // number on the other side of the boundary rather than this component's word
    // for it.
    //
    // Dropped immediately, so `live` returns to zero and `peak` does not, which
    // is the pair that says the allocation happened *and* came back. The
    // *address* is black-boxed and that is not incidental: Rust may elide an
    // allocation whose value never escapes, so a box that is only read from is a
    // box that may never have been allocated. Observing where it landed is what
    // makes the allocation something the optimiser has to perform.
    #[cfg(all(target_os = "none", feature = "image"))]
    {
        let taken = alloc::boxed::Box::new([9u8; 64]);
        let at = core::ptr::from_ref::<[u8; 64]>(taken.as_ref()) as u64;
        core::hint::black_box(at);
        drop(taken);
    }

    // "I am here." The one thing the frame cannot observe from outside, and the
    // only claim this component is currently in a position to make.
    let _ = door::call0(door::ANNOUNCE);

    end(DONE)
}

/// End, and do not come back.
fn end(status: u64) -> ! {
    let _ = door::call(door::EXIT, status, 0);
    // `EXIT` does not return. If it ever did, the frame would have a component
    // it believes is over and a core still inside it, so the only honest thing
    // left is to stop moving.
    park()
}

/// Stop, without ending. Reached only where continuing would be worse.
fn park() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// What happens if this component panics, which nothing in it can do.
///
/// There is no formatting, no unwinding and nothing to print to: a component has
/// no serial port. Stopping is the whole handler, and the frame notices the way
/// it notices anything else — the component stops making progress and its stop
/// deadline passes.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
