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
    // **`None` is *nothing died here*, and zero is not.** This was a `0u64`
    // sentinel on the argument that zero is not a cause, so no notice could
    // produce it — and the frame posted zero on every death, so every death read
    // as nothing and was refilled by the branch for a place never filled. RFC
    // 0126. A notice with no cause on it is now a death decided on cause zero,
    // which no policy restarts after.
    let mut ended: [Option<u64>; crate::routing::PLACES_MAX] = [None; PLACES_MAX];
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
                *slot = Some(entry.ext);
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
    //
    // A row a death arrived for is withheld from it: that is the second
    // question, and answering it by starting the member again is a restart no
    // policy decided. `assemble::Spawning::new` has the boot that showed it.
    let mut withheld = [false; PLACES_MAX];
    for (slot, death) in withheld.iter_mut().zip(ended.iter()) {
        *slot = death.is_some();
    }
    let assembled = crate::assemble::assemble(&board, &control, &withheld);

    // --- the decision --------------------------------------------------------
    let mut said = [crate::routing::Said::default(); PLACES_MAX];
    let mut submitted = assembled.as_ref().map_or(0, |it| it.submitted);
    let mut refused = 0;
    for (index, row) in board.rows.iter().enumerate().take(board.places) {
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
        let facts = crate::policy::Facts {
            ended: ended.get(index).copied().flatten(),
            occupied: row.occupant != 0,
            taken,
            budget: row.budget,
            liveness: row.liveness,
            seen: row.seen,
        };
        // Everything this run does about the row is `policy::answer`'s, which
        // says why the four cases are in the order they are. What is left here
        // is putting its answer on the ring and on the board.
        //
        // Decided against **the place's own manifest**, as the frame copied it,
        // and no longer against a function returning `user/store`'s numbers: the
        // compositor declares a budget of eight and a store of three, and a
        // timeout decided on the store's would be a restart line naming one
        // manifest's number over another's decision.
        let answered = crate::policy::answer(&row.policy.record(), &facts, board.now);
        if let Some(slot) = said.get_mut(index) {
            *slot = crate::routing::Said {
                verdict: answered.verdict.to_wire(),
                budget: answered.budget,
                cause: facts.ended.unwrap_or(0),
                heard: row.liveness,
                seen: answered.seen,
            };
        }
        // The fate, on the one route a supervisor has to end an occupant: a stop
        // against the place's endpoint, whose deadline is this consultation's
        // own tick — already reached, so a kill, which `op::STOP` spells the same
        // way as a polite stop on purpose. `control::at_once` because the tick
        // may be zero and zero is no deadline at all; the first boot of this
        // wrote `board.now` and was refused. The word goes in `ext[0]` and the
        // frame carries it onto the notice that tells the place's holders why;
        // RFC 0126 is the argument that it may, and `cause::named_above` is the
        // one word it will accept there.
        if let Some(named) = answered.stop {
            let entry = f_abi::Sqe {
                opcode: f_abi::control::op::STOP,
                cap: row.endpoint,
                user_data: index as u64 + 1,
                deadline: f_abi::control::at_once(board.now),
                ext: [named, 0],
                ..f_abi::Sqe::ZERO
            };
            if control.submit(entry).is_err() {
                refused += 1;
            }
        }
        let verdict = answered.verdict;
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

// `declared()` was here: the four manifest fields `policy::decide` reads, as a
// function returning `user/store`'s, because a supervisor holds a content hash
// and not the manifest behind it. Its own comment said it was right by
// coincidence and named its reversal — *a row that carries the four fields, the
// increment where two supervised places declare different policies*. `E3-B05e`
// supervises the compositor, which declares eight restarts in sixty thousand
// ticks, so the reversal fell due and was paid: `crate::routing::at::POLICY`.

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
