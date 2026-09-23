// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `panel` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to, and the six entries it says before it
//! ends.
//!
//! # Why there is no attribute on [`start`]
//!
//! Because there cannot be, and `user/init/src/component.rs` records the scar at
//! length: naming an entry point means `#[unsafe(no_mangle)]` or
//! `#[unsafe(link_section)]`, both of which are unsafe attributes in this
//! edition, and a crate that forbids unsafe code cannot write one. So
//! `user/init/link.ld` places the section this function is compiled into at the
//! image's first byte, and `cargo xtask component` checks that the symbol which
//! actually landed there is this one.
//!
//! # The loop is not a server's, and that is the whole shape
//!
//! Every other component in this tree waits for work. This one has work: it
//! submits an entry, waits for the completion that answers it, and submits the
//! next. It ends when the script is finished rather than when the frame says
//! stop, so there is no control ring in this life and no `TOLD` outcome — the
//! frame learns the run is over the way it learns any process is, which is that
//! the join returns.
//!
//! **One entry at a time, and it is a rule rather than a simplification.** The
//! payload travels in the channel's own arena and this component writes every
//! payload at offset zero; a second entry submitted before the first was
//! answered would overwrite bytes the frame had not read yet, and the frame
//! would lose an entry out of a frame that then could not close. Batching is
//! `E3-B01j`'s to count and `E3-B01l`'s reconciler to produce.
//!
//! # What it never finds out
//!
//! Whether the tree it declared still exists. It holds a handle — a number the
//! frame wrote on its board — and it holds nothing else; there is no entry in
//! `f_abi::semantic` that asks a receiver what a tree contains, and this
//! component would have nowhere to put the answer. That is section 11's
//! inversion from the application's side, and it is worth noticing that from
//! here it looks like *less*: an application under the old arrangement knows its
//! own interface, and this one has to ask. What it buys is everything on the
//! other side of the ring, and `docs/design/ring-scene-boot.html` part III is
//! where the case is made.

use f_abi::semantic::{Delta, Entry};
use f_abi::{Cqe, door, state};
use f_ring::adopt::{Adopted, Client};
use f_ring::device::Window;

use crate::routing::{self, at, life, node, reported, stopped};
use crate::script;

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
    let entry = door::Entry::from_bits(argument);
    // Which of this component's lives the frame asked for. A selector this build
    // does not name falls through to the announcement rather than inventing a
    // third, which is what a *spawn* into a place asks for and is what every
    // ordinary boot of this tree gets from this component.
    if entry.selector() == life::DECLARE {
        declare()
    }

    // The frame tells a component what it holds rather than letting it assume,
    // and `door::Entry` argues why: a second occupant of a place finds its
    // capabilities at the same indices and a later generation.
    let _ = entry.granted(0);

    // "I am here." The one thing the frame cannot observe from outside.
    let _ = door::call0(door::ANNOUNCE);

    end(DONE)
}

/// Say the interface, and end.
///
/// Every failure here ends the run with a reason in [`reported::OUTCOME`] rather
/// than a panic, and the reason matters: a component that stopped because its
/// routing page was blank and one that stopped because the frame refused an
/// entry look identical from outside, and only one of them is a disagreement
/// about a vocabulary.
fn declare() -> ! {
    let Ok(board) = Window::at(routing::AT, routing::BYTES) else {
        // Nothing to report *into*, so the status word is all there is.
        end(stopped::BAD_ROUTING)
    };
    // R04 at the one place this component reads a structure it did not build. A
    // page of zeroes is what a frame that was mapped and never filled in looks
    // like, and a zero length taken for a length reads as a peer problem rather
    // than as a frame that did not speak.
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        report(&board, &Said::NOTHING, stopped::NO_ROUTING);
        end(stopped::NO_ROUTING)
    }

    let Some(parts) = laid_out(&board) else {
        report(&board, &Said::NOTHING, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    };

    // **Before a single entry, and it is the one refusal this component makes
    // about its own authority.** An application that declared an interface it
    // was granted no right to declare would have every entry turned down one at
    // a time, and the boot would read six refusals where the cause was one
    // absent word.
    if parts.handle == 0 {
        report(&board, &Said { handle: 0, ..Said::NOTHING }, stopped::NO_HANDLE);
        end(stopped::NO_HANDLE)
    }

    let Ok(entries) = script::entries() else {
        report(&board, &Said { handle: parts.handle, ..Said::NOTHING }, stopped::NO_SCRIPT);
        end(stopped::NO_SCRIPT)
    };

    let mut said = Said { handle: parts.handle, ..Said::NOTHING };
    let outcome = say(&parts, &entries, &mut said);
    report(&board, &said, outcome);
    end(outcome)
}

/// What this component counted while it spoke.
///
/// Every field is a number the frame also knows, because the frame is the peer
/// that answered them. None of them is this component's opinion about its own
/// success, which is the property `kernel/src/semantic.rs` rests its verdict on.
struct Said {
    /// Entries put on the ring. Unit: entries.
    submitted: u64,
    /// Completions reaped, whatever they said. Unit: entries.
    answered: u64,
    /// Completions carrying a refusal, or answering an entry this component was
    /// not waiting for. Unit: entries.
    refused: u64,
    /// The handle the frame said this component holds, quoted back.
    /// Unit: none — a capability handle's bits.
    handle: u64,
}

impl Said {
    /// A component that has said nothing yet.
    const NOTHING: Self = Self { submitted: 0, answered: 0, refused: 0, handle: 0 };
}

/// Submit the script, one entry at a time, and wait for each answer.
///
/// Answers the outcome the run ends with.
fn say(parts: &Parts, entries: &[Entry; script::ENTRIES], said: &mut Said) -> u64 {
    for (position, body) in entries.iter().enumerate() {
        // `user_data` is the entry's position, one-based, so that zero is never
        // a token this component is waiting for: a completion carrying a zeroed
        // field would otherwise match the first entry.
        let token = position as u64 + 1;
        let delta = Delta { user_data: token, class: 0, payload_offset: 0, flags: 0, body: *body };
        let (entry, payload) = delta.encode();
        if !parts.data.copy_in(0, &payload) {
            return stopped::NO_RING;
        }
        if parts.data.submit(entry).is_err() {
            return stopped::NO_RING;
        }
        said.submitted += 1;

        match wait(parts, token) {
            Waited::Answered { refused } => {
                said.answered += 1;
                if refused {
                    said.refused += 1;
                    return stopped::TURNED_DOWN;
                }
            }
            Waited::Silent => return stopped::UNANSWERED,
            Waited::Broken => return stopped::NO_RING,
        }
    }
    stopped::SAID
}

/// What waiting for one completion produced.
enum Waited {
    /// One arrived, and whether it was a refusal or an answer to something else.
    Answered {
        /// The completion was a refusal, or named an entry this component was
        /// not waiting for. Two things at once on purpose: a client that checked
        /// only the result would count the answer to entry three as the answer
        /// to entry four and never notice.
        refused: bool,
    },
    /// None arrived inside the bound. RFC 0046 — a hang is a count.
    Silent,
    /// The ring stopped validating, which is a peer that has stopped speaking.
    Broken,
}

/// Wait for the completion that answers `token`.
fn wait(parts: &Parts, token: u64) -> Waited {
    let mut idle: u64 = 0;
    loop {
        match parts.data.take() {
            Ok(Some(answer)) => return Waited::Answered { refused: badly(&answer, token) },
            Ok(None) => {
                idle += 1;
                if idle > parts.spins {
                    return Waited::Silent;
                }
                core::hint::spin_loop();
            }
            Err(_) => return Waited::Broken,
        }
    }
}

/// Is this completion anything other than *the entry it names was accepted*?
fn badly(answer: &Cqe, expected: u64) -> bool {
    answer.result < 0 || answer.user_data != expected
}

/// Everything the routing page said, in the types that use it.
struct Parts {
    /// The ring this component declares across, from the submitting end.
    data: Client,
    /// The right it holds over the tree it is about to declare.
    /// Unit: none — a capability handle's bits.
    handle: u64,
    /// Turns spent waiting for one completion before giving up. Unit: turns.
    spins: u64,
}

/// Read the routing page and state everything it names.
///
/// `None` for any address that cannot be stated as a channel — which is a frame
/// that filled this page in wrongly, and is refused here rather than
/// dereferenced to find out.
fn laid_out(board: &Window) -> Option<Parts> {
    // The ring offers nothing and requires nothing. A semantic entry's payload
    // travels in the channel's own arena — `payload = "inline"` in the manifest
    // — so there is no registration to negotiate and no feature bit that would
    // mean anything here.
    let data = Adopted::at(
        board.read64(at::DATA_AT).ok()?,
        u32::try_from(board.read64(at::DATA_LEN).ok()?).ok()?,
        0,
        0,
    )
    .ok()?
    .client();

    // What the frame negotiated on this component's behalf, refused rather than
    // carried. This build speaks one ABI version and offers no feature, so
    // anything else here is a peer that agreed to something this component does
    // not implement, and R04 says refuse rather than proceed and misread.
    if board.read64(at::NEGOTIATED_VERSION).ok()? != u64::from(f_abi::ABI_VERSION) {
        return None;
    }
    if board.read64(at::NEGOTIATED_FEATURES).ok()? != 0 {
        return None;
    }

    // A bound of zero is a component that gives up before it has looked once,
    // which is a routing page this build cannot honour rather than a frame that
    // meant *do not wait*. Refused here, where it is a layout, rather than
    // discovered as a run that said nothing.
    let spins = board.read64(at::IDLE_SPINS).ok()?;
    if spins == 0 {
        return None;
    }

    // The tree's address is deliberately *not* read here and carried. It is
    // read once, in `publish`, at the moment it is used — because a component
    // that held an address it was told and did not use would be holding the one
    // kind of field that can be wrong with nothing to notice, and this page has
    // one such field already in `TREE_SLOT`, which is quoted and never
    // dereferenced.
    Some(Parts { data, handle: board.read64(at::TREE_HANDLE).ok()?, spins })
}

/// Write what this component did into the half of the routing page that is its
/// own, and into the tree the frame mounted for it.
///
/// The magic goes last, which is the whole of the discipline: a frame that reads
/// a page this function never finished finds a zero rather than a plausible
/// tally. RFC 0013's *read, never delivered* — the frame takes these numbers out
/// of memory it granted, and this component is never asked for them.
fn report(board: &Window, said: &Said, outcome: u64) {
    let _ = board.write64(reported::SUBMITTED, said.submitted);
    let _ = board.write64(reported::ANSWERED, said.answered);
    let _ = board.write64(reported::REFUSED, said.refused);
    let _ = board.write64(reported::HANDLE, said.handle);
    let _ = board.write64(reported::OUTCOME, outcome);
    let _ = board.write64(reported::PUBLISHED, publish(board, said));
    let _ = board.write64(reported::MAGIC, routing::MAGIC);
}

/// Store this run's counts into the tree the frame published for this instance.
///
/// **The address comes off the board and is never assumed.** Only the shapes
/// that publish a tree map one, so a component holding the constant would fault
/// at ring 3 on every boot that stood it up some other way.
///
/// Answers how many nodes took a word, which the frame reads back beside the
/// tree itself. A node the schema does not carry is refused by
/// `f_abi::state::Writer::set`, so a manifest and a [`node`] list that disagree
/// produce a number below [`node::WRITTEN`]'s length rather than a silence.
///
/// This is **not** the semantic tree. It is RFC 0013's monitoring tree, in a page
/// mapped into this address space, and the difference is the task: what this
/// component writes here is what it *did*, and what it declared across the ring
/// is what it *is* — one lives in memory the frame will take back when this
/// component ends, and the other in memory this component was never shown.
/// Unit: nodes.
fn publish(board: &Window, said: &Said) -> u64 {
    let Ok(tree_at) = board.read64(at::TREE_AT) else { return 0 };
    if tree_at == 0 {
        return 0;
    }
    let Ok(tree) = state::Writer::at(tree_at, routing::TREE_BYTES) else { return 0 };
    let mut written = 0;
    for (id, value) in [
        (node::SUBMITTED, said.submitted),
        (node::ANSWERED, said.answered),
        (node::REFUSED, said.refused),
        (node::HANDLE, said.handle),
    ] {
        if tree.set(id, value) {
            written += 1;
        }
    }
    written
}

/// End, and do not come back.
fn end(status: u64) -> ! {
    let _ = door::call(door::EXIT, status, 0);
    // `EXIT` does not return. If it ever did, the frame would have a component it
    // believes is over and a core still inside it, so the only honest thing left
    // is to stop moving.
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
/// There is no formatting, no unwinding and nothing to print to: a component has
/// no serial port. Stopping is the whole handler, and the frame notices the same
/// way it notices anything else — the component stops making progress and the
/// join's bound passes.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
