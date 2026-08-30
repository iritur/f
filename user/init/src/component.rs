// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `init` as something that runs: the first thing this system executes that the
//! kernel did not compile into itself.
//!
//! # What is different about this file
//!
//! Everything under `kernel/` is the frame. `kernel::arch::x86_64::probe` is
//! the frame's adversary — sixty instructions of hand-written assembly,
//! assembled into the kernel's own `.rodata`, there because a milestone needed
//! *something* at ring 3 before there was a loader. This is the other thing: an
//! ordinary component, in ordinary Rust, compiled separately, linked
//! separately, handed to the machine by the boot loader as a file, and copied
//! into a frame it was granted. The kernel does not contain it and cannot see
//! inside it.
//!
//! It contains no `unsafe`. Not by convention: this crate inherits
//! `unsafe_code = "forbid"` from the workspace and `cargo xtask lint-unsafe`
//! fails the build if that stops being true. The one instruction that crosses
//! the boundary is in [`f_abi::door`], which is part of the frame and is
//! reviewed as one — that module argues why it is the right side of the line
//! rather than a convenient one.
//!
//! # Why there is no attribute on [`start`]
//!
//! Because there cannot be. Naming an entry point means `#[unsafe(no_mangle)]`
//! or `#[unsafe(link_section)]`, both of which are *unsafe attributes* in this
//! edition, and a crate that forbids unsafe code cannot write one — the lint
//! does not distinguish an attribute whose hazard is a duplicate symbol from a
//! dereference of a wild pointer.
//!
//! That is a real consequence of the policy rather than an oversight, and the
//! answer is to put the placement where it belongs anyway. `user/init/link.ld`
//! names the section this function is compiled into and places it at the start
//! of the image; `cargo xtask init` then checks that the symbol which actually
//! landed at the image's first byte is this one, so a toolchain that changes
//! how it names sections breaks the build loudly instead of producing an image
//! that starts in the middle of something.
//!
//! # What it does, and why so little
//!
//! It announces itself, uses the three capabilities it was granted — correctly
//! — asks the frame repeatedly whether it has run long enough, and ends.
//!
//! That is deliberately the same sequence the probe's preamble makes, and not
//! by coincidence: it means the frame has one expectation of a well-behaved
//! process rather than two, so this component is judged by a tally that already
//! existed. A component with its own private notion of correct behaviour would
//! be a component whose misbehaviour looked like a new test failing rather than
//! an old one.
//!
//! There is no channel to say anything on and no ring to submit anything to.
//! Both are M5. Until then a component's whole vocabulary is the door, and the
//! honest version of `init` is short.
//!
//! # What it may assume about where it is
//!
//! Its text is mapped read-only and executable, its stack is one writable page,
//! and one further page is reachable only if it maps it. Nothing else — in
//! particular there is no writable static anywhere in this image, because the
//! text page is not writable and a mutable global would fault on first write
//! rather than fail to link. Nothing here needs one, and `cargo xtask init`
//! checks the image has no writable data rather than trusting that nobody adds
//! one.

use f_abi::cap::{Handle, rights};
use f_abi::door;

/// Which of the frame's grants is this component's address space.
///
/// The grants arrive as one handle and an order, not as a list: the frame puts
/// them in consecutive slots and tells the component the first, so an index is
/// the whole of what has to be known here. [`door::Entry`] argues why they are
/// told rather than assumed, and the short version is that a second process on
/// a core finds them at a later generation.
const SPACE: u16 = 0;

/// The second grant: one frame, carrying read and derive and revoke, and
/// pointedly not write.
const FRAME: u16 = 1;

/// Where a frame may be mapped.
///
/// One unmapped page above the top of the stack, which is the frame's layout
/// and not this component's choice. It is a constant here for the same reason
/// the handles are: there is no way to be told it yet.
const GRANT: u64 = 0x0040_4000;

/// A run that did what it meant to.
pub const DONE: u64 = 0;

/// The inspect was refused.
///
/// Any non-zero status fails the boot. They are distinguished so that the log
/// says *which* step went wrong rather than only that one did.
pub const REFUSED_INSPECT: u64 = 1;

/// The derive was refused.
pub const REFUSED_DERIVE: u64 = 2;

/// The mapping was refused.
pub const REFUSED_MAP: u64 = 3;

/// The frame gave up waiting for the ticks it wanted out of ring 3.
pub const GAVE_UP: u64 = 4;

/// How long to wait between asking the frame whether to stop.
///
/// A bounded pause and not a measurement. What it costs does not matter — the
/// frame decides when this component has run long enough — and it exists only so
/// that the polling loop is not one system call per instruction, which would
/// make the boot a measurement of the door.
const PAUSE: u32 = 4096;

/// Where the frame starts this component.
///
/// The image is flat and the frame jumps to its first byte, so this has to be
/// the first thing in `.text`. The module comment says how that is arranged and
/// how it is checked.
///
/// It never returns: [`door::EXIT`] does not come back, and the loop after it is
/// what happens if the frame ever lets it.
pub fn start(argument: u64) -> ! {
    let entry = door::Entry::from_bits(argument);

    // "I am here." The frame records that ring 3 ran at all, which is the one
    // thing it cannot observe from outside.
    let _ = door::call0(door::ANNOUNCE);

    let status = use_what_was_granted(entry);
    if status != DONE {
        end(status);
    }

    // Ask, rather than count. How long a fixed number of iterations takes
    // differs by two orders of magnitude between an emulator and a machine, so
    // a component that measured its own lifetime would make the boot log depend
    // on how fast the host is — and the boot log is a fixture. The frame counts
    // timer ticks taken out of ring 3 and answers when it has had enough.
    loop {
        match door::call0(door::PROGRESS) {
            door::KEEP_GOING => spin(),
            door::ENOUGH => end(DONE),
            // The frame stopped waiting. Ending with a status that says so is
            // better than looping, which is a boot that hangs.
            _ => end(GAVE_UP),
        }
    }
}

/// Inspect a capability, narrow it, and map what came out.
///
/// Three calls, and they are the positive control the whole capability suite
/// rests on: a frame that refused everything would pass every negative test
/// there is. Nothing here tries to exceed what was granted — that is the
/// probe's job, and it is written in assembly precisely so that it can attempt
/// things this language will not express.
fn use_what_was_granted(entry: door::Entry) -> u64 {
    let space = entry.granted(SPACE);
    let frame = entry.granted(FRAME);

    if door::call(door::CAP_INSPECT, u64::from(frame.bits()), 0) < 0 {
        return REFUSED_INSPECT;
    }

    // A copy: the same rights, which makes it a child of the capability it came
    // from rather than a peer of it, so that a revoke of the parent reaches it.
    // `kernel/src/cap.rs` argues that against seL4.
    let copy = door::call(
        door::CAP_DERIVE,
        u64::from(frame.bits()),
        u64::from(rights::READ | rights::DERIVE | rights::REVOKE),
    );
    if copy < 0 {
        return REFUSED_DERIVE;
    }
    // Non-negative, so the narrowing keeps every bit that means anything: a
    // handle is thirty-two bits and the answer is a widened one.
    let Ok(bits) = u32::try_from(copy) else {
        return REFUSED_DERIVE;
    };
    let copy = Handle::from_bits(bits);

    // Read-only, because the capability this was derived from carries no write
    // right. Asking for write here is what `cap=rights` does, and it is refused.
    if door::call(
        door::CAP_MAP,
        door::map_operands(space, copy),
        door::map_address(GRANT, rights::READ),
    ) < 0
    {
        return REFUSED_MAP;
    }

    DONE
}

/// End, and do not come back.
fn end(status: u64) -> ! {
    let _ = door::call(door::EXIT, status, 0);
    // `EXIT` does not return. If it ever did, the frame would have a process it
    // believes is over and a core that is still inside it, so the only honest
    // thing left is to stop moving.
    park()
}

/// Wait a little before asking again.
fn spin() {
    let mut left = PAUSE;
    while left > 0 {
        core::hint::spin_loop();
        left -= 1;
    }
}

/// Stop, without ending. Reached only where continuing would be worse.
fn park() -> ! {
    loop {
        core::hint::spin_loop();
    }
}

/// What happens if this component panics, which nothing in it can do.
///
/// There is no formatting, no unwinding and nothing to print to: a component
/// has no serial port. Stopping is the whole handler, and the frame notices the
/// same way it notices anything else — the process stops asking for progress,
/// the give-up bound is reached, and the boot fails saying so.
///
/// Present in every build of this crate except the test one, and the exception
/// is not about the target. A `staticlib` needs a panic handler wherever it is
/// built, including on the host, because nothing else in the link will supply
/// one — `no_std` means `std` is not there to. The test harness *is* linked
/// against `std`, which brings its own, and two crates claiming `panic_impl` is
/// a link error rather than a choice.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
