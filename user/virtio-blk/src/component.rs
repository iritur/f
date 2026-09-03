// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `virtio-blk` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to.
//!
//! # Why there is no attribute on [`start`]
//!
//! Because there cannot be, and `user/init/src/component.rs` records the scar
//! at length: naming an entry point means `#[unsafe(no_mangle)]` or
//! `#[unsafe(link_section)]`, both of which are unsafe attributes in this
//! edition, and a crate that forbids unsafe code cannot write one. So
//! `user/init/link.ld` places the section this function is compiled into at the
//! image's first byte, and `cargo xtask component` checks that the symbol which
//! actually landed there is this one. The path `component::start` is
//! load-bearing across three crates now: one linker script, one placement rule,
//! one check.
//!
//! # Why this is three lines and the driver is two thousand
//!
//! Because the two halves of this component run in different places today, and
//! saying so here is better than a reader discovering it.
//!
//! The image is what a *spawn* produces: a component file, named by a hash over
//! its record and these bytes, put in a place, given the needs its manifest
//! declares. What it cannot yet do is serve, because serving means draining a
//! ring, draining a ring means adopting a mapped channel, and
//! `f_ring::Mapping::adopt` is `unsafe`. That is E1-B08's wall and RFC 0033
//! deliberately does not climb it — a channel is shared with a hostile peer, a
//! device window is not, and one argument for both would be the wrong argument
//! made twice.
//!
//! So the driver's own code — [`crate::driver`], [`crate::transport`],
//! [`crate::queue`] — is called by the frame at boot, from `kernel/src/blk.rs`,
//! against real registers and a real device. One body of code, called from the
//! wrong side of the boundary for one milestone, rather than a driver here and
//! a copy of it in the kernel. `--gc-sections` is why none of it is in this
//! image: nothing [`start`] reaches pulls it in.
//!
//! *Reversal, and it is a date rather than a measurement:* E1-B08 lands a safe
//! channel adoption for components, at which point this function grows a
//! polling loop over its control ring and its data ring, and the call from
//! `kernel/src/blk.rs` goes.

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
    // capabilities at the same indices and a later generation. For this
    // component the order is the manifest's — the four register frames, the
    // untyped region for its queues, its interrupt, its powerbox endpoint — and
    // reading the first handle here is where a serving loop would start.
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

/// What happens if this component panics.
///
/// There is no formatting, no unwinding and nothing to print to: a component
/// has no serial port. Stopping is the whole handler, and the frame notices the
/// same way it notices anything else — the component stops making progress and
/// its supervisor's stop deadline passes. Its manifest then restarts it, which
/// is what `restart.policy = "on_fault"` is for.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
