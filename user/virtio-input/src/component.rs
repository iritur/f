// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `virtio-input` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to, and the polling loop that drains a
//! device from ring 3 and submits what it found.
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
//! load-bearing across seven crates: one linker script, one placement rule, one
//! check.
//!
//! # This loop is neither of the two shapes that came before it
//!
//! The block and display drivers' loops read a *request* off a ring, do it, and
//! answer. The network driver's has a second thing to poll — a receive queue
//! nothing asked for — alongside the requests it answers. This one has **only**
//! the second half: there is no request, there is no answer, and there is
//! nothing on the data ring to read, because this component holds the client's
//! end of it and the entries go the other way.
//!
//! So an idle turn here is the ordinary resting state of a machine whose user is
//! not touching it, and [`routing::at::IDLE_SPINS`] is load-bearing rather than
//! a backstop: it is the whole of what ends a run that is not stopped. The
//! display driver's equivalent bound means *the frame stopped serving*, which is
//! a failure; this one means *nobody typed anything*, which is Tuesday.
//!
//! # The control ring carries notices and nothing else
//!
//! The three drivers before this one use their control ring for RFC 0047's
//! `DEVICE_MAP`: they hold a *client's* buffer and must ask the frame to
//! translate it, because a component may not grant itself a device address. This
//! driver holds no client buffer. The only memory the device ever touches is the
//! queue region the manifest routed, and the frame translated that before the
//! first instruction and wrote the answer at
//! [`routing::at::QUEUES_DEVICE_AT`].
//!
//! So [`Route`] here drains notices and does not ask for anything, and this
//! component's run publishes a zero where the other three publish RFC 0047's
//! evidence that the translation route was used. `kernel/src/input.rs` says so
//! rather than reading that zero as a failure — a verdict that required a
//! `DEVICE_MAP` from a driver with nothing to map would be a check passing on
//! three datapaths for a reason that is not the property it names.
//!
//! # What it does *not* do, said rather than implied
//!
//! It is not spawned into the place `kernel/src/component.rs` builds for it. The
//! frame stands this instance up the way it stands the other three up — image,
//! account-less, needs unchecked — because the supervisor that would hand a
//! *place's* occupant a core is the ring-3 supervisor `E1-B05` still owes.
//! `CHAOS_GAP` in xtask carries that difference and it is unchanged by this task.
//!
//! **And it does put the device back in reset when it ends**, which is the
//! opposite of the display driver and for the reason
//! `crate::transport::Transport::reset` gives: what a running virtio-input
//! device does is write into this component's buffers whenever the user acts,
//! and the frame is about to take that memory back.

use f_abi::control::{is_notice, notice};
use f_abi::door;
use f_ring::adopt::{Adopted, Client};
use f_ring::device::{Region, Window};

use crate::clock::Interrupt;
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
    // third, which is what a *spawn* into a place still asks for.
    if entry.selector() == life::SERVE {
        serve()
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

/// Drain the device and submit what it reports until the frame says stop.
///
/// Every failure here ends the run with a reason in [`reported::OUTCOME`] rather
/// than a panic, and the reason matters: a component that stopped because its
/// routing page was blank and one that stopped because it was told to look
/// identical from outside, and only one of them is the run the boot asked for.
fn serve() -> ! {
    let Ok(board) = Window::at(routing::AT, routing::BYTES) else {
        // Nothing to report *into*, so the status word is all there is.
        end(stopped::BAD_ROUTING)
    };
    // R04 at the one place this component reads a structure it did not build. A
    // page of zeroes is what a frame that was mapped and never filled in looks
    // like, and a zero length taken for a length reads as a device problem
    // rather than as a frame that did not speak.
    //
    // And a page carrying one of the *other three* drivers' magics is refused
    // here too, which is the whole reason the constants differ: one driver shape
    // means every component reads its board at the same address, so the magic is
    // the only thing that says which board this is.
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        report(&board, None, stopped::NO_ROUTING);
        end(stopped::NO_ROUTING)
    }

    let Some(parts) = laid_out(&board) else {
        report(&board, None, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    };

    // Its own outcome, and the module comment on `stopped::NO_CLOCK` says why:
    // it is the one field on that page whose wrong value produces a run that
    // *works*. Every entry would carry the same stamp, every stage downstream
    // would read them as simultaneous, and nothing would fail.
    let Some(clock) = Interrupt::new(parts.seed, parts.tick_nanos) else {
        report(&board, None, stopped::NO_CLOCK);
        end(stopped::NO_CLOCK)
    };

    let Ok(mut driver) = crate::driver::Driver::start(
        parts.windows,
        parts.queues,
        parts.data,
        clock,
        parts.class,
        parts.origin,
    ) else {
        report(&board, None, stopped::NO_DEVICE);
        end(stopped::NO_DEVICE)
    };

    let mut route = Route { control: parts.control, told: false };
    let mut idle: u64 = 0;
    let outcome = loop {
        // The control ring first, because a stop is the one thing that ends this
        // loop and work taken after it would be work submitted to a peer the
        // frame has already told this component it no longer has.
        if route.drain().is_err() {
            break stopped::NO_RING;
        }
        if route.told {
            break stopped::TOLD;
        }

        match driver.drain() {
            Ok(0) => {
                idle += 1;
                if idle > parts.spins {
                    break stopped::IDLE;
                }
                driver.spun();
                core::hint::spin_loop();
            }
            Ok(_) => idle = 0,
            // Every refusal `drain` can answer is the device disagreeing with
            // the specification — a used element naming a buffer this driver
            // never posted, or one shorter than the record it was required to
            // write — or an accessor refusing an offset, which is this
            // component's own arithmetic. Both end the run: a driver that
            // carried on after either would be decoding bytes it cannot account
            // for and submitting the result as something the user did.
            //
            // A full peer ring is deliberately **not** here. `Driver::drain`
            // counts it and continues, because an input driver that stopped when
            // a compositor fell behind would turn a slow frame into a dead
            // machine.
            Err(_) => break stopped::BAD_DEVICE,
        }
    };

    // Unlike the display driver, this one does tear the device down, and the
    // module comment says why: what a running virtio-input device does is write
    // into these buffers whenever the user acts, and the frame is about to take
    // the memory back.
    driver.stop();
    // The fold, onto the ring after the last event, for a consumer that is not
    // the frame. `crate::driver::Outbound::attest` is the argument. After the
    // device is stopped, so that nothing can be submitted behind it; and on
    // every ending that has a driver, not only `TOLD`, because a consumer
    // checking a run that ended some other way is owed the word for what did
    // cross. A ring with no room is published as *not attested* rather than
    // retried — the consumer then says it found none, which is true.
    let _ = driver.attest();
    // The input path's half of `E3-B04e`'s seam, asked last: every report this
    // run will ever see has been stamped, so the window is the one the
    // compositor will rebuild from the ring. Published and never submitted —
    // `crate::forecast` is why.
    let foreseen = driver.foresee(parts.asked);
    report(&board, Some((&driver, foreseen)), outcome);
    end(outcome)
}

/// Everything the routing page said, in the types that use it.
struct Parts {
    windows: Windows,
    queues: Region,
    control: Client,
    /// The **client's** end, which is what makes this component a producer.
    data: Client,
    /// The class every entry this component submits carries.
    /// Unit: none — an `f_abi::class` field.
    class: u16,
    /// Turns with nothing anywhere before the loop ends. Unit: turns.
    spins: u64,
    /// Unit: none — a seed.
    seed: u64,
    /// Unit: nanoseconds per report.
    tick_nanos: u64,
    /// Where the pointer starts, `(x, y)`: the frame's word, and the reason is
    /// [`routing::at::ORIGIN_X_X65536`]'s.
    /// Unit: device pixels, scaled by 65 536.
    origin: (i32, i32),
    /// The scanout this component's own predictor is asked about — a target
    /// the frame states about its display, and never a reading.
    /// [`routing::at::SCANOUT_AT_NANOS`] is the argument. RFC 0134.
    asked: crate::forecast::Asked,
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
    // ring whose peer cannot speak notices is not a control ring — and on this
    // component it is the *only* thing the control ring is for.
    let control = Adopted::at(
        board.read64(at::CONTROL_AT).ok()?,
        u32::try_from(board.read64(at::CONTROL_LEN).ok()?).ok()?,
        f_abi::feature::CONTROL_EVENTS,
        f_abi::feature::CONTROL_EVENTS,
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
    .client();

    // What the frame negotiated on this component's behalf, refused rather than
    // carried. This build speaks one ABI version and offers no feature on the
    // data ring, so anything else here is a peer that agreed to something this
    // component does not implement, and R04 says refuse rather than proceed and
    // misread. `user/panel/src/component.rs` is the other component in this
    // position and the check is deliberately the same two lines.
    if board.read64(at::NEGOTIATED_VERSION).ok()? != u64::from(f_abi::ABI_VERSION) {
        return None;
    }
    if board.read64(at::NEGOTIATED_FEATURES).ok()? != 0 {
        return None;
    }

    // A bound of zero is a driver that gives up before it has looked once, which
    // is a routing page this build cannot honour rather than a frame that meant
    // *do not wait*. Refused here, where it is a layout, rather than discovered
    // as a run that reported nothing.
    let spins = board.read64(at::IDLE_SPINS).ok()?;
    if spins == 0 {
        return None;
    }

    // A word back to the signed number it was written as, and refused unless
    // it is one the accumulator can hold: a word past `i32` taken for a
    // position would be a pointer that starts wrapped.
    let origin = |offset: u32| -> Option<i32> {
        i32::try_from(board.read64(offset).ok()?.cast_signed()).ok()
    };

    Some(Parts {
        windows,
        queues,
        control,
        data,
        class: u16::try_from(board.read64(at::ADMITTED).ok()?).ok()?,
        spins,
        seed: board.read64(at::STAMP_SEED).ok()?,
        tick_nanos: board.read64(at::STAMP_TICK_NANOS).ok()?,
        origin: (origin(at::ORIGIN_X_X65536)?, origin(at::ORIGIN_Y_X65536)?),
        asked: crate::forecast::Asked { scanout_nanos: board.read64(at::SCANOUT_AT_NANOS).ok()? },
    })
}

/// Write what this component did into the half of the routing page that is its
/// own.
///
/// The magic goes last, which is the whole of the discipline: a frame that reads
/// a page this function never finished finds a zero rather than a plausible
/// tally. RFC 0013's *read, never delivered* — the frame takes these numbers out
/// of memory it granted, and this component is never asked for them.
fn report(
    board: &Window,
    driver: Option<(&crate::driver::Driver, Option<crate::forecast::Foreseen>)>,
    outcome: u64,
) {
    if let Some((driver, foreseen)) = driver {
        let counters = driver.counters();
        let _ = board.write64(reported::RECORDS, counters.records);
        let _ = board.write64(reported::REPORTS, counters.reports);
        let _ = board.write64(reported::STAMPED, counters.stamped);
        let _ = board.write64(reported::SUBMITTED, counters.submitted);
        let _ = board.write64(reported::DROPPED, counters.dropped);
        let _ = board.write64(reported::IGNORED, counters.ignored);
        let _ = board.write64(reported::MALFORMED, counters.malformed);
        let _ = board.write64(reported::SPUN, counters.spun);
        let _ = board.write64(reported::CLOCK_AT, driver.clock_at_nanos());
        // The producer's half of the crossing attestation. Written here with
        // the rest of the report rather than as the run goes, for this
        // function's own reason: the magic goes last, so a frame that reads a
        // page this function never finished finds a zero — and a zero is not a
        // fold, because `Crossing::agrees_with` refuses one.
        let crossing = driver.crossing();
        let _ = board.write64(reported::CROSSING, crossing.word());
        let _ = board.write64(reported::CROSSED, crossing.absorbed());
        let _ = board.write64(reported::ATTESTED, u64::from(driver.attested()));
        let _ = board.write64(reported::MOTIONS, driver.motions());
        // Two's complement in a word, for the reason `f_compositor::routing`
        // gives about its own pointer words: a page holds words and not
        // integers with opinions, and a pointer left of the origin is an
        // ordinary place rather than an enormous one.
        let (x_x65536, y_x65536) = driver.at();
        let _ = board.write64(reported::POINTER_X_X65536, i64::from(x_x65536) as u64);
        let _ = board.write64(reported::POINTER_Y_X65536, i64::from(y_x65536) as u64);
        // The input path's prediction, `E3-B04e`. A run that predicted nothing
        // leaves every word zero, and `PREDICTED_FOR_NANOS` zero is what says
        // so — the one value no told scanout can take.
        let _ = board.write64(reported::PREDICTOR_REPORTS, driver.predictor_reports());
        if let Some(seen) = foreseen {
            let _ = board.write64(reported::PREDICTED_FOR_NANOS, seen.for_nanos);
            let _ = board.write64(reported::PREDICTED_X_X65536, i64::from(seen.x_x65536) as u64);
            let _ = board.write64(reported::PREDICTED_Y_X65536, i64::from(seen.y_x65536) as u64);
            let _ =
                board.write64(reported::ANCHOR_X_X65536, i64::from(seen.anchor_x_x65536) as u64);
            let _ =
                board.write64(reported::ANCHOR_Y_X65536, i64::from(seen.anchor_y_x65536) as u64);
            let _ = board.write64(reported::PREDICTED_LEAD_NANOS, seen.lead_nanos);
            let _ = board.write64(reported::PREDICTED_EXTRAPOLATED, u64::from(seen.extrapolated));
        }
    }
    let _ = board.write64(reported::OUTCOME, outcome);
    let _ = board.write64(reported::MAGIC, routing::MAGIC);
}

/// This component's end of the control ring.
///
/// # Why there is no `ask` here
///
/// Because there is nothing to ask for. The other three drivers hold this type
/// to submit RFC 0047's `DEVICE_MAP` — a client's buffer has to be translated by
/// the frame, because a component may not grant itself a device address — and
/// this component holds no client buffer. The module comment is the long version
/// and `kernel/src/input.rs` is where the consequence is stated for a reader of
/// the verdict rather than of this file.
struct Route {
    control: Client,
    /// Whether a stop notice has arrived. Once true it stays true.
    told: bool,
}

impl Route {
    /// Take every notice that has arrived.
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
    fn drain(&mut self) -> Result<(), ()> {
        loop {
            let Ok(taken) = self.control.take() else { return Err(()) };
            let Some(entry) = taken else { return Ok(()) };
            if is_notice(&entry) {
                if !notice::known(entry.result) {
                    return Err(());
                }
                if entry.result == notice::STOP {
                    self.told = true;
                }
                continue;
            }
            // A completion for a request this component never made. There is no
            // request it *can* make — see the type's own comment — so this is a
            // frame answering somebody else's question on this ring, which is a
            // peer this component cannot follow.
            return Err(());
        }
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
