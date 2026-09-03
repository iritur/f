// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `store` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to.
//!
//! # Why there is no attribute on [`start`]
//!
//! Because there cannot be, and `user/init/src/component.rs` records the scar at
//! length: naming an entry point means `#[unsafe(no_mangle)]` or
//! `#[unsafe(link_section)]`, both of which are unsafe attributes in this
//! edition, and a crate that forbids unsafe code cannot write one. So
//! `user/init/link.ld` places the section this function is compiled into at the
//! image's first byte, and `cargo xtask component` checks that the symbol which
//! actually landed there is this one. The path `component::start` is therefore
//! load-bearing across both crates: one linker script, one placement rule, one
//! check.
//!
//! # What it does
//!
//! It announces itself and ends, and the shortness is the honest part rather
//! than a placeholder. A component's whole vocabulary is its control ring, and
//! draining one means adopting a mapped channel, which is `unsafe` and which
//! this crate may not write — E1-B08 and RFC 0030. Until that lands, the
//! interesting half of this component's life happens *to* it: it is spawned
//! from a record, killed, and spawned again into the same place, and what a
//! client sees across that is the thing gate G1 is about.

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
pub fn start(argument: u64) -> ! {
    // The frame tells a component what it holds rather than letting it assume,
    // and `door::Entry` argues why: a second occupant of a place finds its
    // capabilities at the same indices and a later generation, so a component
    // that wrote the handles down would be refused for a reason that looks
    // nothing like the mistake. Read here and unused, because there is nothing
    // yet for this component to use them *for* — and read rather than ignored,
    // so that the day there is, the reading is already in the right place.
    let entry = door::Entry::from_bits(argument);
    let _ = entry.granted(0);

    // "I am here." The one thing the frame cannot observe from outside.
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
/// no serial port. Stopping is the whole handler, and the frame notices the same
/// way it notices anything else — the component stops making progress and its
/// supervisor's stop deadline passes.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
