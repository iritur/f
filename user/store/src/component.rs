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
//! # What it does, and which of its two lives it is in
//!
//! Entered with a selector of zero it announces itself and ends, which is what
//! `E1-B05` spawns, kills and respawns on every boot: the interesting half of
//! that life happens *to* it, and what a client sees across a restart is the
//! thing gate G1 is about.
//!
//! Entered with [`report::RUN`] it is a **runtime**. It adopts its control ring
//! and its own work ring in safe code — `f_ring::adopt`, RFC 0037 — schedules
//! [`report::LOAD`] work items inside the core it was allocated, drains its
//! control ring at every allocation boundary, and parks cleanly when the frame
//! posts a reclaim notice. Between the instruction that enters it and the
//! `EXIT` that leaves it, it crosses no privilege boundary at all, and
//! `kernel/src/runtime.rs` is what counts that rather than asserting it.
//!
//! The sentence this module used to carry — *draining one means adopting a
//! mapped channel, which is `unsafe` and which this crate may not write* — was
//! true until `E1-B08`. `crate::runtime` is where it stopped being true.

use f_abi::door;

use crate::report;

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
    // Which of this component's two lives the frame asked for. A selector the
    // frame does not set is zero, which is the life this component has always
    // had; `report::RUN` and `report::PROVOKE` are the runtime, and a selector
    // this build does not name falls through to the old life rather than
    // inventing a third.
    //
    // Two lives in one image rather than two component files, because they are
    // one component: the place, the manifest, the account and the restart
    // policy are all the same, and what differs is whether the frame gave it a
    // core to schedule inside. A second manifest would be a second place, and a
    // second place is a claim about the topology rather than about scheduling.
    let selector = door::Entry::from_bits(argument).selector();
    if selector == report::RUN || selector == report::PROVOKE {
        crate::runtime::run(selector);
    }

    // The frame tells a component what it holds rather than letting it assume,
    // and `door::Entry` argues why: a second occupant of a place finds its
    // capabilities at the same indices and a later generation, so a component
    // that wrote the handles down would be refused for a reason that looks
    // nothing like the mistake. Read here and unused, because there is nothing
    // yet for this component to use them *for* — and read rather than ignored,
    // so that the day there is, the reading is already in the right place.
    let entry = door::Entry::from_bits(argument);
    let _ = entry.granted(0);

    // An allocation, which is the first one any component in this tree has made.
    //
    // It is here rather than in the runtime because this is the life the frame
    // spawns into a place, and a place is what carries the `heap` need the frame
    // maps. What it proves is not that a vector works: it is that a
    // `#[global_allocator]` in a crate that forbids `unsafe`, over a region the
    // frame granted and described before this instruction, hands out memory that
    // is there. The frame reads the region's own prologue afterwards and reports
    // what was taken, so the evidence is a number on the other side of the
    // boundary rather than this component's word for it.
    //
    // Dropped immediately, so `live` returns to zero and `peak` does not — which
    // is the pair that says the allocation happened *and* came back.
    // The *address* is what is black-boxed, and that is not incidental: Rust is
    // allowed to elide an allocation whose value never escapes, so a box that is
    // only read from is a box that may never have been allocated. Observing
    // where it landed is what makes the allocation something the optimiser has
    // to perform.
    #[cfg(all(target_os = "none", feature = "image"))]
    {
        let taken = alloc::boxed::Box::new([7u8; 64]);
        let at = core::ptr::from_ref::<[u8; 64]>(taken.as_ref()) as u64;
        core::hint::black_box(at);
        drop(taken);
    }

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
