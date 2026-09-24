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
//! Three things, in this order, and the order is the design:
//!
//! 1. **Drains its control ring.** Everything the frame had to say is already
//!    there when the core is handed over — RFC 0076 — so this is where a
//!    `notice::PEER_GONE` becomes *this place lost its occupant*.
//! 2. **Decides.** [`crate::policy::decide`] is RFC 0008's restart rule, above
//!    the frame at last, over the tally the board carried in.
//! 3. **Submits.** `control::op::SPAWN` for every place it decided to refill,
//!    and a verdict written back for every place it did not — because a retire
//!    has no opcode behind it and travels on the board instead.
//!
//! # Why it does not wait for its answers
//!
//! **Because nothing answers while it runs, on purpose.** A driver's control
//! ring is served by the frame *during* the driver's run, because what a driver
//! asks for — a device translation — is resolved against the frame's own tables.
//! A spawn is not: it names the `Untyped` the submitter may spend, and the
//! submitter's table, while this component is running, is the live one on this
//! core. A frame that resolved handles in it from the boot processor would be
//! two cores reaching one table, which `CLAUDE.md` permits in exactly four
//! places and says a fifth needs an argument.
//!
//! So `kernel/src/component.rs` serves this ring *after* the core reports
//! finished, against the table it took back, and the answers wait there until
//! the next consultation. Entries are memory rather than events; nothing is lost
//! by the two ends not moving at once.
//!
//! **RFC 0076 is what turns that from a limitation into a shape.** A supervisor
//! is not a daemon with a loop — it is a thing the frame *runs when it has
//! something to tell it*. Being told happens at the start of a run and acting
//! happens during it, so the only thing this component cannot do is react to a
//! death that arrives while it is already on a core. It is told about that one
//! on its next run, and the RFC names the observation that would make that
//! insufficient.
//!
//! Nothing in this image survives between two runs — there are no writable
//! statics in a component and the core clears the table on the way out — so the
//! restart tally travels on the board. RFC 0076 records that seam: the frame
//! stores those two numbers and never reads them.

use crate::routing::PLACES_MAX;
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

/// Decide what to do about every place the frame named, and do the part of it
/// that is a submission.
///
/// Returns the status [`start`] ends with: [`DONE`] for a run that got every
/// entry it wanted onto its ring, and a packed refusal otherwise — so a boot
/// reads *what went wrong* out of the exit status rather than out of a counter
/// it has to interpret.
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
    // ring. A supervisor's dependence on it is stronger than a driver's — a
    // death arrives on it, and a supervisor that cannot be told is not one.
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
            crate::routing::Board::report((0, 0, 0), &[]);
            return i64::from(packed) as u64;
        }
    };

    // --- what happened, taken off the ring before anything is decided --------
    //
    // RFC 0076: a supervisor is told when it is started. Everything the frame
    // had to say is already here, posted into this ring before the core was
    // handed the job, and this drain is the polling point R05 says every event
    // arrives at. It is **not** a wait — an empty ring means nothing happened,
    // which is the ordinary case and the reason this returns rather than spins.
    let mut told = 0;
    // The cause each place's occupant went by, as the notice carried it, packed.
    //
    // **Words and not flags, since `E3-B05e`.** This was `[bool; PLACES_MAX]`
    // and the decision below read it as *faulted*, which meant a supervisor told
    // an occupant had exited would have restarted it as though it had crashed —
    // under a policy whose entire content is telling those two apart. The cause
    // has been in `Cqe::ext` since RFC 0008 and nothing read it.
    //
    // Zero is *nothing died here*, which is safe for the reason it is safe on
    // the routing page: zero is not a cause, so no notice can produce it.
    let mut ended = [0u64; crate::routing::PLACES_MAX];
    while let Ok(Some(entry)) = control.take() {
        if !f_abi::control::is_notice(&entry) {
            // An answer to something submitted on a previous run, arriving now
            // because nothing answers this ring while its owner holds a core.
            // Dropped rather than misread: the verdict it would inform has
            // already been acted on by the frame.
            continue;
        }
        // R04: a kind this build does not define is the frame speaking a
        // protocol this component was not built against, and reading on after
        // one would be guessing.
        if !f_abi::control::notice::known(entry.result) {
            break;
        }
        if entry.result != f_abi::control::notice::PEER_GONE {
            continue;
        }
        told += 1;
        // R04 again, one field in: a cause this build cannot read is carried
        // through rather than translated, and `Record::restarts_after_cause`
        // answers `Leave` for it. Refused at the decision and not here, because
        // *told and did not act* is a fact worth having in `TOLD` — a drain that
        // dropped the notice would report a death nobody was told about.
        for (index, row) in board.rows.iter().enumerate().take(board.places) {
            // Which place, by the endpoint the notice pends in. The board is the
            // only thing that maps a handle to a row, which is why a supervisor
            // is given its endpoints rather than left to infer them.
            if u64::from(row.endpoint) == entry.user_data
                && let Some(slot) = ended.get_mut(index)
            {
                *slot = entry.ext;
            }
        }
    }

    // --- the generation ------------------------------------------------------
    //
    // **RFC 0094, and the act this component was missing.** Between the drain and
    // the decision, because the two answer different questions and only one of
    // them is about a death: the assembler answers *what does this generation say
    // to start*, and `policy::decide` answers *should this place be refilled now
    // that its occupant has died*. They meet at a row, and a row the assembler
    // started is one the loop below finds already filled.
    //
    // `None` on every boot that selected no generation, which is every boot with
    // no `f.root=` — the fault boots, the datapath boots, `cargo xtask user`.
    // Those run exactly the loop they ran before this landed.
    let assembled = crate::assemble::assemble(&board, &control);

    // --- the decision --------------------------------------------------------
    let mut said = [(crate::policy::Verdict::Leave, crate::policy::Budget::default()); PLACES_MAX];
    let mut submitted = assembled.as_ref().map_or(0, |it| it.submitted);
    let mut refused = 0;
    for (index, row) in board.rows.iter().enumerate().take(board.places) {
        let mut budget = row.budget;
        // A place nothing died in has nothing to decide about — except on the
        // first consultation, where the frame has handed over a place it built
        // and deliberately never filled. Those two are told apart by whether a
        // death arrived, and by the tally being untouched. A flag on the board
        // saying *fill this* would be the frame deciding and this component
        // typing, which is the thing RFC 0008 refuses.
        // A row the assembler already submitted a spawn for is not a row with
        // nothing in it. Without this the loop below would see an untouched
        // tally, read it as *the frame built this place and never filled it*,
        // and submit a second spawn the frame answers by refusing — which would
        // be this component arguing with itself in the boot log.
        //
        // Asked of the row rather than of the run: a run that skipped every
        // member leaves every tally exactly as it found it, so *the assembler
        // ran* cannot tell a taken row from an untouched one. The first attempt
        // at this asked the run, and the boot it produced left the held-open
        // place empty and failed with `a spawn named a place it may not occupy`.
        let taken =
            assembled.as_ref().and_then(|it| it.filled.get(index)).copied().unwrap_or(false);
        let verdict = if taken {
            crate::policy::Verdict::Leave
        } else if let Some(packed) = ended.get(index).copied()
            && packed != 0
        {
            // The cause the frame put on the notice, and not this component's
            // guess at one. A stop still never restarts — it is this
            // supervisor's own act — but now because the *word* says `STOPPED`
            // rather than because a boolean was hard-coded to say fault.
            crate::policy::decide(
                &declared(),
                &mut budget,
                f_abi::control::cause::of(packed),
                board.now,
            )
        } else if budget == crate::policy::Budget::default() {
            crate::policy::Verdict::Restart(0)
        } else {
            crate::policy::Verdict::Leave
        };
        if let Some(slot) = said.get_mut(index) {
            *slot = (verdict, budget);
        }
        if !matches!(verdict, crate::policy::Verdict::Restart(_)) {
            continue;
        }
        // The token is the row, plus one so that it is never zero: zero is the
        // `user_data` an entry nobody set carries, and an answer matched against
        // it would match the frame's own notices.
        let entry = f_abi::Sqe {
            opcode: f_abi::control::op::SPAWN,
            cap: board.account,
            user_data: index as u64 + 1,
            ext: [row.manifest, 0],
            ..f_abi::Sqe::ZERO
        };
        // A full ring is the one refusal this component can see for itself, and
        // it is a real one: nothing drains this ring while this component holds
        // the core, so a supervisor with more places than ring entries would
        // silently submit a prefix. Counted rather than retried — there is
        // nobody to wait for.
        match control.submit(entry) {
            Ok(_) => submitted += 1,
            Err(_) => refused += 1,
        }
    }

    crate::routing::Board::report((submitted, refused, told), &said[..board.places]);
    if let Some(assembled) = assembled.as_ref() {
        crate::routing::Board::assembled(assembled);
    }
    if refused == 0 { DONE } else { i64::from(RING_FULL) as u64 }
}

/// The manifest fields [`crate::policy::decide`] reads.
///
/// # Why a supervisor does not read the manifest itself
///
/// Because it has no way to. A manifest is compiled into a record beside the
/// component's image (RFC 0030) and lives in a boot module the frame maps for
/// itself; a supervisor holds a content *hash*, which names a manifest and does
/// not reach it. Handing the whole record across would be handing a component a
/// page it did not ask for, and every field in it but four is the frame's
/// business.
///
/// **So this is a stated gap rather than a helper**, and it is a function with
/// this comment rather than four literals inline so that it is greppable. What
/// it returns is the policy `user/store` declares, which is the manifest behind
/// every place this boot supervises — so it is right today and is right by
/// coincidence.
///
/// *Reversal:* a row that carries the four fields. It costs the board 32 bytes
/// and the frame four writes it already has the values for, and it is not here
/// because the increment that needs it is the one where two supervised places
/// declare *different* policies. This boot has one manifest behind all of them,
/// and a field that cannot yet differ is a field nothing tests.
fn declared() -> f_abi::manifest::Record {
    f_abi::manifest::Record {
        restart: f_abi::manifest::restart::ON_FAULT,
        max_restarts: 3,
        budget_window_ticks: 3000,
        backoff_first_ticks: 8,
        backoff_max_ticks: 64,
        ..f_abi::manifest::Record::EMPTY
    }
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
