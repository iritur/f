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
//! # What it does *not* do, said rather than implied
//!
//! It is not spawned into the place `kernel/src/component.rs` builds for it.
//! The frame stands this instance up the way `kernel/src/runtime.rs` stands a
//! runtime up — image, account-less, needs unchecked — because the supervisor
//! that would hand a *place's* occupant a core is the ring-3 supervisor E1-B05
//! still owes. So the sentence this component now supports is *the code that
//! serves the datapath runs at ring 3 in its own loop*, and not yet *the
//! occupant of a place serves the datapath*. `CHAOS_GAP` in xtask is what
//! carries the difference, shrunk to exactly that.

use f_abi::control::{is_notice, notice};
use f_abi::deadline::Admitted;
use f_abi::{Cqe, Negotiated, Sqe, door, error, feature};
use f_ring::adopt::{Adopted, Client};
use f_ring::device::{Region, Window};
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

    let Some(parts) = laid_out(&board) else {
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
}

/// Read the routing page and state everything it names.
///
/// `None` for any address that cannot be stated as a window, a region or a
/// channel — which is a frame that filled this page in wrongly, and is refused
/// here rather than dereferenced to find out.
fn laid_out(board: &Window) -> Option<Parts> {
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
    let windows = Windows {
        common: structure(at::COMMON_OFFSET, at::COMMON_LEN)?,
        notify: structure(at::NOTIFY_OFFSET, at::NOTIFY_LEN)?,
        isr: structure(at::ISR_OFFSET, at::ISR_LEN)?,
        config: structure(at::CONFIG_OFFSET, at::CONFIG_LEN)?,
        notify_multiplier: u32::try_from(board.read64(at::NOTIFY_MULTIPLIER).ok()?).ok()?,
    };

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
fn report(
    board: &Window,
    driver: Option<&crate::driver::Driver>,
    queue: Option<(&Pending, Order)>,
    drained: u64,
    outcome: u64,
) {
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
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
