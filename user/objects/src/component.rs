// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `objects` as something that runs: the body a spawn puts at
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
//! actually landed there is this one. The path `component::start` is load-bearing
//! across every component in this tree: one linker script, one placement rule,
//! one check.
//!
//! # What this component is for, which is one thing
//!
//! It is the read path in a place. `E2-B08`'s exit is a **count taken across the
//! objects ring**, and `intent/0006-state/spec.md` defines an application byte as
//! one a client submitted on that ring — so until this crate is something a
//! client can submit to, every number it produces is taken at a write path or a
//! modelled device and is honestly recorded as such. `claims/0017` and
//! `claims/0022` are `pending` for exactly that reason, and so is the second
//! clause of `E2-B01`.
//!
//! # What it does not do yet, said before anybody infers it from the code
//!
//! **This body serves nothing.** `f_abi::objects::op::known` admits
//! `op::READ` now and [`crate::service`] is the body behind it — so the opcode
//! is answered, and what is missing has moved one layer out. A place's occupant
//! is given a control ring and no data ring (`kernel/src/component.rs`), so
//! there is no channel for [`start`] to adopt and no client on the other end of
//! one. The loop that would close it is four lines and is written out in
//! [`crate::service`]'s own comment; what it needs is a peer that describes a
//! channel, which for `user/virtio-blk` and `user/store` is a module in the
//! frame written for each of them.
//!
//! So what this component demonstrates is narrower than what it is for: it
//! **builds, fits, spawns, allocates and ends**. That was unavailable while the
//! heap was a plan, and `crate`'s own module comment argued at length that it
//! would stay unavailable. It is not.
//!
//! **It is narrower still than it first looks, and the number says so.** This
//! image is 1848 bytes — *smaller* than `user/store`'s 5408, which links four
//! fewer crates. The reason is `--gc-sections` in `user/init/link.ld`: nothing in
//! `f-blob`, `f-zone`, `f-index` or `f-hash` is reachable from [`start`], so the
//! linker discards all of it. The first draft of this file claimed the image
//! proved that dependency graph fits in what the frame maps for a component. It
//! proves no such thing, and the measurement is what said so. Whether the read
//! path fits in `kernel::process::INIT_TEXT_PAGES` is an open question, and it is
//! answered by the diff that makes an opcode reachable rather than by this one.
//!
//! **Still 1848 bytes on the diff that answered `READ`, and the two words that
//! sentence turns on are not the same word.** *Answered* is
//! `f_abi::objects::op::known` admitting the opcode and [`crate::service`]
//! having a body for it; *reachable* is a call to that body from [`start`],
//! which is the only root `--gc-sections` keeps anything from. There is no such
//! call, because [`start`] has no channel to take an entry off and no device
//! below it to read from, so the linker discards `f_objects::service` exactly as
//! it discards the four crates under it and the image does not move. The
//! prediction above was right about what answers the question and wrong about
//! which diff would be the one; it is left standing, with the measurement that
//! caught it, because the next reader will make the same substitution.

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
    // nothing like the mistake. Read here and unused, because this body has
    // nothing to use them *for*: `crate::service` answers an opcode now, but it
    // answers an entry somebody hands it, and the handle that would carry a
    // channel to take one off is exactly the one this component does not have.
    // Read rather than ignored, so that the day it does, the reading is already
    // in the right place.
    let entry = door::Entry::from_bits(argument);
    let _ = entry.granted(0);

    // The allocation, and what is new about it is which crate is making it.
    //
    // `user/store` already proved that a `#[global_allocator]` in a crate that
    // forbids `unsafe` hands out memory the frame granted. What was never proved
    // is that the crate this task needs can be one: `f-objects` links `f-blob`,
    // `f-zone`, `f-index` and `f-hash`, and `crate`'s own module comment named
    // that dependency graph as the reason no image could exist — a component
    // linking `f-blob` needs an allocator, an allocator is an `unsafe impl
    // GlobalAlloc`, and RFC 0001 forbids `unsafe` above the frame. `f_ring::heap`
    // is where that `unsafe` went, and this line is the whole of what a component
    // above the frame has to write to use it.
    //
    // Dropped immediately, so the frame's reading of the region's prologue shows
    // `live` back at zero and `peak` not — which is the pair that says the
    // allocation happened *and* came back. The address is black-boxed because
    // Rust may elide an allocation whose value never escapes, so a buffer that is
    // only written to is a buffer that may never have been allocated.
    #[cfg(all(target_os = "none", feature = "image"))]
    {
        let mut taken: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(64);
        taken.push(7);
        let at = taken.as_ptr() as u64;
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
