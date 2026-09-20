// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `virtio-blk` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to, and — since RFC 0047 — the polling
//! loop that serves its client from ring 3.
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
//! # What changed, and what the sentence here used to say
//!
//! It used to say that this file is three lines and the driver is two thousand,
//! because *serving means draining a ring, draining a ring means adopting a
//! mapped channel, and `f_ring::Mapping::adopt` is `unsafe`*. That stopped
//! being true twice. RFC 0037 made a channel adoptable in safe code, and RFC
//! 0047 gave a scheduled driver the two things it still lacked: more than one
//! page of text, and a route by which it asks the frame for a device
//! translation. So the loop is here now, and `kernel/src/blk.rs` no longer
//! calls `Driver::execute` — which is RFC 0033's own reversal, stated as a grep
//! anybody can run.
//!
//! # The three things this component cannot do for itself, and what it does
//!
//! **It cannot program a remapping unit.** The unit is the frame's, its page
//! tables are the frame's, and a component that could program one could point
//! any device at any memory. So [`Route`] asks, over the control ring —
//! `f_abi::control::op::DEVICE_MAP` — and the answer is an address or the
//! refusal the frame's own check produced. That check is unchanged from when
//! the frame called this code directly: the client's handle, resolved against
//! the client's table, refused without `GRANT`.
//!
//! **It cannot find out where anything is.** A device's register structures are
//! at offsets the *device* publishes and its queue memory has a device address
//! a *translation* answered, so both are read out of [`crate::routing`] rather
//! than assumed. One address is a constant on both sides and everything else is
//! data.
//!
//! **It cannot decide when to stop.** RFC 0008 says a component ends when its
//! supervisor says so, and this one ends on a `STOP` notice drained at the same
//! polling point as everything else — R05, there is no second path in.
//!
//! # Which instance serves, and on which boot
//!
//! Two, and which one a boot uses is a parameter rather than a design. On the
//! three `blk=` provocation halves and the three `deadline=` halves, the frame
//! stands the serving instance up the way `kernel/src/runtime.rs` stands a
//! runtime up — image, account-less, needs unchecked — and points a client at
//! it. On `blk=place` and `blk=served` the instance is the occupant of the place
//! `kernel/src/component.rs` builds from this manifest, and the difference
//! between those two is the life it is entered at.
//!
//! `blk=place` supplies that place with the device window it cannot carve, tells
//! it where the device's four register structures are on the routing page its
//! manifest declares a `board` need for, and hands it a core to *read* with: it
//! enters at [`start`] with [`crate::routing::life::IDENTIFY`], reads the disk's
//! capacity out of the device's own configuration structure, writes it back onto
//! the board, and ends.
//!
//! `blk=served` supplies the same place, fills in all twenty-eight routing slots
//! rather than the twelve an identify life reads, maps it a data ring out of its
//! own account from the `data` need this manifest declares, and enters it at
//! [`crate::routing::life::SERVE`] on a core the frame does **not** wait for.
//! The frame is the client on its own core for the length of that run, and
//! answers this component's translation requests on the control ring while it
//! waits — `smp::start_on` and `smp::join_serviced`, which is what *serving*
//! means and what the run-to-completion path could not do.
//!
//! # What that leaves, said rather than implied
//!
//! So all three of the sentences `CHAOS_GAP` names are supported by this
//! component: *the code that serves the datapath runs at ring 3 in its own
//! loop*, *a place's occupant reads the device through the window its place was
//! supplied with*, and *the occupant of a place serves a client concurrently*.
//! What is not yet true is the sentence about the *gate*: `cargo xtask blk` with
//! no argument still runs its three halves on the account-less instance, and
//! `CHAOS_GAP`'s needle is the call that stands that one up. It goes when the
//! provocations move onto the place path and survive being killed there, which
//! is `E1-P06`.

use f_abi::control::{is_notice, notice};
use f_abi::deadline::Admitted;
use f_abi::{Cqe, Negotiated, Sqe, door, error, feature};
use f_ring::adopt::{Adopted, Client};
use f_ring::device::{Granted, Region, Window};
use f_ring::refusal;
use f_ring::registry::{Domains, Refusal};

use crate::pending::{Admission, Order, Pending};
use crate::routing::{self, at, life, reported, stopped};
use crate::transport::Windows;

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
    // Which of this component's lives the frame asked for. A selector this
    // build does not name falls through to the announcement rather than
    // inventing a fourth, which is what a *spawn* into a place still asks for.
    let selector = entry.selector();
    if selector == life::SERVE || selector == life::ESCAPE {
        serve(selector)
    }
    if selector == life::IDENTIFY {
        identify()
    }

    // The frame tells a component what it holds rather than letting it assume,
    // and `door::Entry` argues why: a second occupant of a place finds its
    // capabilities at the same indices and a later generation. For this
    // component the order is the manifest's — the four register frames, the
    // untyped region for its queues, its interrupt, its powerbox endpoint.
    let _ = entry.granted(0);

    // "I am here." The one thing the frame cannot observe from outside.
    let _ = door::call0(door::ANNOUNCE);

    end(DONE)
}

/// Serve the data ring until the frame says stop.
///
/// Every failure here ends the run with a reason in [`reported::OUTCOME`]
/// rather than a panic, and the reason matters: a component that stopped
/// because its routing page was blank and one that stopped because it was told
/// to look identical from outside, and only one of them is the run the boot
/// asked for.
fn serve(selector: u32) -> ! {
    let Ok(board) = Window::at(routing::AT, routing::BYTES) else {
        // Nothing to report *into*, so the status word is all there is.
        end(stopped::BAD_ROUTING)
    };
    // R04 at the one place this component reads a structure it did not build.
    // A page of zeroes is what a frame that was mapped and never filled in
    // looks like, and a zero length taken for a length reads as a device
    // problem rather than as a frame that did not speak.
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        report(&board, None, None, 0, stopped::NO_ROUTING);
        end(stopped::NO_ROUTING)
    }

    let Some(mut parts) = laid_out(&board) else {
        report(&board, None, None, 0, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    };

    let Ok(mut driver) =
        crate::driver::Driver::start(parts.windows, parts.queues, parts.agreed, parts.admission)
    else {
        report(&board, None, None, 0, stopped::NO_DEVICE);
        end(stopped::NO_DEVICE)
    };

    // The self-check that makes the published zero worth reading, run before
    // the data path so that a build in which it silently did nothing fails
    // rather than being hidden by a transfer that also did nothing. It is the
    // one call in this crate that moves bytes, and `cargo xtask lint-datapath`
    // is what keeps that true.
    if driver.provoke_copy().is_err() {
        report(&board, Some(&driver), None, 0, stopped::NO_SELF_CHECK);
        end(stopped::NO_SELF_CHECK)
    }

    let mut route = Route { control: parts.control, token: 0, told: false };

    // --- what this instance inherited, replayed before it serves anybody -----
    //
    // **The incoming half of RFC 0063's phase A**, and the ordering is the whole
    // of it: before the first entry is taken off any ring, because a client
    // whose `SetId` resolved against an empty table would be told `NO_SUCH_CAP`
    // for a registration it never lost.
    //
    // Replayed through this instance's *own* `Driver::execute`, which is why the
    // transfer is a history and not a table. Each deed is re-executed — the
    // frame is asked for its own translations, this table issues its own slots —
    // and what makes the `SetId` a client still holds keep working is that the
    // deeds arrive in the order they were done. A table copied across would have
    // been somebody else's device addresses wearing this instance's slots.
    if parts.replay > 0 {
        let taken = parts
            .window
            .as_ref()
            .map(|window| replay(&mut driver, &mut route, window, parts.replay));
        match taken {
            Some(replayed) => {
                let _ = board.write64(reported::REPLAYED, u64::from(replayed));
                // The count first and the flag second, which is the same
                // discipline `report` keeps with its magic: a frame that read
                // the flag before the number would read whatever the board held
                // before this instance touched it. After this store the frame
                // may acknowledge, and until it does this instance has served
                // nobody.
                let _ = board.write64(reported::REPLAY_DONE, 1);
            }
            // Told there were records and given no window to read them from.
            // Refused rather than served: an instance that carried on would be
            // one whose clients think they inherited a table it never received,
            // which is the `amnesiac` control in `sim/src/swap.rs` arriving as
            // ordinary behaviour.
            None => {
                report(&board, Some(&driver), None, 0, stopped::BAD_ROUTING);
                end(stopped::BAD_ROUTING)
            }
        }
    }
    // What has been taken off the ring and not yet handed to the device. This
    // is the whole of `E1-B06` in this component: the ring is drained into it in
    // arrival order and the device is fed out of it in the order
    // `f_abi::deadline::inherit` decided. RFC 0049.
    let mut queue = Pending::new();
    let mut drained: u64 = 0;
    // Whether the frame's hold has been satisfied once. Once, and then never
    // again: a hold that re-armed would stall on whatever the first pick left
    // behind, and the pick it exists to make deterministic has already happened.
    let mut held_once = false;
    let outcome = loop {
        // The control ring first, because a stop is the one thing that ends
        // this loop and an entry taken after it would be work done for a client
        // the frame has already told this component it no longer has.
        match route.drain(0) {
            Ok(_) => {}
            Err(()) => break stopped::NO_RING,
        }
        if route.told {
            break stopped::TOLD;
        }

        // Take everything the client has published, up to what this queue can
        // hold. Draining before choosing is what makes a choice possible at
        // all: a loop that took one entry and served it has no queue and
        // therefore no order, which is what this driver was until now.
        //
        // Every entry is admitted on the way in — RFC 0025's bound on the
        // caller, answered before the request has a rank at all — so an entry
        // claiming a class its submitter does not hold is refused here and
        // never joins the queue. It is refused *after* being counted as
        // drained, because it did cross the boundary.
        let mut stopping = None;
        while !queue.is_full() {
            let taken = match parts.data.pop() {
                Ok(taken) => taken,
                Err(_) => {
                    stopping = Some(stopped::NO_RING);
                    break;
                }
            };
            let Some(entry) = taken else { break };
            drained += 1;
            // Zero, and it is a literal for `Driver::execute`'s reason: this
            // crate observes no clock. `Admission::floor` states what that
            // costs bound 3, and `DEADLINE_GAP` in xtask is what goes red the
            // day this stops being a literal.
            let admitted = match driver.admit(&entry, 0) {
                Ok(order) => Some(order),
                Err(cqe) => {
                    if parts.data.post(cqe).is_err() {
                        stopping = Some(stopped::NO_RING);
                    }
                    None
                }
            };
            let Some(order) = admitted else {
                if stopping.is_some() {
                    break;
                }
                continue;
            };
            if let Err((packed, detail)) = queue.push(entry, order) {
                // Unreachable while the queue is at least as deep as the
                // client's ring, which `pending::CAPACITY` argues it is.
                // Answered rather than dropped anyway: a request that vanished
                // because something more urgent arrived is a client that waits
                // forever, and a service may not do that quietly.
                if parts.data.post(refusal(entry.user_data, packed, detail, 0)).is_err() {
                    stopping = Some(stopped::NO_RING);
                }
                break;
            }
        }
        if let Some(why) = stopping {
            break why;
        }

        if queue.is_empty() {
            // --- the quiescent point, and the swap is asked for here ---------
            //
            // `user/virtio-blk/manifest.toml`'s `[transfer]` table named this
            // line in advance: *the driver needs a point in its own loop where
            // it holds nothing, and it has one already.* An empty `Pending` is
            // the whole of what this component has accepted and not answered,
            // because `pending::IN_FLIGHT` is one and `Driver::execute` polls
            // its chain back out of the device before it returns.
            //
            // That has a named expiry and it is worth restating where the code
            // relies on it: `E1-B09` waits on the interrupt instead of spinning
            // and raises `IN_FLIGHT` above one, and on that day an empty queue
            // stops meaning an empty driver. The assertion below then grows a
            // second term over the used ring, and this comment is what tells the
            // next reader that the single term was a decision rather than an
            // oversight.
            // **Read here and not at start-up**, which is the whole of what
            // makes it a question rather than a setting. The frame writes this
            // word while this component holds a core — it is the only thing it
            // *can* do, because R05 delivers nothing to a running component — so
            // a value captured when the page was first read is a value that can
            // never become true. The first boot of this path hung for exactly
            // that reason: the frame asked, and the component was still looking
            // at the answer it had read before the question.
            if board.read64(at::HAND_OVER).unwrap_or(0) != 0 {
                break hand_over(&board, &driver, parts.window.as_mut());
            }
            core::hint::spin_loop();
            continue;
        }
        // The frame's hold, and it is a fixture rather than a policy --
        // `routing::at::HOLD` says why at length. It applies to one pick, after
        // the frame's own prelude has been served, and its whole effect is that
        // what is queued at that pick is a fact the boot chose instead of a
        // race between two cores.
        if !held_once && drained > parts.hold_after {
            if (queue.len() as u64) < parts.hold {
                core::hint::spin_loop();
                continue;
            }
            held_once = true;
        }
        let Some(waiting) = queue.take(parts.order) else {
            continue;
        };
        let entry = waiting.entry;

        // Two entry points and not a flag, so the provocation is greppable: the
        // data path calls `execute`, and only the escape life reaches
        // `provoke_escape`.
        //
        // **And only on a read**, which is not fussiness. The write before it is
        // the positive control: it is what puts the pattern on the disk, and a
        // run in which it also escaped would compare a sink against a sector
        // that was never written — *the bytes do not match* for a reason that
        // has nothing to do with the provocation being refused. That is the
        // green-for-the-wrong-reason this epoch has recorded four times, and it
        // is why the frame applied the displacement to exactly one entry when
        // this code ran in the frame.
        let bend = selector == life::ESCAPE && entry.opcode == crate::driver::op::READ;
        let answer = if bend {
            driver.provoke_escape(&entry, waiting.order, &mut route, 0, parts.beyond)
        } else {
            driver.execute(&entry, waiting.order, &mut route, 0)
        };
        if parts.data.post(answer).is_err() {
            break stopped::NO_RING;
        }
    };

    // Told to stop, or stopping because something stopped making sense. Either
    // way the device goes back into reset before this component's memory does,
    // because a device left able to address memory the frame is about to hand
    // to somebody else is the corruption this whole subsystem is about.
    let _ = driver.stop();
    report(&board, Some(&driver), Some((&queue, parts.order)), drained, outcome);
    end(outcome)
}

/// Everything the routing page said, in the types that use it.
struct Parts {
    windows: Windows,
    queues: Region,
    control: Client,
    data: f_ring::adopt::Server,
    agreed: Negotiated,
    /// Unit: bytes.
    beyond: u64,
    /// Which order work is handed to the device in.
    order: Order,
    /// What this component is admitted for and what its channel says about the
    /// peer submitting on it.
    admission: Admission,
    /// How many requests to accumulate before the first choice among them.
    /// Unit: requests.
    hold: u64,
    /// How many to serve before that hold applies. Unit: requests.
    hold_after: u64,
    /// The transfer window, where there is one. `None` for every instance
    /// nobody is swapping, which is every instance in every boot but one.
    window: Option<Granted>,
    /// How many records are waiting in the window for this instance to replay
    /// before it serves anybody. Unit: records.
    replay: u32,
}

/// Which build of this component this is.
///
/// One, or two for the image built with the `successor` feature — the only
/// difference between the two, and `user/virtio-blk/Cargo.toml` argues at length
/// why it is the only one.
const GENERATION: u64 = if cfg!(feature = "successor") { 2 } else { 1 };

/// Replay a predecessor's history into this instance's own table.
///
/// Answers how many deeds were re-executed without refusal. The count is
/// reported and compared against what the outgoing instance said it wrote, which
/// is two tallies of one number with neither derived from the other.
///
/// A record whose check word does not hold is **skipped and not replayed**, and
/// the count is what says so. That is the `garble` control in `sim/src/swap.rs`
/// as ordinary behaviour: a window that did not cross intact is a window this
/// instance may not act on, and acting on part of one would be a table half
/// inherited — which is worse than an empty one, because an empty one is
/// visible to every client at once.
fn replay(
    driver: &mut crate::driver::Driver,
    route: &mut Route,
    window: &Granted,
    records: u32,
) -> u32 {
    let bytes = window.bytes();
    let mut done = 0;
    for index in 0..records as usize {
        let at = index * crate::state::RECORD_BYTES as usize;
        let Some(slice) = bytes.get(at..at + crate::state::RECORD_BYTES as usize) else { break };
        let Ok(eight) = <[u8; crate::state::RECORD_BYTES as usize]>::try_from(slice) else { break };
        let record = crate::state::Record::from_bytes(&eight);
        // A record that did not cross intact is skipped and the count says so.
        // `sim/src/swap.rs`'s `garble` control as ordinary behaviour.
        if !record.intact() {
            continue;
        }
        let Some(entry) = record.replay() else { continue };
        let Ok(order) = driver.admit(&entry, 0) else { continue };
        if !driver.execute(&entry, order, route, 0).is_error() {
            done += 1;
        }
    }
    done
}

/// Write this instance's history into the transfer window and stop.
///
/// The outgoing half, and it answers the `stopped` value the loop breaks with so
/// that the frame can tell the two outcomes apart: a hand-over that happened and
/// one that could not.
///
/// **A journal that overflowed refuses.** The history it would hand on is
/// shorter than the history it lived, so a successor replaying it would believe
/// it inherited a table it did not. Abandoning costs every client its
/// registrations — the place restarts — and costs nothing else, which is
/// strictly better than a successor that is quietly wrong.
fn hand_over(board: &Window, driver: &crate::driver::Driver, window: Option<&mut Granted>) -> u64 {
    if driver.overflowed() {
        let _ = board.write64(reported::QUIESCENT, 1);
        return stopped::CANNOT_HAND_OVER;
    }
    let Some(window) = window else {
        let _ = board.write64(reported::QUIESCENT, 1);
        return stopped::BAD_ROUTING;
    };
    let deeds = driver.deeds();
    let room = window.len() as usize / crate::state::RECORD_BYTES as usize;
    if deeds.len() > room {
        let _ = board.write64(reported::QUIESCENT, 1);
        return stopped::CANNOT_HAND_OVER;
    }
    let bytes = window.bytes_mut();
    let mut written = 0u64;
    for (index, deed) in deeds.iter().enumerate() {
        let at = index * crate::state::RECORD_BYTES as usize;
        let Some(slot) = bytes.get_mut(at..at + crate::state::RECORD_BYTES as usize) else { break };
        slot.copy_from_slice(&deed.to_bytes());
        written += 1;
    }
    // The count last, after the bytes, for the reason every board in this tree
    // writes its magic last: a reader that saw the count before the records
    // would read whatever was in the window before this instance touched it.
    let _ = board.write64(reported::RECORDS, written);
    let _ = board.write64(reported::QUIESCENT, 1);
    stopped::HANDED_OVER
}

/// The four register structures and the notification stride, out of the routing
/// page.
///
/// Its own function because two lives need it and only one of them needs
/// anything else: [`identify`] reads a window and stops, [`serve`] goes on to
/// state a queue, two rings and two ceilings. A copy of these fifteen lines in
/// the short life would be a second reading of the same page that could come to
/// a different answer about where a device's configuration structure is, which
/// is the class of disagreement `Windows` exists to prevent one argument down.
///
/// `None` for any offset that cannot be stated as a window inside the register
/// span, which is a frame that filled this page in wrongly.
fn routed(board: &Window) -> Option<Windows> {
    let registers_at = board.read64(at::REGISTERS_AT).ok()?;
    let registers_len = u32::try_from(board.read64(at::REGISTERS_LEN).ok()?).ok()?;
    let registers = Window::at(registers_at, registers_len).ok()?;

    // Narrowing, never widening: `Window::slice` is always inside the window it
    // came from, so a device that published an implausible offset produces a
    // refusal here rather than a driver reading somebody else's page.
    let structure = |offset: u32, len: u32| -> Option<Window> {
        let at = u32::try_from(board.read64(offset).ok()?).ok()?;
        let bytes = u32::try_from(board.read64(len).ok()?).ok()?;
        registers.slice(at, bytes).ok()
    };
    Some(Windows {
        common: structure(at::COMMON_OFFSET, at::COMMON_LEN)?,
        notify: structure(at::NOTIFY_OFFSET, at::NOTIFY_LEN)?,
        isr: structure(at::ISR_OFFSET, at::ISR_LEN)?,
        config: structure(at::CONFIG_OFFSET, at::CONFIG_LEN)?,
        notify_multiplier: u32::try_from(board.read64(at::NOTIFY_MULTIPLIER).ok()?).ok()?,
    })
}

/// Read the device's configuration window, say what it holds, and end.
///
/// # What a run of this proves, said exactly
///
/// That the register window the frame **supplied** to this component's place —
/// rather than carving it out of the account, which it cannot do for a device —
/// is mapped in this component's own address space, at the address the routing
/// page names, and readable from ring 3. The evidence is a number the device
/// published and this component could not have invented: the disk's capacity in
/// sectors, written into [`reported::CAPACITY`] and compared by the boot
/// against the image it was given.
///
/// # What it does not touch, on purpose
///
/// The device's status register. This life neither resets nor acknowledges
/// anything, so a `SERVE` run in the same boot would find the device exactly as
/// the machine left it. That is what makes this safe to run beside a datapath
/// that is still stood up the old way.
fn identify() -> ! {
    let Ok(board) = Window::at(routing::AT, routing::BYTES) else {
        // Nothing to report *into*, so the status word is all there is.
        end(stopped::BAD_ROUTING)
    };
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        // R04 again, and here it is the whole point: an occupant whose place was
        // built and whose board was never filled in reads a page of zeroes, and
        // a zero capacity taken for a capacity would read as an empty disk.
        let _ = board.write64(reported::OUTCOME, stopped::NO_ROUTING);
        let _ = board.write64(reported::MAGIC, routing::MAGIC);
        end(stopped::NO_ROUTING)
    }
    let outcome = match routed(&board).map(|windows| crate::transport::capacity(&windows.config)) {
        Some(Ok(sectors)) => {
            let _ = board.write64(reported::CAPACITY, sectors);
            stopped::IDENTIFIED
        }
        Some(Err(_)) => stopped::NO_DEVICE,
        None => stopped::BAD_ROUTING,
    };
    let _ = board.write64(reported::OUTCOME, outcome);
    // Last, for [`reported::MAGIC`]'s own reason: a frame reading a page this
    // component never reached finds a zero rather than a plausible tally.
    let _ = board.write64(reported::MAGIC, routing::MAGIC);
    end(DONE)
}

/// Read the routing page and state everything it names.
///
/// `None` for any address that cannot be stated as a window, a region or a
/// channel — which is a frame that filled this page in wrongly, and is refused
/// here rather than dereferenced to find out.
fn laid_out(board: &Window) -> Option<Parts> {
    let windows = routed(board)?;

    let queues = Region::at(
        board.read64(at::QUEUES_AT).ok()?,
        board.read64(at::QUEUES_DEVICE_AT).ok()?,
        u32::try_from(board.read64(at::QUEUES_LEN).ok()?).ok()?,
    )
    .ok()?;

    // The control ring requires the feature that carries notices in both
    // directions, which is the one refusal a control ring depends on: a control
    // ring whose peer cannot speak notices is not a control ring.
    let control = Adopted::at(
        board.read64(at::CONTROL_AT).ok()?,
        u32::try_from(board.read64(at::CONTROL_LEN).ok()?).ok()?,
        feature::CONTROL_EVENTS,
        feature::CONTROL_EVENTS,
    )
    .ok()?
    .client();

    let data = Adopted::at(
        board.read64(at::DATA_AT).ok()?,
        u32::try_from(board.read64(at::DATA_LEN).ok()?).ok()?,
        0,
        0,
    )
    .ok()?
    .server();

    // The two ceilings, refused rather than approximated: `Admitted::new`
    // answers `None` for anything that is not one of the four class ordinals,
    // and a routing page carrying one is a frame that did not speak rather than
    // a frame that meant batch. R04, at the same place the magic is checked.
    let admission = Admission {
        mine: Admitted::new(u16::try_from(board.read64(at::ADMITTED).ok()?).ok()?)?,
        client: Admitted::new(u16::try_from(board.read64(at::CLIENT_ADMITTED).ok()?).ok()?)?,
        floor: board.read64(at::FLOOR).ok()?,
    };
    // A hold deeper than the queue is a hold that can never be satisfied, which
    // is a component that stops serving and looks exactly like one that wedged.
    // Refused here, where it is a routing page this build cannot honour, rather
    // than discovered five seconds later as an unanswered completion.
    let hold = board.read64(at::HOLD).ok()?;
    if hold > crate::pending::CAPACITY as u64 {
        return None;
    }

    Some(Parts {
        windows,
        queues,
        control,
        data,
        agreed: Negotiated {
            version: u32::try_from(board.read64(at::NEGOTIATED_VERSION).ok()?).ok()?,
            features: board.read64(at::NEGOTIATED_FEATURES).ok()?,
        },
        beyond: board.read64(at::BEYOND).ok()?,
        order: Order::from_ordinal(board.read64(at::ORDERING).ok()?),
        admission,
        hold,
        hold_after: board.read64(at::HOLD_AFTER).ok()?,
        // Zero on every page no swap wrote, which reads as *there is no window*
        // rather than as a window at address zero — `Granted::at` refuses that
        // address by name, so the `Option` is the refusal rather than a second
        // test of the same thing.
        window: Granted::at(
            board.read64(at::WINDOW_AT).ok()?,
            u32::try_from(board.read64(at::WINDOW_LEN).ok()?).ok()?,
        )
        .ok(),
        replay: u32::try_from(board.read64(at::REPLAY).ok()?).ok()?,
    })
}

/// Write what this component did into the half of the routing page that is
/// its own.
///
/// The magic goes last, which is the whole of the discipline: a frame that
/// reads a page this function never finished finds a zero rather than a
/// plausible tally. RFC 0013's *read, never delivered* — the frame takes these
/// numbers out of memory it granted, and this component is never asked for
/// them.
/// The ids `user/virtio-blk/manifest.toml`'s `[[state]]` table declares.
///
/// Four leaves under one subtree, and every one of them a count this driver
/// already keeps: nothing here is minted for the tree's benefit. `id = 1` is
/// the `blk` subtree itself and carries no word, which is why the list starts
/// at two.
mod node {
    /// Entries answered without a refusal.
    pub const SERVED: u32 = 2;
    /// Entries refused.
    pub const REFUSED: u32 = 3;
    /// Bytes the device transferred for clients.
    pub const BYTES: u32 = 4;
    /// Bytes this component copied on the data path, which is required zero.
    pub const COPIES: u32 = 5;
}

/// Store this run's counts into the tree the frame mounted for this instance.
///
/// **The address comes off the board and is never assumed.** Only
/// `component::spawn` maps `process::SPAWN_TREE`; the two ring-3 shapes in
/// `kernel/src/process.rs` map the control ring, the board, the registers and
/// the queues and not this page. So `at::TREE_AT` is zero on every boot that is
/// not a place, and a zero means *do not write* rather than *write to zero*.
/// Taking the constant instead would fault at ring 3 on six boots with nothing
/// in the fault naming the cause.
///
/// A tree that will not bind is skipped, and that is not a silent skip: the
/// frame reads this page before this instance's first instruction and again
/// after its core comes back, and an instance that stored nothing leaves the
/// two readings equal. The failure is reported by the reader on the far side
/// rather than claimed by the writer on this one — RFC 0013's arrangement, and
/// `user/store/src/runtime.rs` makes the same argument at length.
///
/// Nothing here is `unsafe` and nothing here could be: `f_abi::state::Writer`
/// is why that type has a write side.
fn publish(tree_at: u64, counters: &crate::driver::Counters) {
    if tree_at == 0 {
        return;
    }
    let Ok(tree) = f_abi::state::Writer::at(tree_at, routing::TREE_BYTES) else { return };
    tree.set(node::SERVED, u64::from(counters.served));
    tree.set(node::REFUSED, u64::from(counters.refused));
    tree.set(node::BYTES, counters.bytes);
    tree.set(node::COPIES, counters.copies);
}

fn report(
    board: &Window,
    driver: Option<&crate::driver::Driver>,
    queue: Option<(&Pending, Order)>,
    drained: u64,
    outcome: u64,
) {
    // Which build this is, written on every report and not only on a swap. A
    // number that appeared only when somebody was looking for it would be a
    // number nobody could use to notice a swap they had not expected.
    let _ = board.write64(reported::GENERATION, GENERATION);
    if let Some(driver) = driver {
        let counters = driver.counters();
        let _ = board.write64(reported::SERVED, u64::from(counters.served));
        let _ = board.write64(reported::REFUSED, u64::from(counters.refused));
        let _ = board.write64(reported::BYTES, counters.bytes);
        let _ = board.write64(reported::COPIES, counters.copies);
        let _ = board.write64(reported::ESCAPED, u64::from(counters.escaped));
        let _ = board.write64(reported::PROVOKED, counters.provoked);
        let _ = board.write64(reported::CAPACITY, driver.capacity());
        let _ = board.write64(reported::SHORTFALL, u64::from(counters.shortfall));
        let _ = board.write64(reported::UNADMITTED, u64::from(counters.unadmitted));
        // The same four counts, into the region the frame mounted under its own
        // root. The board is this component's answer to *what did you do*; the
        // tree is the machine's answer to *what is it running*, and RFC 0013
        // wants the second read rather than delivered. Both, from one set of
        // counters, so the two cannot disagree without one of them being wrong.
        publish(board.read64(at::TREE_AT).unwrap_or(0), &counters);
    }
    if let Some((queue, order)) = queue {
        let _ = board.write64(reported::OVERTAKEN, u64::from(queue.overtaken()));
        let _ = board.write64(reported::QUEUED_MAX, u64::from(queue.deepest()));
        let _ = board.write64(reported::IN_FLIGHT, u64::from(crate::pending::IN_FLIGHT));
        // What this component *did*, and not what it was told to do. The frame
        // wrote the ordinal into the other half of this page and can read it
        // back from there; what it cannot know without being told is whether
        // this component understood it, and a control run that quietly used the
        // ordering would pass every comparison between the two halves.
        let _ = board.write64(
            reported::ORDERED,
            match order {
                Order::Rank => 1,
                Order::Arrival => 0,
            },
        );
    }
    let _ = board.write64(reported::DRAINED, drained);
    let _ = board.write64(reported::OUTCOME, outcome);
    let _ = board.write64(reported::MAGIC, routing::MAGIC);
}

/// This component's end of the control ring, and the one thing it asks the
/// frame for.
///
/// # Why the same object drains notices and waits for an answer
///
/// Because there is one ring and R04 does not let an entry be skipped. A
/// translation request is answered on the same completion ring the frame
/// publishes notices onto, so waiting for the answer means draining whatever is
/// in front of it — and a wait that discarded a stop notice on the way would be
/// a component that had been told to stop and did not know.
struct Route {
    control: Client,
    /// The submitter's value on the next request. Monotonic, so a completion
    /// carrying an older one is an answer to a request this component is no
    /// longer waiting for and is not mistaken for this one.
    /// Unit: none — a token.
    token: u64,
    /// Whether a stop notice has arrived. Once true it stays true: a promise
    /// may only move earlier, and a component that forgot it had been told
    /// would be one that kept serving.
    told: bool,
}

impl Route {
    /// Take completions until the one carrying `awaiting`, or until the ring is
    /// empty when `awaiting` is zero.
    ///
    /// **This is the polling point.** Notices are recorded on the way past;
    /// nothing is discarded, and a kind this build cannot name ends the run
    /// rather than being skipped — R04, and `f_abi::control::notice::known` is
    /// the one list that says which kinds exist.
    ///
    /// # Errors
    ///
    /// The ring stopped validating, or it carried something this component
    /// cannot name. Both mean the peer has stopped speaking, and RFC 0008 says
    /// what happens to a component whose peer has.
    fn drain(&mut self, awaiting: u64) -> Result<Option<Cqe>, ()> {
        loop {
            let taken = match self.control.take() {
                Ok(taken) => taken,
                Err(_) => return Err(()),
            };
            let Some(entry) = taken else {
                if awaiting == 0 {
                    return Ok(None);
                }
                // The frame answers from its own polling loop on another core,
                // so an empty ring is *not yet* rather than *never*.
                core::hint::spin_loop();
                continue;
            };
            if is_notice(&entry) {
                if !notice::known(entry.result) {
                    return Err(());
                }
                if entry.result == notice::STOP {
                    self.told = true;
                }
                continue;
            }
            if entry.user_data == awaiting && awaiting != 0 {
                return Ok(Some(entry));
            }
            // A completion for a request this component is no longer waiting
            // for. There is none in this build; if one arrives it is dropped
            // rather than mistaken for the answer, because the token is what
            // says which answer this is.
        }
    }

    /// Ask the frame for something, and wait for the answer on the same ring.
    fn ask(&mut self, entry: Sqe) -> Result<Cqe, Refusal> {
        let gone = (error::pack(error::PEER, error::peer::GONE), 0);
        self.token = self.token.wrapping_add(1);
        let token = self.token;
        if self.control.submit(Sqe { user_data: token, ..entry }).is_err() {
            return Err(gone);
        }
        match self.drain(token) {
            Ok(Some(answer)) => Ok(answer),
            _ => Err(gone),
        }
    }
}

impl Domains for Route {
    fn map(&mut self, cap: u32, len: u32) -> Result<u64, Refusal> {
        let answer =
            self.ask(Sqe { opcode: f_abi::control::op::DEVICE_MAP, cap, len, ..Sqe::ZERO })?;
        match answer.error() {
            // Passed through unchanged, because a refusal this component
            // invented a code for is a refusal its client cannot act on.
            Some((domain, code)) => Err((error::pack(domain, code), answer.ext)),
            None => Ok(answer.ext),
        }
    }

    fn unmap(&mut self, cap: u32, address: u64, len: u32) {
        // Answered even though it cannot refuse, and waited for. A withdrawal
        // that had not happened yet when the next transfer went out would be a
        // translation still live at the moment `InFlight::reclaim` rests on it
        // being gone.
        let _ = self.ask(Sqe {
            opcode: f_abi::control::op::DEVICE_UNMAP,
            cap,
            len,
            offset: address,
            ..Sqe::ZERO
        });
    }
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
///
/// # Why `target_os = "none"` and not the `image` feature alone
///
/// A `#[panic_handler]` is a lang item and there may be exactly one in a linked
/// artefact. `user/virtio-blk/Cargo.toml` turns the `image` feature off for the
/// one crate that links this as a library — the frame, which has its own — and
/// that worked while every consumer was a bare-metal one. It stops working the
/// moment a **host** crate takes this: cargo unifies features across a
/// workspace build, so `f-virtio-blk` built as a member with its own defaults
/// and taken by `f-sim` without them resolves to the union, and the handler
/// collides with `std`'s. `f-sim` takes this crate for one module —
/// `crate::state`, the state record RFC 0063 says only another build of this
/// component may read — and a harness that swapped one component while writing
/// its own copy of that layout would be the second reader `sim/src/deploy.rs`
/// refuses one file over.
///
/// So the gate is narrowed rather than the feature's polarity flipped. This item
/// belongs to a build that could *be* an image, and a host build never is. It
/// costs nothing: the module around it still compiles for the host and is still
/// linted there, which a `default = []` would have given up.
///
/// *Reversal:* a host target that is also `target_os = "none"`. There is none
/// today and one would break this for the reason the feature alone broke — at
/// which point the answer is the feature's polarity and an explicit
/// `--features image` in `xtask`'s component build.
#[cfg(all(not(test), target_os = "none"))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
