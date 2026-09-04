// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A user-level runtime: a component that holds a core, schedules its own work
//! inside it, and crosses the boundary exactly once — on the way out.
//!
//! # What this is the first of
//!
//! `deadline-all-the-way-down` section 02 has said since before M1 that *the
//! kernel does not schedule tasks; it allocates cores to runtimes, and a
//! runtime schedules its own work inside that allocation with no kernel
//! involvement*. Until this module there was nothing above the frame that could
//! do that, for one reason and it was not scheduling: driving a ring means
//! adopting a mapped channel, `f_ring::Mapping::adopt` is `unsafe`, and a
//! `user/` crate may not write it. `f_ring::adopt` is the answer — RFC 0037 —
//! and this is the first thing that stands on it.
//!
//! # The loop, and where the boundary is
//!
//! One **quantum** is [`report::QUANTUM`] work items: submitted onto this
//! runtime's own ring, executed off it, and reaped. Between quanta the runtime
//! returns to its **polling point** and drains its control ring, which is where
//! every event it will ever receive arrives (R05, RFC 0008). Those two facts
//! are one fact: *the polling point is the allocation boundary*. A reclaim
//! notice is acted on there and nowhere else, so the frame never has to
//! interrupt a task, and a task never has to be written to be interruptible.
//!
//! What that buys is the exit criterion, and it is worth stating as an absence:
//! between the `iretq` that enters this component and the `EXIT` that leaves
//! it, **this code executes no instruction that crosses a privilege boundary**.
//! Not a system call, not a fault, not a page it has not been given. Everything
//! it does happens in two frames of its own memory. The frame counts what
//! crossed and requires zero — `kernel/src/runtime.rs`, and RFC 0038 for what
//! that count excludes and why.
//!
//! # Parking cleanly, which is the half a deadline cannot express
//!
//! On a reclaim notice this runtime stops taking new work and finishes what it
//! already submitted, then exits. The number that says it parked *cleanly* is
//! not the deadline it met — a runtime that stopped at the deadline with tasks
//! still on its ring has abandoned them rather than parked them. It is
//! [`report::QUIESCENT`]: its own queue was empty when it went.
//!
//! # Both ends of one ring, said out loud
//!
//! The work ring is adopted twice — once as a [`Client`] that submits and
//! reaps, once as a [`Server`] that drains and answers — because an executor
//! genuinely is both ends of its own queue. `f_ring::Mapping`'s safety note
//! already permits it in the sentence it was written for: *two ends sharing a
//! region is the intended use, not a violation of it*. The control ring is
//! adopted once, as a client, because the frame is the only producer on it and
//! this component may never be.

use f_abi::control::{is_notice, notice};
use f_abi::{Cqe, Sqe, door, feature};
use f_ring::{Adopted, RingError};

use crate::report::{self, Tally};

/// Where the frame maps this runtime's control ring.
///
/// A constant, for the reason `user/init/src/component.rs` gives about its own
/// two: there is no way to be told yet. RFC 0008 says a component's first
/// instruction runs with the address of its control ring in a register, and
/// `f_abi::door::Entry` carries a selector and a handle instead — so until that
/// word grows a third field, this is the frame's layout written down twice and
/// checked by the machine. It must equal `kernel::process::RING`, and a build
/// where it does not is a page fault at the first adoption, reported by the
/// frame as an ordinary ring-3 fault.
/// Unit: bytes, in this component's address space.
const CONTROL_AT: u64 = 0x0040_7000;

/// Where the frame maps this runtime's own work ring.
///
/// Must equal `kernel::process::WORK`. See [`CONTROL_AT`].
/// Unit: bytes, in this component's address space.
const WORK_AT: u64 = 0x0040_8000;

/// How many bytes each of them is. One frame, which is what the account paid
/// for. Unit: bytes.
const REGION_BYTES: u32 = 4096;

/// The call the provocation makes.
///
/// An opcode the door does not implement, on purpose. The whole content of the
/// provocation is *that the boundary was crossed*, so a call which accomplished
/// something would make the number partly about that call; a refused one
/// crosses exactly as far and does nothing else. The frame answers
/// `ARGUMENT/UNKNOWN_OPCODE` and counts the entry, which is the point.
/// Unit: none — a door call number.
const NOTHING: u64 = 0xFF;

/// Hold a core and do the work in it.
///
/// `selector` is [`report::RUN`] or [`report::PROVOKE`]; anything else runs the
/// load without the provocation, because a selector this build does not name is
/// not a reason to invent behaviour.
///
/// It never returns: `door::EXIT` does not come back, and [`park`] is what
/// happens if the frame ever lets it.
pub fn run(selector: u32) -> ! {
    // The control ring, adopted the way a component adopts anything the frame
    // mapped for it: validated once, believed once, and re-checked at every
    // access thereafter. `CONTROL_EVENTS` is *required* rather than offered —
    // a control ring whose peer cannot speak notices is not a control ring.
    let control = match Adopted::at(
        CONTROL_AT,
        REGION_BYTES,
        feature::CONTROL_EVENTS,
        feature::CONTROL_EVENTS,
    ) {
        Ok(bound) => bound.client(),
        Err(refusal) => end(refused(report::NO_CONTROL, refusal)),
    };

    // This runtime's own queue, adopted twice. An executor is both ends of it,
    // and saying so in two values is cheaper than a comment claiming it.
    let Ok(submit) = Adopted::at(WORK_AT, REGION_BYTES, 0, 0) else {
        end(refused(report::NO_WORK, 0))
    };
    let Ok(execute) = Adopted::at(WORK_AT, REGION_BYTES, 0, 0) else {
        end(refused(report::NO_WORK, 0))
    };
    let (submit, execute) = (submit.client(), execute.server());

    let mut tally = Tally::default();
    let mut submitted: u64 = 0;

    while tally.completed < report::LOAD {
        // ------------------------------------------- the allocation boundary
        //
        // Every notice this component will ever receive arrives here, as a
        // completion entry carrying the notice flag, drained because this
        // component chose to drain it. There is no handler, no interrupted
        // instruction stream and no second path in.
        match drain(&control, &mut tally) {
            Ok(false) => {}
            // A reclaim. Take no new work; what is already on the ring below
            // still gets finished, because *cleanly* is about the queue and not
            // about the clock.
            Ok(true) => {
                tally.flags |= report::RECLAIMED;
                break;
            }
            Err(code) => end(stopped(code, &tally)),
        }

        // The one deliberate crossing, and only under the selector that asks
        // for it. It sits inside the work loop rather than beside it, because a
        // provocation outside the window the frame is counting would move no
        // counter and prove nothing.
        if selector == report::PROVOKE && tally.completed == report::QUANTUM {
            let _ = door::call(NOTHING, 0, 0);
            tally.provoked += 1;
        }

        // ------------------------------------------------------- one quantum
        let mut staged = 0;
        while staged < report::QUANTUM && tally.completed + staged < report::LOAD {
            let entry = Sqe { user_data: submitted, ..Sqe::ZERO };
            match submit.submit(entry) {
                Ok(_) => {}
                // A full ring at a quantum of half its depth means the executor
                // below did not run, which cannot happen — so it is a refusal
                // rather than a retry.
                Err(_) => end(stopped(report::RING_REFUSED, &tally)),
            }
            submitted += 1;
            staged += 1;
        }

        let mut executed = 0;
        while executed < staged {
            match execute.pop() {
                Ok(Some(task)) => {
                    // Room to answer before taking the question, which is the
                    // one rule `f_ring::Service::drain` states and the one
                    // failure a ring must not have.
                    match execute.free() {
                        Ok(0) | Err(_) => end(stopped(report::RING_REFUSED, &tally)),
                        Ok(_) => {}
                    }
                    let answer = f_ring::completion(task.user_data, 0, 0);
                    if execute.post(answer).is_err() {
                        end(stopped(report::RING_REFUSED, &tally));
                    }
                    executed += 1;
                }
                Ok(None) | Err(_) => end(stopped(report::RING_REFUSED, &tally)),
            }
        }

        let mut reaped = 0;
        while reaped < executed {
            match submit.take() {
                Ok(Some(_)) => reaped += 1,
                Ok(None) | Err(_) => end(stopped(report::RING_REFUSED, &tally)),
            }
        }
        tally.completed += reaped;
    }

    tally.parked = report::LOAD - tally.completed;
    // Quiescent, and it is arithmetic rather than a claim: everything this
    // runtime submitted came back. A ring occupancy read beside it would be a
    // second reading of the same fact, and the reading that is *not* this
    // component's own is the one that matters — the frame drains both halves of
    // this ring after the run and requires them empty, which is a number taken
    // on the other side of the boundary rather than a claim taken on this one.
    if u64::from(tally.completed) == submitted {
        tally.flags |= report::QUIESCENT;
    } else {
        tally.code = report::NOT_QUIET;
    }
    end(report::pack(tally))
}

/// Drain the control ring, and answer whether a core is being taken back.
///
/// # Errors
///
/// A [`report`] code. Every one of them is a frame bug rather than this
/// component's: R04 does not permit a component to skip an entry it cannot
/// name, so meeting one is a reason to stop rather than to carry on.
fn drain(control: &f_ring::Client, tally: &mut Tally) -> Result<bool, u8> {
    let mut reclaim = false;
    loop {
        let entry = match control.take() {
            Ok(Some(entry)) => entry,
            Ok(None) => return Ok(reclaim),
            Err(RingError::Full | RingError::Corrupt | RingError::EpochChanged) => {
                return Err(report::RING_REFUSED);
            }
        };
        tally.notices += 1;
        if !is_notice(&entry) {
            return Err(report::STRAY_COMPLETION);
        }
        if !notice::known(entry.result) {
            return Err(report::UNKNOWN_NOTICE);
        }
        // One notice per core, never one for several — so a runtime holding
        // four cores may be parking three of them against three deadlines and
        // must not treat the newest as the only one. This runtime holds one, so
        // the general shape is a flag; the entry's `ext` carries the core and
        // the deadline for the day it holds more.
        if entry.result == notice::RECLAIM {
            reclaim = true;
        }
        let _: Cqe = entry;
    }
}

/// The status a run that could not adopt reports.
///
/// The refusal travels in the two fields a failing run has no other use for —
/// [`report::refusal`] says which and why both are needed — and it is a
/// refusal rather than a silence because RFC 0010 asks a caller to handle
/// refusals as ordinary control flow, which a caller that cannot see the code
/// cannot do.
///
/// It is carried as a domain and a reason rather than as the packed `i32`,
/// because the packed form is a *negated* pair: its low sixteen bits are the
/// two's complement of the reason, so a field holding it verbatim turns
/// `ARGUMENT/MALFORMED_HEADER` into `0xFFFF`. Unpacking first is what makes the
/// number the frame reads the number this component meant.
const fn refused(code: u8, refusal: i32) -> u64 {
    match f_abi::error::unpack(refusal) {
        Some((domain, reason)) => report::pack(report::refusal(code, domain, reason)),
        // A non-negative value is not a refusal, and there is no honest way to
        // report one here. The zero pair, which the frame reads as a refusal
        // with no reason given and no domain it can name.
        None => report::pack(report::refusal(code, 0, 0)),
    }
}

/// The status a run that stopped mid-loop reports: what it had done, and why it
/// stopped.
const fn stopped(code: u8, tally: &Tally) -> u64 {
    report::pack(Tally { code, ..*tally })
}

/// End, and do not come back.
///
/// **This is the allocation boundary and the only boundary crossing this
/// component makes on purpose.** It is excluded from the hot-path count for
/// exactly that reason and counted separately, so the exclusion is a number a
/// reader can see rather than a sentence they have to trust.
fn end(status: u64) -> ! {
    let _ = door::call(door::EXIT, status, 0);
    park()
}

/// Stop, without ending. Reached only where continuing would be worse.
fn park() -> ! {
    loop {
        core::hint::spin_loop();
    }
}
