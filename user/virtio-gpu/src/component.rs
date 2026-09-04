// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `virtio-gpu` as something that runs: the body a spawn puts at
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
//! load-bearing across five crates: one linker script, one placement rule, one
//! check.
//!
//! # This loop is the block driver's and not the network driver's
//!
//! Read the ring, admit the entry, answer it, post the completion. There is no
//! second thing to poll — a display sends nothing this component did not ask for
//! — so an idle turn here is a turn with nothing to do, which is what
//! `user/virtio-blk`'s loop also has and what `user/virtio-net`'s deliberately
//! does not.
//!
//! That is the same finding [`crate::driver`]'s module comment makes one layer
//! down, and it is worth making twice because it is the whole of what a third
//! sample buys: RFC 0051's differences were about **receiving** and not about
//! being a second driver. A device that answers what it is asked puts a driver
//! back on the first shape, whatever kind of device it is.
//!
//! # The bound on an idle loop, and why it is a backstop here
//!
//! [`routing::at::IDLE_SPINS`] is the number of turns this loop will spend with
//! nothing arriving before it stops. It exists because RFC 0046 says a hang is a
//! count, and it is *not* the mechanism that ends an ordinary run: the frame's
//! stop notice is. On the network driver the equivalent bound is load-bearing,
//! because nothing owes that driver a packet; here every command is owed an
//! answer and a run that reaches this number is a run where the **frame** stopped
//! serving, which is why it is its own outcome — [`stopped::IDLE`] — and not
//! folded into [`stopped::TOLD`].
//!
//! # What it does *not* do, said rather than implied
//!
//! It is not spawned into the place `kernel/src/component.rs` builds for it. The
//! frame stands this instance up the way it stands the other two up — image,
//! account-less, needs unchecked — because the supervisor that would hand a
//! *place's* occupant a core is the ring-3 supervisor E1-B05 still owes.
//! `CHAOS_GAP` in xtask carries that difference and it is unchanged by this
//! task.
//!
//! **And it does not put the device back in reset when it ends.** That is the
//! one line of teardown a reader will look for and not find, and
//! `crate::transport`'s module comment is the argument: a reset destroys every
//! resource the display holds and blanks the screen, so a display driver whose
//! last act is a reset throws away the thing it was asked to produce. What takes
//! the device's access to memory away is the frame — `kernel/src/gpu.rs` clears
//! the bus-master bit and detaches the function from its domain — and what makes
//! that sufficient is that a display controller does nothing until it is told.

use f_abi::control::{is_notice, notice};
use f_abi::deadline::Admitted;
use f_abi::{Cqe, Negotiated, Sqe, door, error, feature};
use f_ring::adopt::{Adopted, Client};
use f_ring::device::{Region, Window};
use f_ring::registry::{Domains, Refusal};

use crate::driver::Admission;
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
    // untyped region for its queue, its interrupt, its powerbox endpoint.
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
    // And a page carrying one of the *other two* drivers' magics is refused here
    // too, which is the whole reason the three constants differ: one driver
    // shape means every component reads its board at the same address, so the
    // magic is the only thing that says which board this is.
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

        let taken = match parts.data.pop() {
            Ok(taken) => taken,
            Err(_) => break stopped::NO_RING,
        };
        let Some(entry) = taken else {
            idle += 1;
            if idle > parts.spins {
                break stopped::IDLE;
            }
            driver.spun();
            core::hint::spin_loop();
            continue;
        };
        idle = 0;
        drained += 1;

        // Zero, and it is a literal for `Driver::execute`'s reason: this crate
        // observes no clock. `DEADLINE_GAP` in xtask is what goes red the day
        // this stops being a literal.
        match driver.admit(&entry, 0) {
            Err(cqe) => {
                if parts.data.post(cqe).is_err() {
                    break stopped::NO_RING;
                }
            }
            Ok(order) => {
                // Two entry points and not a flag, so the provocation is
                // greppable: the data path calls `execute`, and only the escape
                // life reaches `provoke_escape`.
                //
                // Unlike the network driver there is no second opcode to
                // restrict the provocation to. This protocol has one operation
                // and it is the one that hands the device an address, so the
                // escape life bends every entry it serves — which is also why
                // the control for it is a *client* that asks for nothing rather
                // than a driver with a mode.
                let cqe = if selector == life::ESCAPE {
                    driver.provoke_escape(&entry, order, &mut route, 0, parts.beyond)
                } else {
                    driver.execute(&entry, order, &mut route, 0)
                };
                if parts.data.post(cqe).is_err() {
                    break stopped::NO_RING;
                }
                // Asked after the completion is posted, because a client owed a
                // refusal is owed it whether or not this component is ending. A
                // display command can fail while the device is still holding a
                // client's buffer as the backing of a resource, and the only way
                // out of that is the reset `Driver::sequence` performs — which
                // blanks the screen and leaves this driver's resource
                // identifiers naming nothing, so there is nothing sensible to
                // serve afterwards.
                if driver.stopped() {
                    break stopped::DEVICE_HOLDS;
                }
            }
        }
    };

    // No teardown of the device, and the absence is the point — the module
    // comment and `crate::transport`'s argue it. What is torn down is nothing at
    // all: this driver holds no client buffer between entries, because
    // `Driver::sequence` detaches the backing before it answers, so unlike
    // `user/virtio-net` there is no obligation left to discharge here.
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
    /// Turns with nothing on either ring before the loop ends. Unit: turns.
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
    // as a run that served nothing.
    let spins = board.read64(at::IDLE_SPINS).ok()?;
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
        let _ = board.write64(reported::SHOWN, u64::from(counters.shown));
        let _ = board.write64(reported::COMMANDS, u64::from(counters.commands));
        let _ = board.write64(reported::DECLINED, u64::from(counters.declined));
        let _ = board.write64(reported::RESOURCES, u64::from(counters.resources));
        let _ = board.write64(reported::SPUN, counters.spun);
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
        // that had not happened yet when the next command went out would be a
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
