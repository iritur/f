// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `compositor` as something that runs: the body a spawn puts at
//! `kernel::process::TEXT` and jumps to, and the loop that takes a client's
//! deltas from ring 3.
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
//! load-bearing across six crates: one linker script, one placement rule, one
//! check.
//!
//! # The loop is the block driver's, and the one difference is the whole task
//!
//! Read the ring, take the entry, answer it. There is no device below this
//! component and no second thing to poll, so an idle turn is a turn with nothing
//! to do — which is `user/virtio-blk`'s shape and not `user/virtio-net`'s.
//!
//! What differs is what an entry *means*. A block request is answered and
//! forgotten; a scene delta is **retained**, and the answer to the next one
//! depends on every one before it. That is the whole of why the graph is a
//! component's rather than a call's, and it is why this file allocates before
//! the loop and never inside it: the thing the loop mutates has to outlive every
//! entry in it.
//!
//! # The bound on an idle loop, and why it is a backstop here
//!
//! [`at::IDLE_SPINS`] is the number of turns this loop will spend with nothing
//! arriving before it stops. It exists because RFC 0046 says a hang is a count,
//! and it is *not* what ends an ordinary run: the frame's stop notice is. A run
//! that reaches this number is a run where the **frame** stopped serving, which
//! is why it is its own outcome — [`stopped::IDLE`] — and not folded into
//! [`stopped::TOLD`].
//!
//! It is no longer this component's only answer to *nothing has arrived*.
//! `E3-B01g` gave it a second one: on a frame that says it will ring —
//! [`at::DOORBELL`](crate::routing::at::DOORBELL) — an idle turn arms the ring's
//! wakeup flag, looks once more, and then asks the frame to stop this core.
//! The bound above still counts those turns, because a park that is never rung
//! is a hang and RFC 0046 says a hang is a count; what changed is that reaching
//! it now costs the machine nothing per turn.
//!
//! **The order is the protocol and not a style.** Arm, *then look again*, and
//! only then sleep. A component that armed and slept without looking is the
//! lost wakeup RFC 0020 describes from the other side: the entry it would have
//! found arrived between its last look and its decision, and the producer had
//! already read an unarmed flag and rung nothing. The `continue` after the arm
//! below is that second look, and deleting it is a hang rather than a slow loop.
//!
//! # What it does *not* do, said rather than implied
//!
//! It draws nothing. A frame closes, the graph changes, the counters move and
//! the client is told — and no pixel is produced anywhere in this build, because
//! the renderer is `E3-B02` and the surface it would draw into is the display
//! driver's. A reader expecting a picture at the end of this file should read
//! `crate`'s own comment, which names every sibling that owes one.

use f_abi::control::{is_notice, notice};
use f_abi::input::PAYLOAD_BYTES as INPUT_PAYLOAD_BYTES;
use f_abi::scene::PAYLOAD_BYTES;
use f_abi::{Cqe, door, feature, state};
use f_interface::token::Theme;
use f_ring::adopt::{Adopted, Client, Server};
use f_ring::device::Window;
use f_ring::heap::Heap;
use f_scene::arena::Arena;
use f_scene::commit::Batch;

use crate::inbound::Inbound;
use crate::pacing::Tick;
use crate::routing::{self, at, bell, life, node, reported, stopped};
use crate::tree::{FRAME_DELTAS_MAX, Held, Plan};

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
    if entry.selector() == life::SERVE {
        serve()
    }

    // The frame tells a component what it holds rather than letting it assume,
    // and `door::Entry` argues why: a second occupant of a place finds its
    // capabilities at the same indices and a later generation.
    let _ = entry.granted(0);

    // "I am here." The one thing the frame cannot observe from outside.
    let _ = door::call0(door::ANNOUNCE);

    end(DONE)
}

/// Hold the graph and serve the data ring until the frame says stop.
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
    // like, and a zero length taken for a length reads as a peer problem rather
    // than as a frame that did not speak.
    if board.read64(at::MAGIC) != Ok(routing::MAGIC) {
        report(&board, None, &Inbound::UNCONNECTED, stopped::NO_ROUTING);
        end(stopped::NO_ROUTING)
    }

    let Some(parts) = laid_out(&board) else {
        report(&board, None, &Inbound::UNCONNECTED, stopped::BAD_ROUTING);
        end(stopped::BAD_ROUTING)
    };

    // --- the graph ----------------------------------------------------------
    //
    // **The only allocations this component makes, and they are made before the
    // loop rather than inside it.** `crate`'s comment says why the graph is on
    // the heap at all; what is here is why the size is checked first. `Box::new`
    // has no fallible form on this toolchain — `Box::try_new` is unstable — so
    // an allocation that could not be answered reaches `handle_alloc_error`,
    // which in an image compiled with `panic=immediate-abort` is an instruction
    // that stops the core with nothing written anywhere and nothing in the boot
    // log naming the cause.
    //
    // So the region is measured against what is about to be asked of it, out of
    // the prologue the *frame* wrote, and a heap that cannot answer ends this
    // run with a reason instead. That is what makes [`stopped::NO_GRAPH`] a
    // reachable outcome rather than a name for something that aborts.
    let heap = Heap::COMPONENT;
    if !heap.valid() || (heap.bytes() as usize) < crate::HELD_BYTES + crate::HEAP_OVERHEAD {
        report(&board, None, &Inbound::UNCONNECTED, stopped::NO_GRAPH);
        end(stopped::NO_GRAPH)
    }
    // **Two boxes and not one, and `crate::tree::Held`'s own comment is the
    // arithmetic.** The short version: an empty arena is all zeroes and a box of
    // one is an allocation and a `memset`, while an empty batch is not, so a
    // single value holding both put a hundred and thirty-six kilobyte constant
    // in this component's image and the file outgrew what the frame maps for it.
    let mut graph = alloc::boxed::Box::new(Arena::EMPTY);
    let mut batch = alloc::boxed::Box::new(Batch::<FRAME_DELTAS_MAX>::new());
    // The pacing window rides in `Held` rather than in a third box, and
    // `crate::tree::Held`'s own comment says why: a kibibyte of zeroes is a
    // `memset` and costs the image nothing, which is RFC 0100's rule read the
    // way round that permits something rather than the way round that refuses.
    //
    // **The theme, resolved here and never again.** `Theme::DEFAULT` is a
    // constant of the resolver rather than something this component chose, and
    // it is spelled here because nothing in this build carries a theme *to* a
    // component: there is no word for one on the routing page and no ring that
    // delivers one. The day either exists, the theme arrives beside the pacing
    // inputs in `laid_out` and reaches this argument, and `crate::tree` does not
    // change — which is the whole reason `Held::new` takes a theme instead of
    // reading one.
    //
    // What it cost the image is **6 136 bytes, measured rather than estimated**:
    // `cargo xtask component` reports 23 240 for this component without this
    // argument and the three tree words that go with it, and 29 376 with them,
    // against the 65 536 the frame maps and that command refuses past. The
    // difference is the resolver's two hundred and fifty-six-entry sRGB transfer
    // table and the clamp that walks it, and it is *not* a `Resolved` sitting in
    // `.rodata` — RFC 0100's rule is about the constants a component
    // materialises, and what this line produces is written at run time out of a
    // theme rather than copied out of the image.
    //
    // It is worth a number rather than a reassurance because the number is what a
    // future reader needs: a second theme resolved here would cost the table
    // nothing and the `Resolved` about a kibibyte of stack, while a *constant*
    // `Resolved` — the obvious optimisation, a palette baked at compile time —
    // would cost the image the whole of it and would be RFC 0079's first reversal
    // condition wearing a performance argument.
    let mut held = Held::new(&mut graph, &mut batch, parts.plan, &Theme::DEFAULT);

    let mut route = Route { control: parts.control, told: false };
    let mut idle: u64 = 0;
    // The doorbell's three pieces of state, and they are separate on purpose.
    // `armed` is what the *ring* believes, and it must be put back the moment
    // work arrives or the client rings for every entry it is already draining
    // — which is the number `f_ring::doorbell` exists to drive to zero. The two
    // counts are what this component publishes, and they differ by the times the
    // frame had already been rung when it was asked to stop.
    let mut armed = false;
    let mut parked: u64 = 0;
    let mut halted: u64 = 0;
    // The last reading this component believed. Zero until the frame writes one,
    // which is *the epoch* and not *unknown*: a compositor whose first entry
    // arrives before the frame has ticked charges that frame from the origin,
    // which is a cost that is too large rather than one that is invented.
    let mut last = Tick(0);
    // The input ring's consumer half, `E3-B04g`. Connected or not is decided
    // once, at layout, for `Parts::doorbell`'s reason: it is a fact about the
    // frame that started this component.
    let mut inbound =
        if parts.input.is_some() { Inbound::connected() } else { Inbound::UNCONNECTED };
    let outcome = loop {
        // The control ring first, because a stop is the one thing that ends this
        // loop and work taken after it would be work done for a client the frame
        // has already told this component it no longer has.
        if route.drain().is_err() {
            break stopped::NO_RING;
        }
        if route.told {
            break stopped::TOLD;
        }

        // The input ring second, and all of it, before the scene entry this turn
        // answers. **This is where a position enters this component**, and the
        // order is what makes the latch's window mean what `crate::latch` says:
        // every report that had crossed when this turn began is in the predictor
        // before a commit taken this turn closes a frame. Nothing is answered —
        // every input entry carries `NO_CQE` and nobody is waiting — and nothing
        // here reads a clock: the time is the driver's, in the entry, and
        // `crate::latch::LateLatch::drained` is where it is rebuilt.
        if let Some(input) = parts.input {
            match drain_input(input, &mut inbound, &mut held) {
                Ok(0) => {}
                Ok(_) => idle = 0,
                Err(()) => break stopped::NO_RING,
            }
        }

        // Room to answer in, asked *before* an entry is taken. An entry popped
        // and then not completed is a client waiting forever for a reply that
        // was dropped on the floor, which is the one failure a serving component
        // owes its peer never to produce.
        let room = match parts.data.free() {
            Ok(room) => room,
            Err(_) => break stopped::NO_RING,
        };
        let taken = if room == 0 {
            None
        } else {
            match parts.data.pop() {
                Ok(taken) => taken,
                Err(_) => break stopped::NO_RING,
            }
        };

        let Some(entry) = taken else {
            // A turn with nothing to do, and a turn with nothing to answer
            // *into*, counted as one thing on purpose: both are this loop
            // waiting for a peer, and the bound is a backstop against a peer
            // that never comes back rather than a measurement of why.
            idle += 1;
            if idle > parts.spins {
                break stopped::IDLE;
            }
            // **A turn with no room to answer in is not a turn to sleep on**, and
            // that is the one place the two things this branch counts as one have
            // to come apart. A doorbell means *a submission arrived*; nothing
            // rings when a client reaps, because a completion ring has no wakeup
            // flag and needs none. So a component that parked because its
            // completion ring was full would be waiting for a signal its peer has
            // no reason to send, and would be rescued only by a timer tick —
            // which is an accident and not a protocol.
            //
            // **And a component holding the input ring does not park either.**
            // The driver submits unasked and rings nobody — `f_ring::doorbell`
            // is the scene ring's, and no input entry arms or answers one — so a
            // compositor asleep on the scene ring's bell would take the next
            // position only when a client happened to submit, which is a latch
            // aimed at a pointer the device reported some unbounded time ago.
            // *What would reverse this:* a driver that rings its consumer, which
            // is `E1-B09`'s interrupt delivery arriving at ring 3.
            if !parts.doorbell || room == 0 || inbound.is_connected() {
                core::hint::spin_loop();
                continue;
            }
            if !armed {
                // Tell the client to ring, and then go round again. The second
                // look is the `continue`, and the module comment says why it may
                // not be optimised into a re-read here: a component that armed
                // and slept in one turn is the lost wakeup.
                if parts.data.arm_wakeup().is_err() {
                    break stopped::NO_RING;
                }
                armed = true;
                continue;
            }
            // Armed, and the turn after the arm found nothing. Stop the core.
            parked += 1;
            let _ = board.write64(reported::PARKED, parked);
            if door::call0(door::WAIT) == door::HALTED {
                halted += 1;
                let _ = board.write64(reported::HALTED, halted);
            }
            continue;
        };
        idle = 0;
        if armed {
            // Work arrived, so the client must stop ringing for it. Before the
            // entry is answered rather than after, because the answer is what
            // lets the client submit the next one — and a client that submitted
            // against a flag this component had not put back would be rung for
            // an entry it was already holding.
            if parts.data.disarm_wakeup().is_err() {
                break stopped::NO_RING;
            }
            armed = false;
        }

        // The payload, out of the ring's own arena. **Zeroed first and offered
        // whatever the copy answered**, which is deliberate: an entry framing a
        // payload that is not in the arena is a delta whose bytes never landed,
        // and the decoder refuses a zeroed payload — which poisons the frame it
        // was offered to, so the frame that lost an entry can never close. A
        // refusal invented here instead would answer the client and leave the
        // batch believing it still had a whole frame, which is the one mistake
        // that turns a lost entry into a scene nobody asked for.
        let mut payload = [0u8; PAYLOAD_BYTES];
        let _ = parts.data.copy_out(entry.offset as usize, &mut payload);

        // **The one time this component ever sees, read after the entry was
        // taken and never before it.** The order is what makes the reading
        // belong to this entry: the client writes the tick and then submits, so
        // a reading taken after the pop is the one the client wrote for the
        // entry the pop returned — the ring's `Release` publish and this side's
        // `Acquire` take are what order the two, which is the same pair
        // `ring/src/lib.rs` rests the payload on. A tick read at the top of the
        // turn would be whatever was there when the loop last spun, and a
        // frame's cost would become a measure of how often this loop went round.
        //
        // A page that stops answering leaves the previous reading in place
        // rather than ending the run: a clock that did not refresh is a pacing
        // estimate that is stale, and a compositor that stopped serving its
        // client over a stale estimate would be refusing to draw because it
        // could not say what time it was.
        let now = Tick(board.read64(at::TICK_NANOS).unwrap_or(last.nanos()));
        last = now;

        if let Some(answer) = held.offer(&entry, &payload, now) {
            if parts.data.post(answer).is_err() {
                break stopped::NO_RING;
            }
            // **The return leg of `E3-B01j`'s count, taken where the ring took
            // the completion and not where it was produced.** A completion the
            // ring refused never crossed the boundary, so counting it at the
            // answer would count this component's intentions; and the frame
            // requires this number to equal the completions its own client
            // reaped, which is what makes the two counts independent rather than
            // one derived from the other. The `if let` above was one condition
            // with an `&&` until this line existed and had to come apart, because
            // a short-circuit cannot tell *no completion was owed* from *the post
            // failed*.
            held.answered();
        }
    };

    report(&board, Some(&held), &inbound, outcome);
    end(outcome)
}

/// Take everything on the input ring, decode it, fold it, and hand every
/// position to the latch.
///
/// Answers how many entries were taken, so the loop can tell a busy turn from an
/// idle one. The four steps are `crate::inbound`'s header; what is here is only
/// the ring half — the pop and the payload copy — because that is the half a
/// host cannot drive.
///
/// **Zeroed and offered whatever the copy answered**, for the scene payload's
/// reason in `serve`: an entry framing a payload outside the arena is an event
/// whose bytes never landed, and a zeroed payload is `NOT_STAMPED`, which the
/// decoder refuses and `Inbound` counts. An attestation carries no payload and
/// the copy is ignored for it.
///
/// # Errors
///
/// The ring stopped validating, which is a peer that has stopped speaking.
fn drain_input(input: Server, inbound: &mut Inbound, held: &mut Held<'_>) -> Result<u64, ()> {
    let mut taken = 0;
    loop {
        let entry = match input.pop() {
            Ok(Some(entry)) => entry,
            Ok(None) => return Ok(taken),
            Err(_) => return Err(()),
        };
        taken += 1;
        let mut payload = [0u8; INPUT_PAYLOAD_BYTES];
        let _ = input.copy_out(entry.offset as usize, &mut payload);
        if let Some(event) = inbound.take(&entry, &payload) {
            let _ = held.drained(&event);
        }
    }
}

/// Everything the routing page said, in the types that use it.
struct Parts {
    control: Client,
    data: Server,
    /// The input driver's data channel, whose consumer end this component
    /// holds, or `None` where the frame connected none — `input=withheld`, and
    /// every boot that is not about input.
    input: Option<Server>,
    /// Turns with nothing to do before the loop ends. Unit: turns.
    spins: u64,
    /// Whether the frame says it will ring a doorbell for this component.
    ///
    /// Read once, at layout, and never again: it is a property of the frame
    /// this component was started by, and a frame that changed its mind half
    /// way through a run would be a frame that had stopped ringing for a
    /// component already asleep. `crate::routing::at::DOORBELL` says why it is
    /// told rather than discovered.
    doorbell: bool,
    /// What this machine can draw with, how often it scans out, and how much of
    /// a frame to keep in hand.
    plan: Plan,
}

/// Read the routing page and state everything it names.
///
/// `None` for any address that cannot be stated as a channel — which is a frame
/// that filled this page in wrongly, and is refused here rather than
/// dereferenced to find out.
fn laid_out(board: &Window) -> Option<Parts> {
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

    // The data ring offers nothing and requires nothing. A scene delta's payload
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
    .server();

    // What the frame negotiated on this client's behalf, refused rather than
    // carried. This build speaks one ABI version and offers no feature on its
    // data ring — `features = []` in its manifest — so anything else here is a
    // peer that agreed to something this component does not implement, and R04
    // says refuse rather than proceed and misread.
    if board.read64(at::NEGOTIATED_VERSION).ok()? != u64::from(f_abi::ABI_VERSION) {
        return None;
    }
    if board.read64(at::NEGOTIATED_FEATURES).ok()? != 0 {
        return None;
    }

    // A bound of zero is a component that gives up before it has looked once,
    // which is a routing page this build cannot honour rather than a frame that
    // meant *do not wait*. Refused here, where it is a layout, rather than
    // discovered as a run that served nothing.
    let spins = board.read64(at::IDLE_SPINS).ok()?;
    if spins == 0 {
        return None;
    }

    // The pacing inputs, read and **not** refused. Every one of them has an
    // honest answer at zero — no display declared, no margin wanted, no
    // capability reported — and `crate::pacing` says what each zero produces: a
    // scanout at now, a wake time with no margin in it, and a published rung of
    // *none*. A refusal here would be this component declining to serve a client
    // because nobody had told it about a screen, which is the opposite of what
    // it is for.
    let plan = Plan {
        backend_bits: board.read64(at::BACKEND_CAPABILITIES).ok()?,
        scanout_period_nanos: board.read64(at::SCANOUT_PERIOD_NANOS).ok()?,
        margin_nanos: board.read64(at::PACING_MARGIN_NANOS).ok()?,
        // Narrowed here rather than in `crate::latch`, because a node identifier
        // is a `u32` on the wire and this is the one place the page's word
        // becomes one. A word naming a node outside that range is a frame this
        // component cannot honour, and `f_abi::scene::NO_NODE` — zero — is what
        // it becomes: no latch, published as a decline on every frame, rather
        // than a truncation that would name a different node.
        pointer_node: u32::try_from(board.read64(at::POINTER_NODE).ok()?).unwrap_or(0),
    };

    // Not refused at any value, and the reason is the same shape as the pacing
    // inputs above: every reading has an honest answer, and the honest answer to
    // a word this frame did not write is *nobody rings, so look for yourself*.
    // A refusal here would be this component declining to serve a client because
    // it had not been promised a wakeup.
    let doorbell = board.read64(at::DOORBELL).ok()? == bell::RING;

    // The input ring, `E3-B04g`, offering and requiring nothing — the driver's
    // manifest declares `features = []` on it. Zero is *not connected*, which is
    // an honest answer and the control's whole difference, so it is not a
    // refusal; a word that is not zero and does not adopt is a frame that filled
    // this page in wrongly, and is refused as every other address here is.
    let input_at = board.read64(at::INPUT_AT).ok()?;
    let input = if input_at == 0 {
        None
    } else {
        let len = u32::try_from(board.read64(at::INPUT_LEN).ok()?).ok()?;
        Some(Adopted::at(input_at, len, 0, 0).ok()?.server())
    };

    Some(Parts { control, data, input, spins, plan, doorbell })
}

/// Write what this component did into the half of the routing page that is its
/// own, and into the tree the frame mounted for it.
///
/// The magic goes last, which is the whole of the discipline: a frame that reads
/// a page this function never finished finds a zero rather than a plausible
/// tally. RFC 0013's *read, never delivered* — the frame takes these numbers out
/// of memory it granted, and this component is never asked for them.
fn report(board: &Window, held: Option<&Held>, inbound: &Inbound, outcome: u64) {
    let mut published = 0;
    // The input ring, `E3-B04g`, on every ending and whether or not a graph was
    // ever held: a component that refused its routing page still says it was
    // not connected, which is the true sentence about that run.
    let crossing = inbound.crossing();
    let attested = inbound.attested();
    for (offset, value) in [
        (reported::INPUT_CONNECTED, u64::from(inbound.is_connected())),
        (reported::INPUT_ENTRIES, inbound.entries()),
        (reported::INPUT_DECODED, inbound.decoded()),
        (reported::INPUT_REFUSED, inbound.refused()),
        (reported::INPUT_MOTIONS, inbound.motions()),
        (reported::INPUT_CROSSING, crossing.word()),
        (reported::INPUT_CROSSED, crossing.absorbed()),
        (reported::INPUT_ATTESTATIONS, inbound.attestations()),
        (reported::INPUT_ATTESTED, attested.map_or(0, |said| said.word())),
        (reported::INPUT_ATTESTED_COUNT, attested.map_or(0, |said| said.absorbed())),
        (reported::INPUT_UNATTESTED, inbound.unattested()),
        (reported::INPUT_AGREED, u64::from(inbound.agrees())),
    ] {
        let _ = board.write64(offset, value);
    }
    if let Some(held) = held {
        let counters = held.counters();
        let _ = board.write64(reported::DRAINED, counters.drained);
        let _ = board.write64(reported::STAGED, counters.staged);
        let _ = board.write64(reported::FRAMES, counters.frames);
        let _ = board.write64(reported::EDITS, counters.edits);
        let _ = board.write64(reported::CREATED, counters.created);
        let _ = board.write64(reported::REMOVED, counters.removed);
        let _ = board.write64(reported::LIVE, held.live());
        let _ = board.write64(reported::REFUSED, counters.refused);
        let _ = board.write64(reported::TOKEN, counters.token);
        let _ = board.write64(reported::LATE, counters.late);
        // The pacing decision, whole. Four numbers where one would do, because
        // the exit's sentence is a subtraction and a reader handed only the
        // answer cannot check it — `crate::routing::reported::WAKE` argues the
        // same point one file over.
        let story = held.story();
        let _ = board.write64(reported::SCANOUT, story.decision.scanout_nanos);
        let _ = board.write64(reported::ESTIMATE, story.decision.estimate_nanos);
        let _ = board.write64(reported::MARGIN, story.decision.margin_nanos);
        let _ = board.write64(reported::WAKE, story.decision.wake_nanos);
        let _ = board.write64(reported::SAMPLES, held.samples());
        // The resolved theme, `E3-B06d`. **The report is carried out rather than
        // consulted and dropped**, which is the clause: a compositor that
        // resolved a theme, moved somebody's colours to clear a floor and then
        // threw away the record of having done it would make RFC 0079's
        // *clamped, not refused* an arrangement nobody could audit. Five words
        // where a flag would do, for `reported::WAKE`'s reason — a reader handed
        // only *clean* cannot tell a theme this layer agreed with from a report
        // whose notes all fell out of the bottom.
        let readability = held.readability();
        let _ = board.write64(reported::RESOLVES, readability.resolves());
        let _ = board.write64(reported::NOTES, readability.report().len() as u64);
        let _ = board.write64(reported::DROPPED, u64::from(readability.report().dropped()));
        let _ = board.write64(reported::CLEAN, u64::from(readability.report().is_clean()));
        let _ = board.write64(reported::RULES, readability.rules_owed());
        // The degradation register, `E3-B07d`: one field per frame for the last
        // `crate::pacing::degraded::FRAMES` frames, and not the last frame's
        // answer. `crate::pacing::Record` argues why a snapshot cannot be
        // evidence of a per-frame decision, and RFC 0118 is the entry.
        let _ = board.write64(reported::DEGRADED, story.degraded.word());
        // The rung comes off `Held` and not off the story, which is RFC 0119: a
        // value written once when this component started does not belong in the
        // struct a degradation policy rewrites every frame.
        let _ = board.write64(reported::RUNG, held.rung().word());
        let _ = board.write64(reported::DEADLINE, story.deadline_nanos);
        // The boundary crossings, `E3-B01j`. Three words where one would do, for
        // `reported::WAKE`'s reason: the exit's sentence is an addition and a
        // division, and a reader handed only the answer cannot check either. Both
        // directions are already above — `DRAINED` and `ANSWERED` — and the sum is
        // the component's own rather than the frame's, because the frame keeping
        // the only sum is the *one side counts and the other trusts it*
        // arrangement this line exists to prevent.
        let _ = board.write64(reported::ANSWERED, counters.answered);
        let _ = board.write64(reported::CROSSINGS, counters.crossings());
        let _ = board
            .write64(reported::CROSSINGS_PER_FRAME_X1000, counters.crossings_per_frame_x1000());
        // The synchronisation state, `E3-B05f`. **Carried out rather than
        // consulted and dropped**, which is this file's rule for every record it
        // holds: a compositor that traced a frame's waits and threw the trace away
        // would have made the one failure of this mechanism that leaves no
        // evidence — `abi/src/sync.rs`'s third outcome, *a hang writes no log
        // line* — leave no evidence again. Seven words where three have a node,
        // and `crate::routing::reported` draws the division: three say what a
        // reader of a running machine wants, and four are about whether the record
        // those three come out of can be believed.
        let waits = held.waits().published();
        let _ = board.write64(reported::WAITS, waits.outstanding);
        let _ = board.write64(reported::SIGNALLED, waits.signalled);
        let _ = board.write64(reported::TIMEOUTS, waits.timeouts);
        let _ = board.write64(reported::TRACED, waits.traced);
        let _ = board.write64(reported::TRACE_DROPPED, waits.dropped);
        let _ = board.write64(reported::TRACE_COMPLETE, waits.complete);
        let _ = board.write64(reported::CHAIN_REFUSALS, waits.refusals);
        // The latch, `E3-B01i`. Carried out rather than consulted and dropped,
        // for the traces' reason above: the one thing this component does that no
        // client asked for is the one thing a reader has no other way to see.
        //
        // Both ends of the transform and neither difference: `crate::latch::
        // Latched` says why, and the short version is that a component
        // publishing its own subtraction cannot be checked against itself.
        let latch = held.latch();
        let _ = board.write64(reported::POINTER_REPORTS, latch.reports());
        let _ = board.write64(reported::POINTER_STALE, latch.stale());
        let _ = board.write64(reported::POINTER_UNSTAMPED, latch.unstamped());
        let _ = board.write64(reported::LATCHES, latch.latches());
        let _ = board.write64(reported::LATCH_DECLINES, latch.declines());
        let _ = board.write64(reported::LATCH_AIM_NANOS, latch.aimed_at_nanos());
        if let Ok(latched) = latch.last() {
            let _ = board.write64(reported::LATCH_ENTRY, u64::from(latched.before_entry()));
            let _ =
                board.write64(reported::LATCH_COMMITTED_X, latched.committed_tx_x65536() as u64);
            let _ =
                board.write64(reported::LATCH_COMMITTED_Y, latched.committed_ty_x65536() as u64);
            let _ = board.write64(reported::LATCH_X, latched.latched_tx_x65536() as u64);
            let _ = board.write64(reported::LATCH_Y, latched.latched_ty_x65536() as u64);
            let _ = board.write64(reported::LATCH_LEAD_NANOS, latched.lead_nanos());
            let _ = board.write64(reported::LATCH_EXTRAPOLATED, u64::from(latched.extrapolated()));
            let _ = board.write64(reported::LATCH_UNMOVED_BEFORE, latched.unmoved_before());
            let _ = board.write64(reported::LATCH_UNMOVED_AFTER, latched.unmoved_after());
            let _ = board.write64(reported::LATCH_WALKED, u64::from(latched.walked()));
        }
        // The same numbers, into the region the frame mounted under its own
        // root. The board is this component's answer to *what did you do*; the
        // tree is the machine's answer to *what is it running*, and RFC 0013
        // wants the second read rather than delivered. Both come out of one set
        // of counters, so the two cannot disagree without one of them being
        // wrong.
        published = publish(board.read64(at::TREE_AT).unwrap_or(0), held);
    }
    let _ = board.write64(reported::PUBLISHED, published);
    let _ = board.write64(reported::OUTCOME, outcome);
    let _ = board.write64(reported::MAGIC, routing::MAGIC);
}

/// Store this run's counts into the tree the frame published for this instance.
///
/// **The address comes off the board and is never assumed.** Only the shapes
/// that publish a tree map one, so a component holding the constant would fault
/// at ring 3 on every boot that stood it up some other way — `user/virtio-blk`
/// found that first and the comment there is the long version.
///
/// Answers how many nodes took a word, which the frame reads back beside the
/// tree itself. A node the schema does not carry is refused by
/// `f_abi::state::Writer::set`, so a manifest and a [`node`] list that disagree
/// produce a number below [`node::WRITTEN`]'s length rather than a silence.
///
/// Nothing here is `unsafe` and nothing here could be: `f_abi::state::Writer` is
/// why that type has a write side.
/// Unit: nodes.
fn publish(tree_at: u64, held: &Held) -> u64 {
    if tree_at == 0 {
        return 0;
    }
    let Ok(tree) = state::Writer::at(tree_at, routing::TREE_BYTES) else { return 0 };
    let counters = held.counters();
    let story = held.story();
    let mut written = 0;
    let waits = held.waits().published();
    // In `node::WRITTEN`'s order, and the frame reads it back in that order.
    // Fifteen words and not four: the five `E3-B01k` adds are the frame's story —
    // the rung it is drawing with, the frame it last closed, the deadline that
    // frame carried, what a frame costs on this machine, and what was given up
    // to fit — the three `E3-B06d` adds are the resolved theme, and the three
    // `E3-B05f` adds are the synchronisation of the frame that closed last. Every
    // one of them is a value this component already holds, which is RFC 0013's
    // rule: a node with no counter behind it would be a serialisation with extra
    // steps wearing RFC 0013's name.
    //
    // **Fifteen is this manifest full.** `f_abi::manifest::STATE_NODES_MAX` is
    // sixteen and the subtree is one of them, so the next node this component
    // wants is an RFC widening that bound and not a row in `manifest.toml`.
    // `node::WRITTEN` says the same thing where the ids are, and it is said twice
    // on purpose: the author who reaches for a sixteenth word will be in one of
    // the two files and not necessarily this one.
    let readability = held.readability();
    for (id, value) in [
        (node::FRAMES, counters.frames),
        (node::EDITS, counters.edits),
        (node::NODES, held.live()),
        (node::REFUSED, counters.refused),
        (node::RUNG, held.rung().word()),
        (node::FRAME, counters.token),
        (node::DEADLINE, story.deadline_nanos),
        (node::PACING, story.decision.estimate_nanos),
        (node::DEGRADED, story.degraded.word()),
        // The resolved theme, `E3-B06d`. `len()` is a `usize` in a crate that
        // compiles for two architectures, so the widening is written rather than
        // inferred; on neither of them can a count bounded by
        // `f_interface::token::NOTES_MAX` fail to fit.
        (node::RESOLVES, readability.resolves()),
        (node::NOTES, readability.report().len() as u64),
        (node::RULES, readability.rules_owed()),
        // The synchronisation state, `E3-B05f`. Three of the seven this component
        // reports, and the three a reader of a *running machine* wants: is
        // anything still waiting, what value did this compositor reach, and how
        // many frames has it abandoned. The other four are the trace's own
        // integrity and are on the board, where the frame checks this component
        // rather than reading the machine.
        (node::WAITS, waits.outstanding),
        (node::SIGNALLED, waits.signalled),
        (node::TIMEOUTS, waits.timeouts),
    ] {
        if tree.set(id, value) {
            written += 1;
        }
    }
    written
}

/// This component's end of the control ring.
///
/// There is nothing on it this component asks for: a compositor reaches no
/// device, so it needs no translation, and the ring carries the frame's notices
/// in one direction only. That is why this type is smaller than the drivers'
/// equivalent and why it has no token in it — a component that never submits
/// never has an answer to match.
struct Route {
    control: Client,
    /// Whether a stop notice has arrived. Once true it stays true.
    told: bool,
}

impl Route {
    /// Take everything the frame has posted, and record a stop.
    ///
    /// **This is the polling point.** Notices are read on the way past; nothing
    /// is discarded, and a kind this build cannot name ends the run rather than
    /// being skipped — R04, and `f_abi::control::notice::known` is the one list
    /// that says which kinds exist.
    ///
    /// # Errors
    ///
    /// The ring stopped validating, or it carried something this component
    /// cannot name. Both mean the peer has stopped speaking, and RFC 0008 says
    /// what happens to a component whose peer has.
    fn drain(&mut self) -> Result<(), ()> {
        loop {
            let taken = match self.control.take() {
                Ok(taken) => taken,
                Err(_) => return Err(()),
            };
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
            // A completion for something this component submitted, which in this
            // build is nothing. Dropped rather than misread: an answer to a
            // question nobody asked is a frame speaking a protocol this
            // component was not built against, and the one thing worse than
            // dropping it would be matching it against a request that does not
            // exist.
            let _: Cqe = entry;
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
