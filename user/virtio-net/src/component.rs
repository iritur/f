// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `virtio-net` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to, and the polling loop that serves its
//! client from ring 3.
//!
//! # Why there is no attribute on [`start`]
//!
//! Because there cannot be, and `user/init/src/component.rs` records the scar at
//! length: naming an entry point means `#[unsafe(no_mangle)]` or
//! `#[unsafe(link_section)]`, both of which are unsafe attributes in this
//! edition, and a crate that forbids unsafe code cannot write one. So
//! `user/init/link.ld` places the section this function is compiled into at the
//! image's first byte, and `cargo xtask component` checks that the symbol which
//! actually landed there is this one. The path `component::start` is now
//! load-bearing across four crates: one linker script, one placement rule, one
//! check.
//!
//! # What is the same as the block driver's loop, and the one thing that is not
//!
//! The frame is the same. Read the routing page, refuse it if the frame did not
//! speak, bring the device up, run the zero-copy self-check, adopt two rings in
//! safe code (`f_ring::adopt`, RFC 0037), drain the control ring first because a
//! stop is the one thing that ends the loop, drain the data ring, and report
//! into the half of the board that is this component's before ending.
//!
//! The difference is a second thing to poll, and it is not a refinement. A block
//! driver's loop has one source of work: the client's ring. If it is empty there
//! is nothing to do and the loop spins. This one has two, and the second is a
//! **device** — a frame arrives on the link when a peer sends one, with no entry
//! on any ring and nothing in this component having asked. So every turn calls
//! [`Driver::collect`](crate::driver::Driver::collect) whether or not the client
//! submitted anything, and an idle loop here is a loop that is still doing the
//! useful half of its job.
//!
//! That is R05 holding rather than bending: *nothing is delivered
//! asynchronously, every event is a ring entry drained at a polling point*. A
//! packet is not delivered to this component either — it lands in a client's
//! buffer, and this component *notices* at a polling point of its own choosing
//! and posts a completion. The device writes; the driver polls; the client is
//! told on a ring. There is no callback anywhere and there is no path in.
//!
//! # The bound on waiting, and why it is told rather than chosen
//!
//! [`routing::at::RECEIVE_SPINS`] is the number of turns this loop will spend
//! with nothing arriving before it stops. It exists because the block driver's
//! bounds do not generalise: every one of those waits for an answer a device
//! owes, and a receive queue owes nothing. A driver with no interrupt and no
//! bound on that wait is a driver that hangs, and calling the hang *waiting for
//! traffic* would be the exact failure `docs/rfc/0046` names — a hang is a count.
//!
//! # What it does *not* do, said rather than implied
//!
//! It is not spawned into the place `kernel/src/component.rs` builds for it. The
//! frame stands this instance up the way it stands the block driver up — image,
//! account-less, needs unchecked — because the supervisor that would hand a
//! *place's* occupant a core is the ring-3 supervisor E1-B05 still owes.
//! `CHAOS_GAP` in xtask carries that difference and it is unchanged by this
//! task: a second driver in the same position widens nothing.

use f_abi::control::{is_notice, notice};
use f_abi::deadline::Admitted;
use f_abi::{Cqe, Negotiated, Sqe, door, error, feature};
use f_ring::adopt::{Adopted, Client};
use f_ring::device::{Region, Window};
use f_ring::registry::{Domains, Refusal};

use crate::driver::{Admission, Answered};
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
    // Which of this component's lives the frame asked for. A selector this build
    // does not name falls through to the announcement rather than inventing a
    // fourth, which is what a *spawn* into a place still asks for.
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

/// Serve the data ring and the device until the frame says stop.
///
/// Every failure here ends the run with a reason in [`reported::OUTCOME`] rather
/// than a panic, and the reason matters: a component that stopped because its
/// routing page was blank and one that stopped because it was told to look
/// identical from outside, and only one of them is the run the boot asked for.
fn serve(selector: u32) -> ! {
    let Ok(board) = Window::at(routing::AT, routing::BYTES) else {
        // Nothing to report *into*, so the status word is all there is.
        end(stopped::BAD_ROUTING)
    };
    // R04 at the one place this component reads a structure it did not build. A
    // page of zeroes is what a frame that was mapped and never filled in looks
    // like, and a zero length taken for a length reads as a device problem
    // rather than as a frame that did not speak.
    //
    // And a page carrying the *other* driver's magic is refused here too, which
    // is the whole reason the two constants differ: one driver shape means both
    // components read their board at the same address, so the magic is the only
    // thing that says which board this is.
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        report(&board, None, 0, stopped::NO_ROUTING);
        end(stopped::NO_ROUTING)
    }

    let Some(parts) = laid_out(&board) else {
        report(&board, None, 0, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    };

    let Ok(mut driver) =
        crate::driver::Driver::start(parts.windows, parts.queues, parts.agreed, parts.admission)
    else {
        report(&board, None, 0, stopped::NO_DEVICE);
        end(stopped::NO_DEVICE)
    };

    // The self-check that makes the published zero worth reading, run before the
    // data path so that a build in which it silently did nothing fails rather
    // than being hidden by a datapath that also did nothing. It is the one call
    // in this crate that moves bytes, and `cargo xtask lint-datapath` is what
    // keeps that true.
    if driver.provoke_copy().is_err() {
        report(&board, Some(&driver), 0, stopped::NO_SELF_CHECK);
        end(stopped::NO_SELF_CHECK)
    }

    let mut route = Route { control: parts.control, token: 0, told: false };
    let mut drained: u64 = 0;
    // Turns with nothing on the client's ring and nothing from the device. Reset
    // by *either* — a loop that only counted the ring would stop while frames
    // were still arriving, and one that only counted the device would never stop
    // at all.
    let mut idle: u64 = 0;
    let outcome = loop {
        // The control ring first, because a stop is the one thing that ends this
        // loop and work taken after it would be work done for a client the frame
        // has already told this component it no longer has.
        match route.drain(0) {
            Ok(_) => {}
            Err(()) => break stopped::NO_RING,
        }
        if route.told {
            break stopped::TOLD;
        }

        let mut busy = false;

        // The device, before the client, and the order is a decision. A frame
        // the device has already written is sitting in a client's buffer with
        // the client unable to touch it — RFC 0024's `InFlight` has no method
        // that reaches its bytes — so every turn this side spends elsewhere is a
        // turn a client waits for memory it already owns. There is no such
        // asymmetry on the transmit side, where the client is waiting for
        // nothing it can use.
        match driver.collect(0) {
            Ok(Some(answer)) => {
                busy = true;
                if parts.data.post(answer).is_err() {
                    break stopped::NO_RING;
                }
            }
            Ok(None) => {}
            // A device naming a chain this driver never posted. Its own outcome,
            // because it is a device steering the driver rather than a device
            // failing to start, and the two want different answers from whoever
            // reads the log.
            Err(_) => break stopped::BAD_DEVICE,
        }

        // Then the client. One entry per turn rather than a drain, so that a
        // client submitting continuously cannot starve the receive path above —
        // which is a starvation a block driver cannot have, because its only
        // source of work is the client that would be doing the starving.
        let taken = match parts.data.pop() {
            Ok(taken) => taken,
            Err(_) => break stopped::NO_RING,
        };
        if let Some(entry) = taken {
            busy = true;
            drained += 1;
            // Zero, and it is a literal for `Driver::execute`'s reason: this
            // crate observes no clock. `DEADLINE_GAP` in xtask is what goes red
            // the day this stops being a literal.
            match driver.admit(&entry, 0) {
                Err(cqe) => {
                    if parts.data.post(cqe).is_err() {
                        break stopped::NO_RING;
                    }
                }
                Ok(order) => {
                    // Two entry points and not a flag, so the provocation is
                    // greppable: the data path calls `execute`, and only the
                    // escape life reaches `provoke_escape`.
                    //
                    // **And only on a receive**, which is not fussiness. The
                    // transmit is the positive control — it is what causes a
                    // frame to come back at all — and a run in which it also
                    // escaped would compare an empty sink against a link nothing
                    // was ever put on: *nothing arrived* for a reason that has
                    // nothing to do with the provocation being refused. That is
                    // the green-for-the-wrong-reason this epoch has recorded
                    // four times.
                    let bend = selector == life::ESCAPE && entry.opcode == crate::driver::op::RECV;
                    let answered = if bend {
                        driver.provoke_escape(&entry, order, &mut route, 0, parts.beyond)
                    } else {
                        driver.execute(&entry, order, &mut route, 0)
                    };
                    if let Answered::Now(cqe) = answered
                        && parts.data.post(cqe).is_err()
                    {
                        break stopped::NO_RING;
                    }
                    // Asked after the completion is posted, because a client
                    // owed a refusal is owed it whether or not this component
                    // is ending. A transfer can fail *after* its buffer is with
                    // the device — a doorbell that could not be rung on an
                    // offered chain, a frame the device never took — and there
                    // is no refusal that answers that: a refusal hands the
                    // client back an `Idle` it may write while a network card
                    // holds a descriptor into it. So the driver puts the device
                    // in reset instead and says so here, and the teardown below
                    // gives the buffer back as a cancellation, which is the one
                    // exit RFC 0024 leaves.
                    if driver.stopped() {
                        break stopped::DEVICE_HOLDS;
                    }
                }
            }
        }

        if busy {
            idle = 0;
            continue;
        }
        idle += 1;
        if idle > parts.spins {
            // Not a failure and not `TOLD`: the run did what it was asked and
            // nothing more is coming. A component that spun here forever would
            // be a hang, and `docs/rfc/0046` says a hang is a count — so it is
            // counted, and `Counters::spun` publishes how much of the count was
            // spent so a reader can see the bound and its use together.
            break stopped::TOLD;
        }
        core::hint::spin_loop();
    };

    // Told to stop, or stopping because something stopped making sense. The
    // teardown is two steps and the order between them is load-bearing in a way
    // the block driver's teardown is not.
    //
    // First the device goes back into reset, and for a network device that is
    // not a tidiness measure: a device left holding posted receive buffers
    // writes into them the next time anything arrives on the link, which is a
    // thing no code in this system decides.
    let _ = driver.quiesce();
    // Only then are the buffers it was holding given back, as cancellations.
    // `Driver::cancel` argues why this exists at all — a posted receive is a
    // buffer with no answer owed, and a client left holding an `InFlight` for
    // one has none of RFC 0024's three exits — and the order is why it is here
    // rather than before the reset: *given back* means the client may write it,
    // and a live device pointed at it would still be writing.
    //
    // A ring with no room is not an error to stop on: the run is already over,
    // and the alternative to giving up here is a component that cannot end.
    // What it costs is written down in `Counters::cancelled`, which counts the
    // buffer as given back — so a run whose ring was full would show a
    // cancellation this loop could not deliver, which is the honest direction to
    // be wrong in.
    while let Some(answer) = driver.cancel(0) {
        if parts.data.post(answer).is_err() {
            break;
        }
    }
    report(&board, Some(&driver), drained, outcome);
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
    admission: Admission,
    /// Turns with nothing happening before the loop ends. Unit: turns.
    spins: u64,
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

    // A bound of zero is a driver that gives up before it has looked once, which
    // is a routing page this build cannot honour rather than a frame that meant
    // *do not wait*. Refused here, where it is a layout, rather than discovered
    // as a run that received nothing.
    let spins = board.read64(at::RECEIVE_SPINS).ok()?;
    if spins == 0 {
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
        admission,
        spins,
    })
}

/// Write what this component did into the half of the routing page that is its
/// own.
///
/// The magic goes last, which is the whole of the discipline: a frame that reads
/// a page this function never finished finds a zero rather than a plausible
/// tally. RFC 0013's *read, never delivered* — the frame takes these numbers out
/// of memory it granted, and this component is never asked for them.
fn report(board: &Window, driver: Option<&crate::driver::Driver>, drained: u64, outcome: u64) {
    if let Some(driver) = driver {
        let counters = driver.counters();
        let _ = board.write64(reported::SERVED, u64::from(counters.served));
        let _ = board.write64(reported::REFUSED, u64::from(counters.refused));
        let _ = board.write64(reported::BYTES, counters.bytes);
        let _ = board.write64(reported::COPIES, counters.copies);
        let _ = board.write64(reported::ESCAPED, u64::from(counters.escaped));
        let _ = board.write64(reported::PROVOKED, counters.provoked);
        let _ = board.write64(reported::SHORTFALL, u64::from(counters.shortfall));
        let _ = board.write64(reported::UNADMITTED, u64::from(counters.unadmitted));
        let _ = board.write64(reported::SENT, u64::from(counters.sent));
        let _ = board.write64(reported::RECEIVED, u64::from(counters.received));
        let _ = board.write64(reported::POSTED, u64::from(counters.posted));
        let _ = board.write64(reported::SPUN, counters.spun);
        let _ = board.write64(reported::CANCELLED, u64::from(counters.cancelled));
        let _ = board.write64(reported::HALTED, u64::from(counters.halted));
    }
    let _ = board.write64(reported::DRAINED, drained);
    let _ = board.write64(reported::OUTCOME, outcome);
    let _ = board.write64(reported::MAGIC, routing::MAGIC);
}

/// This component's end of the control ring, and the one thing it asks the frame
/// for.
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
    /// Whether a stop notice has arrived. Once true it stays true.
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
            // rather than mistaken for the answer, because the token is what says
            // which answer this is.
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
        // being gone — and on this driver that matters at a moment nothing chose,
        // because a receive buffer the device still holds is written to when a
        // packet arrives rather than when anything here asks.
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
/// way it notices anything else — the component stops making progress and its
/// supervisor's stop deadline passes. Its manifest then restarts it, which is
/// what `restart.policy = "on_fault"` is for.
#[cfg(not(test))]
#[panic_handler]
fn panicked(_: &core::panic::PanicInfo) -> ! {
    park()
}
