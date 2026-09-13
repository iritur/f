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
//! # What this does
//!
//! It reads the page the frame filled in for it, adopts its control ring, and
//! **submits `control::op::SPAWN` for every place the frame said it may fill**.
//! That is the act `TODO.md`'s E1-B05 has been about since it was written and
//! the one `kernel/src/component.rs` named in three bullets it called one thing:
//! *there is no supervisor component*.
//!
//! The previous version of this comment said the submission loop was
//! deliberately absent because writing it would be writing against a machine
//! state no boot reaches — a promised layer with no owner. The state is reached
//! now: RFC 0075 handed an occupant a core, and the boot that proved it also
//! found the reason no component had ever executed (`user/init/link.ld`).
//!
//! # What it still does not do, stated so the next reader does not go looking
//!
//! **It does not restart anything.** [`crate::policy`] is here and decides, and
//! nothing yet calls it from a notice, because being *told its occupant died* is
//! a `notice::PEER_GONE` for a place this component does not hold an endpoint to
//! — it holds manifests. That is the next increment and it is the one that
//! retires the frame's own restart demonstration.
//!
//! # Why it does not wait for its answers, which is the sharp edge here
//!
//! **Because nothing is answering while it runs, on purpose.** A driver's
//! control ring is served by the frame *during* the driver's run, because what a
//! driver asks for — a device translation — is resolved against the frame's own
//! tables. A spawn is not: it names the `Untyped` the submitter may spend, and
//! the submitter's table, while this component is running, is the live one on
//! this core. A frame that resolved handles in it from the boot processor would
//! be two cores reaching one table, which `CLAUDE.md` permits in exactly four
//! places and says a fifth needs an argument.
//!
//! So `kernel/src/component.rs` serves this ring *after* the core reports
//! finished, against the table it took back. Entries are memory rather than
//! events, so nothing is lost by the two ends not moving at once — what is lost
//! is this component's ability to see the answer, and that is stated here rather
//! than discovered by a loop that spins forever waiting for one.
//!
//! *What it costs:* this supervisor reports what it **submitted**, and the frame
//! reports what it **filled**. Those are two numbers from two sides and the boot
//! prints both, which is a better arrangement than one number anyway — but it is
//! not a supervisor that can retry, and a restart policy has to be able to.
//!
//! *Reversal:* [`crate::policy`] being called from a notice. That needs answers,
//! and answers need one of the two arrangements the frame's comment names.

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

    // "I am here." Submitted before the work rather than after it, so that a
    // supervisor which then wedges is still distinguishable from one that never
    // got a core — those are different failures and the boot log has to be able
    // to tell them apart.
    let _ = door::call0(door::ANNOUNCE);

    end(supervise())
}

/// Fill every place the frame said this component may fill.
///
/// Returns the status [`start`] ends with: [`DONE`] when every place named was
/// filled, and the packed refusal otherwise — so a boot reads *what went wrong*
/// out of the exit status rather than out of a counter it has to interpret.
fn supervise() -> u64 {
    let board = match crate::routing::Board::read() {
        Ok(board) => board,
        // Nothing to report through, because the page the report would go in is
        // the page that could not be read. The status is the whole answer.
        Err(packed) => return i64::from(packed) as u64,
    };

    // `CONTROL_EVENTS` required and not merely offered, which is the one
    // refusal a control ring depends on and `user/virtio-blk` states in the same
    // words: a control ring whose peer cannot speak notices is not a control
    // ring. A supervisor's dependence on it is stronger than a driver's — the
    // next increment's whole input is a notice.
    let control = match f_ring::adopt::Adopted::at(
        board.control_at,
        board.control_len,
        f_abi::feature::CONTROL_EVENTS,
        f_abi::feature::CONTROL_EVENTS,
    ) {
        Ok(adopted) => adopted.client(),
        // Zeroes rather than the refusal, because this half of the board counts
        // entries and the refusal is the exit status — which the frame reads
        // through the door and can therefore still see after this component's
        // memory has gone. Writing an error code into a field named for a count
        // would make both unreadable.
        Err(packed) => {
            crate::routing::Board::report(0, 0);
            return i64::from(packed) as u64;
        }
    };

    let mut submitted = 0;
    let mut refused = 0;
    for (index, manifest) in board.manifests.iter().enumerate().take(board.places) {
        // The token is the row, plus one so that it is never zero: zero is the
        // `user_data` an entry nobody set carries, and an answer matched against
        // it would match the frame's own notices. Nothing in this component
        // reads the answers, but the token is what makes them readable *later*
        // — by the frame draining this ring, and by the supervisor that
        // eventually waits for them.
        let token = index as u64 + 1;
        let entry = f_abi::Sqe {
            opcode: f_abi::control::op::SPAWN,
            cap: board.account,
            user_data: token,
            ext: [*manifest, 0],
            ..f_abi::Sqe::ZERO
        };
        // A full ring is the one refusal this component can see for itself, and
        // it is a real one: the frame is not draining yet, so a supervisor with
        // more places than ring entries would silently submit a prefix. Counted
        // rather than retried — there is nobody to wait for.
        match control.submit(entry) {
            Ok(_) => submitted += 1,
            Err(_) => refused += 1,
        }
    }

    crate::routing::Board::report(submitted, refused);
    if refused == 0 { DONE } else { i64::from(RING_FULL) as u64 }
}

/// What this component ends with when it could not put an entry on its own ring.
///
/// `RESOURCE/QUOTA_EXHAUSTED` and not `PEER/GONE`: the peer is there and will
/// drain this ring the moment this component stops holding the core. What ran
/// out is room, which is a quantity, and reporting it as a departed peer would
/// send a reader looking for a frame that never went anywhere.
const RING_FULL: i32 =
    f_abi::error::pack(f_abi::error::RESOURCE, f_abi::error::resource::QUOTA_EXHAUSTED);

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
