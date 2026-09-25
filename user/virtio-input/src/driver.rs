// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What the device wrote, turned into what the user did, and put on a ring.
//!
//! # The one translation in this system, and why it is here
//!
//! A `virtio_input_event` is eight bytes: a type, a code and a value, in the
//! evdev vocabulary. An `f_abi::input::Event` is a stamp and one of six records
//! in this system's vocabulary. They are not the same vocabulary and neither is
//! a subset of the other, so something has to translate, and this file is that
//! something.
//!
//! It is here — in the driver, at the stage that holds the device — rather than
//! anywhere later, for the reason `abi/src/input.rs` gives about
//! [`PointerMotion`](f_abi::input::PointerMotion) and then declines to give
//! twice: *the stage that accumulates displacement into position must be the
//! stage that holds the device, and there is exactly one of those*. A mouse
//! reports relative motion; the wire carries a position; the accumulator is
//! therefore in this file, in [`Decoder`], and there is nowhere else it could be
//! without two consumers keeping origins that drift apart for good.
//!
//! # What one report is
//!
//! evdev sends a *report*: a run of records ending in `EV_SYN`/`SYN_REPORT`,
//! which together are one thing the user did. A mouse moved diagonally sends
//! `REL_X`, `REL_Y`, `SYN_REPORT` — three records, one motion.
//!
//! So this file is organised around the report and not the record, and two
//! consequences follow that a record-at-a-time driver would get wrong:
//!
//! - **One stamp per report.** [`crate::clock::Interrupt::stamp`] is called when
//!   the first record of a report arrives and not again until the next report
//!   begins. `abi/src/input.rs` is explicit that this is the rule — *two entries
//!   bearing one stamp are one instant, because there is only one clock reading
//!   per instant to bear* — and a driver that stamped per record would report a
//!   diagonal movement as an x movement followed later by a y movement, which is
//!   an ordering the device did not claim.
//! - **Motion and scroll are flushed at the boundary, and buttons are not.** A
//!   report can carry at most one motion and at most one scroll, because both
//!   are accumulations; it can carry several buttons, and each of those is a
//!   discrete thing that happened. So a key record produces its entry
//!   immediately and a `REL_X` produces nothing until the `SYN_REPORT`.
//!
//! # What this driver does not translate, and what each absence costs
//!
//! - **`EV_ABS`.** No tablet, no touchscreen. This driver therefore never
//!   produces an `f_abi::input::TouchPoint` or an `f_abi::input::StylusPoint`,
//!   and that is a refusal rather than a gap: those two records carry a contact
//!   identifier, a pressure and a tilt, and **none of those can be derived from a
//!   relative device**. A driver that filled them in with plausible constants
//!   would be putting values on a wire that no device produced, which is the one
//!   thing an input path must never do — every stage above it would be reasoning
//!   about a pressure nobody applied. `crate::routing::reported::IGNORED` counts
//!   what is skipped. What it costs is that `virtio-tablet-pci` binds and reports
//!   nothing.
//! - **Autorepeat.** `EV_KEY` with a value of two is a key the device is
//!   repeating, and it is ignored. Whether a held key repeats, how fast, and
//!   after how long, is a policy a keymap owns and a user changes; a driver that
//!   forwarded the device's opinion would be putting a setting on the wire at
//!   interrupt time where nothing can change it. The press and the release are
//!   both reported, which is everything a stage above needs to implement any
//!   repeat policy it likes.
//! - **`EV_MSC`, `EV_SW`, `EV_LED`, `EV_SND`, `EV_FF` and everything else.**
//!   Counted, not refused — [`crate::routing::reported::IGNORED`] says why this
//!   is the one place R04's *refuse what you do not know* is deliberately not
//!   applied: a device producing a record type this build has no opcode for is
//!   doing more than this driver was written for, not sending nonsense, and
//!   stopping a machine's input over it would be worse than the absence.
//!
//! # The origin of the pointer, which a relative device cannot supply
//!
//! [`Decoder`] starts the pointer where the frame says and accumulates from
//! there, and nothing clamps it. Both halves are deliberate and both are costs:
//!
//! The origin is told because a mouse has no position to report — it reports
//! that it moved. Nothing in this component knows where the pointer was when
//! the machine booted, so the start is the routing page's
//! [`crate::routing::at::ORIGIN_X_X65536`] rather than a number this file
//! chooses. What that costs is unchanged: the first motion after a restart puts
//! the pointer wherever the told origin plus that movement lands rather than
//! where the user left it. [`Decoder::new`] is the told origin `(0, 0)`, which
//! is what every host test here starts from.
//!
//! **Why told rather than `(0, 0)`, which it was until 2026-09-25.** Not for this
//! component's sake: the frame that stands it up also commits the client's
//! pointer transform, and over a transform committed at zero a compositor latch
//! that *added* the position to the committed translation cannot be told from
//! one that replaced it. A frame that can name the start can commit there, and
//! a start that is not zero is what tells the two apart. RFC 0132.
//!
//! Nothing clamps because clamping needs the size of the surface the pointer is
//! on, which is the compositor's and not the driver's, and a driver that clamped
//! to a guess would be silently discarding movement. `abi/src/input.rs` makes the
//! field signed for exactly this reason — *a pointer that has left the surface
//! has a position outside it, and clamping on the wire would lose the direction
//! it left in*. The saturation at `i32`'s ends is not a clamp to a surface; it is
//! the arithmetic refusing to wrap, which would move the pointer to the other
//! side of the world.

use f_abi::input::{
    self, Crossing, Entry, Event, Key, PointerButton, PointerMotion, Scroll, axis_source, edge,
};
use f_input::stamp::StampNanos;
use f_ring::adopt::Client;
use f_ring::device::Region;

use crate::Trouble;
use crate::clock::Interrupt;
use crate::queue::{self, Queue};
use crate::transport::{Transport, Windows};

/// evdev record types, by the numbers the device puts on the wire.
pub mod ev {
    /// The report boundary.
    pub const SYN: u16 = 0x00;
    /// A key or a button moved.
    pub const KEY: u16 = 0x01;
    /// A relative axis moved.
    pub const REL: u16 = 0x02;
    /// An absolute axis took a value. Not translated — `crate::driver`'s module
    /// comment says what that costs and why filling in a pressure nobody applied
    /// would be worse.
    pub const ABS: u16 = 0x03;
}

/// Codes within [`ev::SYN`].
pub mod syn {
    /// The one that ends a report. Every other `EV_SYN` code is counted and
    /// skipped.
    pub const REPORT: u16 = 0x00;
}

/// Codes within [`ev::REL`] that this driver reads.
pub mod rel {
    /// Movement along x.
    pub const X: u16 = 0x00;
    /// Movement along y.
    pub const Y: u16 = 0x01;
    /// A horizontal wheel or a two-finger horizontal drag.
    pub const HWHEEL: u16 = 0x06;
    /// The wheel.
    pub const WHEEL: u16 = 0x08;
}

/// Codes within [`ev::KEY`] that name a pointer button rather than a key.
pub mod btn {
    /// `BTN_LEFT`, and the bottom of the mouse-button range.
    pub const MOUSE: u16 = 0x110;
    /// `BTN_TASK`, and the top of it.
    ///
    /// Eight codes, which is the range evdev reserves for the buttons on a
    /// pointing device. Everything below it — `BTN_MISC` at `0x100` — and
    /// everything above it — joystick, gamepad, digitiser and wheel buttons —
    /// arrives as an `f_abi::input::Key` instead. That is a cost and it is
    /// stated rather than hidden: a gamepad's *south* button reaches a consumer
    /// as a key with an opaque code, which is exactly what it is on this wire
    /// and is not what a consumer that wanted a gamepad would want. There is no
    /// gamepad opcode in `f_abi::input` and inventing one here would be a format
    /// decided in a driver.
    pub const TASK: u16 = 0x117;
}

/// evdev key values.
mod value {
    /// The key came up.
    pub const RELEASED: u32 = 0;
    /// The key went down.
    pub const PRESSED: u32 = 1;
}

/// The fixed-point scale every coordinate on this wire carries.
///
/// `f_abi::input` states it in every field's name — `x_x65536` — and this is the
/// number those names mean. A relative device reports whole device pixels, so
/// every conversion in this file is a multiplication by this and never a
/// division, which is why there is no rounding anywhere in it.
/// Unit: fixed-point units per device pixel.
const SCALE: i32 = 65_536;

/// How far one wheel detent is taken to move the content.
///
/// Fifteen device pixels, and the number is a **convention rather than a
/// measurement** — which is worth saying plainly, because `f_abi::input::Scroll`
/// asks the driver to convert detents to distance on the grounds that *only the
/// driver has the device's detent resolution*, and this driver does not have it
/// either: the resolution is in the device's configuration space and
/// `crate::transport` says why this build does not read it.
///
/// Fifteen is taken from `f_abi::input::Scroll`'s own `SPECIMEN`, which is
/// `dy_x65536: -983_040` — one detent, negative, fifteen pixels — so the format's
/// author and this driver agree by construction rather than by coincidence.
///
/// *What would reverse this:* reading `VIRTIO_INPUT_CFG_ABS_INFO` and the
/// device's resolution, which is the same change that would let this driver
/// refuse a device it has no translation for. Until then a consumer that scrolls
/// by exactly this much per detent is doing what this driver said, and a user who
/// finds it too fast is looking at this constant.
/// Unit: device pixels per detent.
const DETENT_PIXELS: i32 = 15;

/// What this component did, as numbers the frame reads out of the routing page.
///
/// RFC 0013's *read, never delivered*: every field here is a number this crate
/// was already keeping, so publishing it is the store the driver was going to
/// make anyway and there is no collect step, no sampling interval and no second
/// copy that can disagree with the first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counters {
    /// `virtio_input_event` records read off the device. Unit: records.
    pub records: u64,
    /// Reports that closed — `SYN_REPORT` records that ended a report with
    /// something in it. Unit: reports.
    pub reports: u64,
    /// Reports that took a stamp. Unit: reports.
    ///
    /// **Equal to [`Counters::reports`] at rest, and greater by at most one in
    /// flight**: the stamp is taken when a report opens and the report is
    /// counted when it closes, so a device that has sent half a report has one
    /// more stamp than report. Published beside it rather than asserted equal,
    /// because the difference is the only thing that says a report was
    /// interrupted, and a build that made them one field could not say it.
    pub stamped: u64,
    /// Entries put on the data ring. Unit: entries.
    pub submitted: u64,
    /// Entries that were built and could not be submitted. Unit: entries.
    pub dropped: u64,
    /// Records this build has no opcode for. Unit: records.
    pub ignored: u64,
    /// Entries this component built that its own format check refused.
    /// **Required to be zero.** Unit: entries.
    pub malformed: u64,
    /// Turns of the component's loop that found nothing anywhere. Unit: turns.
    pub spun: u64,
}

/// The accumulator: evdev records in, `f_abi::input` entries out.
///
/// Holds no device and no ring, which is what makes every rule in this file
/// testable on a host with neither. What it holds is the pointer's position, the
/// scroll not yet flushed, and the stamp of the report in progress.
#[derive(Clone, Copy, Debug)]
pub struct Decoder {
    /// The stamp of the report in progress, taken when it opened.
    stamp: Option<StampNanos>,
    /// Where the pointer is. Unit: device pixels from an arbitrary origin,
    /// scaled by [`SCALE`].
    x_x65536: i32,
    /// The same along y. Unit: device pixels, scaled by [`SCALE`].
    y_x65536: i32,
    /// Whether this report moved the pointer.
    moved: bool,
    /// Scroll accumulated in this report, along x.
    /// Unit: device pixels, scaled by [`SCALE`].
    scroll_dx_x65536: i32,
    /// The same along y. Unit: device pixels, scaled by [`SCALE`].
    scroll_dy_x65536: i32,
    /// Whether this report scrolled.
    scrolled: bool,
    /// The class every entry this decoder builds carries.
    ///
    /// The component's own admitted class, out of the routing page — RFC 0025
    /// bound 2 from the submitting end: an entry claiming something more urgent
    /// than the channel reports about its submitter is refused, so the honest
    /// value is the one the frame admitted this component at and there is no
    /// field anywhere in this crate that could raise it.
    /// Unit: none — an `f_abi::class` field.
    class: u16,
    /// How many stamps this decoder has taken. Unit: reports.
    stamped: u64,
}

/// What one record produced.
///
/// Two entries at most, because a report carries at most one motion and at most
/// one scroll and a button record produces exactly one entry of its own. A third
/// would be a report shape this build does not produce, and the array's length is
/// what says so.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Emitted {
    /// The entries, in the order they are submitted.
    ///
    /// Their `payload_offset` is zero: the arena slot is the *ring's* business
    /// and this type does not hold a ring. [`Outbound::put`] is what fills it in,
    /// which is why these are handed out by value rather than submitted here.
    pub events: [Option<Event>; 2],
    /// This record was one this build has no opcode for.
    pub ignored: bool,
    /// This record closed a report that had something in it.
    pub closed_report: bool,
}

impl Decoder {
    /// A decoder whose pointer is at `(0, 0)` and whose entries carry `class`.
    #[must_use]
    pub const fn new(class: u16) -> Self {
        Self::starting_at(class, (0, 0))
    }

    /// A decoder whose pointer starts at `origin`, `(x, y)`, and whose entries
    /// carry `class`. The module's *origin of the pointer* is why it is told.
    /// Unit: `origin` is device pixels, scaled by [`SCALE`].
    #[must_use]
    pub const fn starting_at(class: u16, origin: (i32, i32)) -> Self {
        Self {
            stamp: None,
            x_x65536: origin.0,
            y_x65536: origin.1,
            moved: false,
            scroll_dx_x65536: 0,
            scroll_dy_x65536: 0,
            scrolled: false,
            class,
            stamped: 0,
        }
    }

    /// How many stamps this decoder has taken. Unit: reports.
    #[must_use]
    pub const fn stamped(&self) -> u64 {
        self.stamped
    }

    /// Where the pointer is.
    /// Unit: device pixels from the origin, scaled by [`SCALE`].
    #[must_use]
    pub const fn at(&self) -> (i32, i32) {
        (self.x_x65536, self.y_x65536)
    }

    /// The stamp of the report in progress, opening one if none is.
    ///
    /// The **only** place in this crate that asks for a stamp, which is what
    /// makes *one stamp per report* a property of this function rather than of
    /// every caller agreeing.
    fn opened(&mut self, clock: &mut Interrupt) -> StampNanos {
        match self.stamp {
            Some(stamp) => stamp,
            None => {
                let stamp = clock.stamp();
                self.stamped = self.stamped.saturating_add(1);
                self.stamp = Some(stamp);
                stamp
            }
        }
    }

    /// One entry, with the report's stamp and this component's class.
    const fn entry(&self, stamp: StampNanos, body: Entry) -> Event {
        Event {
            // Zero, and it means *nothing is waiting for this*. Every entry
            // carries `NO_CQE`, so there is no completion to match a token
            // against; a driver that put a counter here would be inventing a
            // correlation nobody reads. `f_abi::input` calls zero a legal token
            // and this is the case it had in mind.
            user_data: 0,
            class: self.class,
            // Filled in by `Outbound::put`, which is the only thing that knows
            // where in the arena this payload will land.
            payload_offset: 0,
            flags: input::FLAGS_ACCEPTED,
            stamp_nanos: stamp.nanos(),
            body,
        }
    }

    /// Feed one `virtio_input_event`.
    ///
    /// `record` is `(type, code, value)` as the device wrote it. The `value` is
    /// carried as a `u32` because that is its width on the wire and read as an
    /// `i32` where the record is a relative axis, which is what evdev means by
    /// it — a cast and not a conversion, because a mouse moving left is a very
    /// large unsigned number and a small negative signed one, and the second is
    /// the one that is true.
    pub fn feed(&mut self, clock: &mut Interrupt, record: (u16, u16, u32)) -> Emitted {
        let (kind, code, value) = record;
        let mut out = Emitted::default();
        match kind {
            ev::SYN if code == syn::REPORT => {
                // The boundary. A report with nothing in it — no stamp was ever
                // opened — closes nothing and is not counted, because *the user
                // did something* is what a report means and an empty one is the
                // device punctuating silence.
                let Some(stamp) = self.stamp.take() else { return out };
                out.closed_report = true;
                let mut at = 0;
                if self.moved {
                    self.moved = false;
                    out.events[at] = Some(self.entry(
                        stamp,
                        Entry::PointerMotion(PointerMotion {
                            x_x65536: self.x_x65536,
                            y_x65536: self.y_x65536,
                        }),
                    ));
                    at += 1;
                }
                if self.scrolled {
                    self.scrolled = false;
                    let body = Entry::Scroll(Scroll {
                        dx_x65536: self.scroll_dx_x65536,
                        dy_x65536: self.scroll_dy_x65536,
                        // `WHEEL` and never `FINGER`, because a relative device
                        // reporting `REL_WHEEL` is a wheel: a touchpad's
                        // two-finger drag arrives as `EV_ABS` on this transport
                        // and is not translated at all. A driver that guessed
                        // `FINGER` here would be telling a consumer to glide
                        // where it should snap.
                        source: axis_source::WHEEL,
                    });
                    self.scroll_dx_x65536 = 0;
                    self.scroll_dy_x65536 = 0;
                    out.events[at] = Some(self.entry(stamp, body));
                }
                out
            }
            ev::KEY => {
                let transition = match value {
                    value::PRESSED => edge::PRESSED,
                    value::RELEASED => edge::RELEASED,
                    // Autorepeat, and everything else the device might mean.
                    // The module comment argues why a repeat is a policy rather
                    // than an event.
                    _ => {
                        out.ignored = true;
                        return out;
                    }
                };
                let stamp = self.opened(clock);
                let body = if (btn::MOUSE..=btn::TASK).contains(&code) {
                    Entry::PointerButton(PointerButton { button: u32::from(code), transition })
                } else {
                    Entry::Key(Key { code: u32::from(code), transition })
                };
                out.events[0] = Some(self.entry(stamp, body));
                out
            }
            ev::REL => {
                let delta = value as i32;
                match code {
                    rel::X => {
                        let _ = self.opened(clock);
                        self.x_x65536 = self.x_x65536.saturating_add(delta.saturating_mul(SCALE));
                        self.moved = true;
                    }
                    rel::Y => {
                        let _ = self.opened(clock);
                        self.y_x65536 = self.y_x65536.saturating_add(delta.saturating_mul(SCALE));
                        self.moved = true;
                    }
                    rel::WHEEL => {
                        let _ = self.opened(clock);
                        // Negated. A wheel rolled away from the user is a
                        // positive `REL_WHEEL` and moves the *content* up the
                        // screen, and `f_abi::input::Scroll` carries the
                        // direction the content moves. The two conventions
                        // differ by exactly this sign and the format says so;
                        // the tie-break is its own `SPECIMEN`, which is one
                        // forward detent as a negative `dy`.
                        self.scroll_dy_x65536 = self.scroll_dy_x65536.saturating_sub(
                            delta.saturating_mul(DETENT_PIXELS).saturating_mul(SCALE),
                        );
                        self.scrolled = true;
                    }
                    rel::HWHEEL => {
                        let _ = self.opened(clock);
                        self.scroll_dx_x65536 = self.scroll_dx_x65536.saturating_add(
                            delta.saturating_mul(DETENT_PIXELS).saturating_mul(SCALE),
                        );
                        self.scrolled = true;
                    }
                    _ => out.ignored = true,
                }
                out
            }
            _ => {
                out.ignored = true;
                out
            }
        }
    }
}

/// The ring this component submits on, and the arena discipline that goes with
/// it.
///
/// # Why the slot is this component's arithmetic and not the ring's
///
/// Because the ring has no allocator and says so: `f_ring::adopt::Client::copy_in`
/// states the caller's obligation as *do not write where an entry the peer has
/// not taken is pointing*. A client that wrote every payload at one offset would
/// have to wait for a completion between entries, and these entries carry
/// `NO_CQE` — there is no completion to wait for and nothing is waiting. So the
/// payloads go in a ring of their own, one slot per entry the channel can hold,
/// and the occupancy check below is what keeps the two rings in step.
pub struct Outbound {
    client: Client,
    /// How many payload slots the arena holds, and therefore the modulus of the
    /// cursor below. Unit: slots.
    slots: u32,
    /// The next slot to write. Unit: none — a slot index.
    next: u32,
    /// What this driver has put on the ring, folded.
    ///
    /// The **producer's half of the attestation**, and it is here rather than
    /// on [`Driver`] because here is the one place that knows an entry reached
    /// the ring: an entry the peer had no room for is counted in
    /// [`Counters::dropped`] and never crossed, and folding it would make this
    /// word a record of what this component built rather than of what it sent.
    /// A consumer holding the second half can then tell a relay that dropped
    /// one from a driver that never submitted it — which are the same symptom
    /// with different repairs, which is the argument
    /// `user/virtio-input/manifest.toml` already makes for publishing
    /// `dropped`.
    /// Unit: none — an `f_abi::input::Crossing`.
    crossing: Crossing,
    /// How many of the entries that crossed were pointer motion.
    ///
    /// Counted here for [`Outbound::crossing`]'s reason — this is the one place
    /// that knows an entry reached the ring — and published so that a boot can
    /// ask how many positions the consumer should have been handed without
    /// asking the consumer. Unit: entries.
    motions: u64,
    /// Whether the fold has been put on the ring as an attestation.
    ///
    /// Once, and after the last event: `f_abi::input::ATTEST` is why the word
    /// travels on the ring at all, and an attestation submitted twice would be
    /// two answers to one question with the later one winning.
    attested: bool,
}

impl Outbound {
    /// State the arena as a ring of payload slots.
    ///
    /// The count is the smaller of the channel's entries and what the arena
    /// holds, because either can be the binding one: a channel with more entries
    /// than payload slots would submit an entry pointing at a slot it has just
    /// overwritten, and one with more slots than entries would leave them unused.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a channel whose arena holds no whole payload,
    /// which is a peer that agreed a channel this protocol cannot be spoken on.
    pub fn over(client: Client) -> Result<Self, Trouble> {
        let layout = client.channel().layout();
        let holds = layout.arena_len() / (input::PAYLOAD_BYTES as u32);
        let slots = layout.entries().min(holds);
        if slots == 0 {
            return Err(Trouble::Layout);
        }
        Ok(Self { client, slots, next: 0, crossing: Crossing::new(), motions: 0, attested: false })
    }

    /// How many payload slots there are. Unit: slots.
    #[must_use]
    pub const fn slots(&self) -> u32 {
        self.slots
    }

    /// What has crossed, folded. Unit: none — an `f_abi::input::Crossing`.
    #[must_use]
    pub const fn crossing(&self) -> Crossing {
        self.crossing
    }

    /// Put one entry on the ring.
    ///
    /// The order is the only one that is safe: the occupancy check, then the
    /// payload, then the entry. Writing the payload first and finding the ring
    /// full would have overwritten a slot a submitted entry still points at;
    /// submitting first and copying after would have published an entry pointing
    /// at bytes that are not yet the payload, and the peer is another core.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoRoom`] when the peer has not drained, which is an event this
    /// system loses and counts. [`Trouble::Layout`] when the entry does not pass
    /// `f_abi::input::Event::check` — this component about to submit something
    /// its peer would refuse, caught at the one place that still knows which
    /// record produced it.
    pub fn put(&mut self, event: &Event) -> Result<(), Trouble> {
        let queued = self.client.queued().map_err(|_| Trouble::NoRoom)?;
        if queued >= self.slots {
            return Err(Trouble::NoRoom);
        }
        let offset = self.next.saturating_mul(input::PAYLOAD_BYTES as u32);
        let event = Event { payload_offset: offset, ..*event };
        if event.check().is_err() {
            return Err(Trouble::Layout);
        }
        let (entry, payload) = event.encode();
        if !self.client.copy_in(offset as usize, &payload) {
            return Err(Trouble::Layout);
        }
        self.client.submit(entry).map_err(|_| Trouble::NoRoom)?;
        // After the submit and not before it, which is the whole discipline of
        // this field: an entry the ring refused did not cross, and a producer
        // whose word counted it would be attesting to something its consumer
        // was never sent. The event folded is the one that went out — with the
        // offset this function chose — and the fold deliberately ignores that
        // offset, so the word survives being carried in a second channel.
        self.crossing.absorb(&event);
        if matches!(event.body, Entry::PointerMotion(_)) {
            self.motions = self.motions.saturating_add(1);
        }
        self.next = (self.next + 1) % self.slots;
        Ok(())
    }

    /// How many pointer-motion entries crossed. Unit: entries.
    #[must_use]
    pub const fn motions(&self) -> u64 {
        self.motions
    }

    /// Whether the fold is on the ring.
    #[must_use]
    pub const fn attested(&self) -> bool {
        self.attested
    }

    /// Put the fold on the ring, after the last event, for a consumer that is
    /// not the frame.
    ///
    /// **The producer's half of `E3-B04g`.** `E3-B04f` published this word on
    /// the routing page and the frame compared it, because the frame was the
    /// consumer and reads that page before taking it back. The consumer is now
    /// `user/compositor`, which never sees this component's page and — on one
    /// worker core — runs after the frame has reaped it. So the word goes where
    /// the entries went. It still goes on the page as well: the frame compares
    /// the compositor's fold against that copy, so the word reaches the check
    /// by two routes and a ring that carried it wrongly is a disagreement
    /// between them.
    ///
    /// No payload and no arena slot, so the occupancy check is the ring's own:
    /// a ring with no room refuses the submit and this answers
    /// [`Trouble::NoRoom`], which the caller publishes as *not attested* rather
    /// than retrying — a consumer that finds no attestation says so, and that is
    /// the right sentence for a ring that was full.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoRoom`] when the ring is full or the fold is already there.
    pub fn attest(&mut self, class: u16) -> Result<(), Trouble> {
        if self.attested {
            return Err(Trouble::NoRoom);
        }
        self.client.submit(self.crossing.attestation(class)).map_err(|_| Trouble::NoRoom)?;
        self.attested = true;
        Ok(())
    }
}

/// The device, the queue, the clock and the ring, as one thing that runs.
pub struct Driver {
    transport: Transport,
    queue: Queue,
    clock: Interrupt,
    decoder: Decoder,
    out: Outbound,
    counters: Counters,
}

impl Driver {
    /// Bring the device up with every buffer already posted.
    ///
    /// The order is load-bearing and is the one thing this `start` does that the
    /// other three drivers' do not have to: the buffers are posted **before**
    /// `DRIVER_OK`. A user pressing a key one microsecond after the device is
    /// told the driver is ready is a device with a record to write, and a queue
    /// with nothing in its available ring is a record dropped inside the device
    /// where nothing here can count it.
    ///
    /// # Errors
    ///
    /// Whatever [`Transport::open`] and [`Queue::over`] refuse, and
    /// [`Trouble::Layout`] for a granted region too small for the layout.
    pub fn start(
        windows: Windows,
        queues: Region,
        client: Client,
        clock: Interrupt,
        class: u16,
        origin: (i32, i32),
    ) -> Result<Self, Trouble> {
        let transport = Transport::open(windows, queue::QUEUE_SIZE)?;
        let region = queues.slice(0, queue::QUEUE_BYTES).map_err(Trouble::from)?;
        let mut ring = Queue::over(region, transport.size())?;
        transport.queue_at(ring.device_desc()?, ring.device_avail()?, ring.device_used()?)?;
        for index in 0..ring.size() {
            ring.post(index)?;
        }
        transport.run()?;
        transport.kick()?;
        let out = Outbound::over(client)?;
        Ok(Self {
            transport,
            queue: ring,
            clock,
            decoder: Decoder::starting_at(class, origin),
            out,
            counters: Counters::default(),
        })
    }

    /// Take everything the device has written, translate it, and submit it.
    ///
    /// Answers how many records were read, so that the component's loop can tell
    /// an idle turn from a busy one without a second question.
    ///
    /// # Errors
    ///
    /// [`Trouble::ShortUsed`] for a device that wrote less than one record,
    /// [`Trouble::Layout`] for one that named a buffer outside the queue, and
    /// [`Trouble::Register`] for a window or region that refused. A full peer
    /// ring is **not** an error here: it is counted in
    /// [`Counters::dropped`] and the loop goes on, because an input driver that
    /// stopped when a compositor fell behind would turn a slow frame into a dead
    /// machine.
    pub fn drain(&mut self) -> Result<u32, Trouble> {
        let mut records = 0;
        while let Some(filled) = self.queue.harvest()? {
            if filled.written < queue::RECORD_BYTES {
                return Err(Trouble::ShortUsed);
            }
            let record = self.queue.record(filled.head)?;
            self.counters.records = self.counters.records.saturating_add(1);
            records += 1;

            let emitted = self.decoder.feed(&mut self.clock, record);
            if emitted.ignored {
                self.counters.ignored = self.counters.ignored.saturating_add(1);
            }
            if emitted.closed_report {
                self.counters.reports = self.counters.reports.saturating_add(1);
            }
            for event in emitted.events.iter().flatten() {
                match self.out.put(event) {
                    Ok(()) => self.counters.submitted = self.counters.submitted.saturating_add(1),
                    Err(Trouble::NoRoom) => {
                        self.counters.dropped = self.counters.dropped.saturating_add(1);
                    }
                    Err(_) => self.counters.malformed = self.counters.malformed.saturating_add(1),
                }
            }

            // Back to the device immediately, and before the next harvest: a
            // driver that reposted at the end of the pass would spend the whole
            // pass one buffer shorter for every record it had taken, which is
            // exactly the window in which a fast user overruns the queue.
            self.queue.post(filled.head)?;
        }
        if records > 0 {
            self.transport.kick()?;
        }
        self.counters.stamped = self.decoder.stamped();
        Ok(records)
    }

    /// Record a turn of the component's loop that found nothing.
    pub fn spun(&mut self) {
        self.counters.spun = self.counters.spun.saturating_add(1);
        // Read and discarded. The value says whether the device raised its
        // interrupt and this build has no interrupt to take; what the read is
        // for is the exit to the emulator, which is the point at which the
        // device's own work can make progress. `crate::transport::Transport::poke`
        // is the long version.
        let _ = self.transport.poke();
    }

    /// What this component did.
    #[must_use]
    pub const fn counters(&self) -> Counters {
        self.counters
    }

    /// How far the clock has been advanced. Unit: nanoseconds of virtual time.
    #[must_use]
    pub const fn clock_at_nanos(&self) -> u64 {
        self.clock.at_nanos()
    }

    /// What this driver put on the ring, folded into one word.
    ///
    /// Not a [`Counters`] field, and the reason is the manifest's: every node
    /// this component declares is one of its own counters, one for one, so that
    /// publishing the tree is the store it was already going to make. This is
    /// not a count of anything — it is a checksum — so it goes where
    /// [`Driver::clock_at_nanos`] goes, onto the routing page the frame reads
    /// after the run, and the `[[state]]` list is left alone.
    ///
    /// A consumer that is not the frame cannot read that page — the frame takes
    /// it back when this component is reaped — and `E3-B04g`'s consumer is
    /// `user/compositor`, so the same word also goes onto the ring after the
    /// last event: [`Driver::attest`]. The ring and not the tree, and RFC 0125
    /// says why; the page copy stays, because the frame compares the
    /// compositor's fold against it, which makes the ring's copy checkable.
    /// Unit: none — an `f_abi::input::Crossing`.
    #[must_use]
    pub const fn crossing(&self) -> Crossing {
        self.out.crossing()
    }

    /// How many pointer-motion entries crossed. Unit: entries.
    #[must_use]
    pub const fn motions(&self) -> u64 {
        self.out.motions()
    }

    /// Where the accumulator ended up.
    ///
    /// Published for the harness that moved the pointer, which holds the other
    /// half of the comparison — how far it asked for — and for the frame, which
    /// compares it against the position a compositor latched after taking the
    /// entries off the ring itself. Neither of those two holds this number
    /// except by reading it here, and neither of them is a stage on the input
    /// path: a position is not a reading.
    /// Unit: device pixels from the origin, scaled by [`SCALE`].
    #[must_use]
    pub const fn at(&self) -> (i32, i32) {
        self.decoder.at()
    }

    /// Whether the fold went onto the ring as an attestation.
    #[must_use]
    pub const fn attested(&self) -> bool {
        self.out.attested()
    }

    /// Put what crossed onto the ring as an attestation, after the last event.
    ///
    /// [`Outbound::attest`] is the argument. Called once, when the run ends,
    /// and not per report: the consumer compares the word against a fold of
    /// everything it drained, and a word covering half the run would be
    /// compared against the other half.
    ///
    /// # Errors
    ///
    /// [`Trouble::NoRoom`], for a ring with no room or a second call.
    pub fn attest(&mut self) -> Result<(), Trouble> {
        self.out.attest(self.decoder.class)
    }

    /// Put the device back in reset, so that it stops writing into memory the
    /// frame is about to take back.
    ///
    /// `crate::transport::Transport::reset` argues why this is the ordinary
    /// teardown here and is refused on the display driver.
    pub fn stop(&self) {
        let _ = self.transport.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A tick a reader can do arithmetic on by eye. Unit: nanoseconds.
    const TICK_NANOS: u64 = 1_000;

    /// The class a test decoder's entries carry.
    ///
    /// `f_abi::class::SOFT`'s ordinal would be the honest production value and
    /// any ordinal does here; what the tests check is that whatever it is
    /// reaches the entry unchanged, which is the half a wrong constant would not
    /// break.
    const CLASS: u16 = 2;

    fn decoder() -> (Decoder, Interrupt) {
        (Decoder::new(CLASS), Interrupt::new(0x1_4E17, TICK_NANOS).expect("a tick"))
    }

    /// Feed a whole report and collect what it produced.
    fn report(
        decoder: &mut Decoder,
        clock: &mut Interrupt,
        records: &[(u16, u16, u32)],
    ) -> [Option<Event>; 2] {
        let mut last = [None, None];
        for record in records {
            let emitted = decoder.feed(clock, *record);
            for event in emitted.events.iter().flatten() {
                // A report's own entries, and there are at most two of them —
                // the button entries come out on their own records, which the
                // tests that want them read one at a time.
                if last[0].is_none() {
                    last[0] = Some(*event);
                } else {
                    last[1] = Some(*event);
                }
            }
        }
        last
    }

    #[test]
    fn a_diagonal_movement_is_one_entry_carrying_a_position() {
        // The property the whole file is organised around. Three records, one
        // motion, one stamp — and a *position*, because the accumulator is here
        // and nowhere later.
        let (mut decoder, mut clock) = decoder();
        let out = report(
            &mut decoder,
            &mut clock,
            &[(ev::REL, rel::X, 3), (ev::REL, rel::Y, -2i32 as u32), (ev::SYN, syn::REPORT, 0)],
        );
        let event = out[0].expect("one motion");
        assert!(out[1].is_none(), "a report carries one motion and no more");
        assert_eq!(
            event.body,
            Entry::PointerMotion(PointerMotion { x_x65536: 3 * SCALE, y_x65536: -2 * SCALE })
        );
        assert_eq!(event.stamp_nanos, TICK_NANOS, "one report is one stamp");
        assert_eq!(decoder.stamped(), 1);
        assert_eq!(event.class, CLASS);
        assert_eq!(event.flags, input::FLAGS_ACCEPTED);
        event.check().expect("an entry this build would accept");

        // And the second report continues from where the first left off, which
        // is what *position and not displacement* means when it is a driver's
        // job rather than a sentence.
        let out =
            report(&mut decoder, &mut clock, &[(ev::REL, rel::X, 1), (ev::SYN, syn::REPORT, 0)]);
        let event = out[0].expect("one motion");
        assert_eq!(
            event.body,
            Entry::PointerMotion(PointerMotion { x_x65536: 4 * SCALE, y_x65536: -2 * SCALE })
        );
        assert_eq!(event.stamp_nanos, 2 * TICK_NANOS, "the second report is the second stamp");
        assert_eq!(decoder.stamped(), 2);
    }

    #[test]
    fn a_told_origin_is_where_the_first_motion_starts_from() {
        // RFC 0132. The first entry is the told origin plus the motion, and not
        // the motion: a decoder that ignored the origin would pass every other
        // test here, which all start at zero, and would put the boot's pointer
        // where the frame did not commit it.
        let origin = (640 * SCALE, -360 * SCALE);
        let mut decoder = Decoder::starting_at(CLASS, origin);
        let mut clock = Interrupt::new(0x1_4E17, TICK_NANOS).expect("a tick");
        assert_eq!(decoder.at(), origin, "nothing has moved, so the pointer is where it was told");
        let out = report(
            &mut decoder,
            &mut clock,
            &[(ev::REL, rel::X, 3), (ev::REL, rel::Y, -2i32 as u32), (ev::SYN, syn::REPORT, 0)],
        );
        assert_eq!(
            out[0].expect("one motion").body,
            Entry::PointerMotion(PointerMotion { x_x65536: 643 * SCALE, y_x65536: -362 * SCALE })
        );
        assert_eq!(decoder.at(), (643 * SCALE, -362 * SCALE));
    }

    #[test]
    fn every_entry_of_one_report_bears_the_same_stamp() {
        // `abi/src/input.rs`: two entries bearing one stamp are one instant,
        // because there is only one clock reading per instant to bear. A driver
        // that stamped per record would report a movement and a click that
        // happened together as one before the other.
        let (mut decoder, mut clock) = decoder();
        let click = decoder.feed(&mut clock, (ev::KEY, btn::MOUSE, value::PRESSED));
        let button = click.events[0].expect("a button entry");
        let out =
            report(&mut decoder, &mut clock, &[(ev::REL, rel::X, 5), (ev::SYN, syn::REPORT, 0)]);
        let motion = out[0].expect("a motion entry");
        assert_eq!(button.stamp_nanos, motion.stamp_nanos);
        assert_eq!(decoder.stamped(), 1, "one report, one reading of the clock");
    }

    #[test]
    fn a_motion_and_a_scroll_in_one_report_are_two_entries_and_one_stamp() {
        let (mut decoder, mut clock) = decoder();
        let out = report(
            &mut decoder,
            &mut clock,
            &[(ev::REL, rel::X, 1), (ev::REL, rel::WHEEL, 1), (ev::SYN, syn::REPORT, 0)],
        );
        let motion = out[0].expect("a motion");
        let scroll = out[1].expect("a scroll");
        assert_eq!(motion.stamp_nanos, scroll.stamp_nanos);
        assert_eq!(
            scroll.body,
            Entry::Scroll(Scroll {
                dx_x65536: 0,
                dy_x65536: -DETENT_PIXELS * SCALE,
                source: axis_source::WHEEL,
            }),
            "one forward detent is the format's own SPECIMEN, sign and magnitude"
        );
    }

    #[test]
    fn a_scroll_is_flushed_and_does_not_accumulate_across_reports() {
        // The half a motion does not have: a position persists and a distance
        // does not. A driver that forgot to clear the accumulator would report
        // every scroll as the sum of every scroll before it.
        let (mut decoder, mut clock) = decoder();
        for _ in 0..3 {
            let out = report(
                &mut decoder,
                &mut clock,
                &[(ev::REL, rel::WHEEL, 1), (ev::SYN, syn::REPORT, 0)],
            );
            let scroll = out[0].expect("a scroll");
            assert_eq!(
                scroll.body,
                Entry::Scroll(Scroll {
                    dx_x65536: 0,
                    dy_x65536: -DETENT_PIXELS * SCALE,
                    source: axis_source::WHEEL,
                })
            );
        }
    }

    #[test]
    fn a_mouse_button_is_a_pointer_button_and_a_key_is_a_key() {
        let (mut decoder, mut clock) = decoder();
        let out = decoder.feed(&mut clock, (ev::KEY, btn::MOUSE, value::PRESSED));
        assert_eq!(
            out.events[0].expect("an entry").body,
            Entry::PointerButton(PointerButton {
                button: u32::from(btn::MOUSE),
                transition: edge::PRESSED
            })
        );
        // `KEY_A` is 30, which is `f_abi::input::Key::SPECIMEN`'s own code.
        let out = decoder.feed(&mut clock, (ev::KEY, 30, value::RELEASED));
        assert_eq!(
            out.events[0].expect("an entry").body,
            Entry::Key(Key { code: 30, transition: edge::RELEASED })
        );
        // And the boundary, both sides of it, because an off-by-one here puts a
        // gamepad's buttons on the pointer.
        let out = decoder.feed(&mut clock, (ev::KEY, btn::TASK, value::PRESSED));
        assert!(matches!(out.events[0].expect("an entry").body, Entry::PointerButton(_)));
        let out = decoder.feed(&mut clock, (ev::KEY, btn::TASK + 1, value::PRESSED));
        assert!(matches!(out.events[0].expect("an entry").body, Entry::Key(_)));
        let out = decoder.feed(&mut clock, (ev::KEY, btn::MOUSE - 1, value::PRESSED));
        assert!(matches!(out.events[0].expect("an entry").body, Entry::Key(_)));
    }

    #[test]
    fn what_this_build_cannot_translate_is_counted_and_never_forged() {
        // The module comment's refusal, as arithmetic. An `EV_ABS` record has no
        // honest `TouchPoint` in it — no contact identifier, no pressure, no tilt
        // — so nothing is produced, the record is counted, and no clock is read:
        // a report that produced nothing did not happen.
        let (mut decoder, mut clock) = decoder();
        for record in [
            (ev::ABS, 0x00, 512),
            (ev::KEY, 30, 2),
            (ev::REL, 0x09, 1),
            (0x04, 0x04, 1),
            (ev::SYN, 0x03, 0),
        ] {
            let out = decoder.feed(&mut clock, record);
            assert!(out.ignored, "{record:?} is not translated and must say so");
            assert!(out.events.iter().all(Option::is_none), "{record:?} forged an entry");
        }
        assert_eq!(decoder.stamped(), 0, "nothing translated is nothing stamped");
    }

    #[test]
    fn an_empty_report_closes_nothing() {
        // A device punctuating silence. Counting it would put a report in the
        // tally with no entry behind it, and `reports` would stop being the
        // number of things the user did.
        let (mut decoder, mut clock) = decoder();
        let out = decoder.feed(&mut clock, (ev::SYN, syn::REPORT, 0));
        assert!(!out.closed_report);
        assert!(out.events.iter().all(Option::is_none));
        assert_eq!(decoder.stamped(), 0);
    }

    #[test]
    fn every_entry_this_driver_produces_survives_the_decoder_a_consumer_runs() {
        // The delivery half, as far as a crate that forbids `unsafe` can reach
        // it. **What this covers:** every entry this driver builds is one
        // `f_abi::input::Event::decode` accepts and rebuilds byte for byte —
        // opcode, flags, stamp, class, offset and the record's own fields — so a
        // consumer draining this driver's ring gets back what the decoder in
        // this file put on it.
        //
        // **What it does not cover, said rather than left to be assumed:** the
        // ring. Building a channel means `f_ring::Mapping::describe`, which is
        // `unsafe`, and this crate forbids `unsafe` at the workspace's
        // insistence — so the arena discipline in `Outbound` is checked by a
        // boot and not here. `E3-B04d`'s notes say which boot.
        let (mut decoder, mut clock) = decoder();
        let records = [
            (ev::REL, rel::X, 7),
            (ev::REL, rel::Y, (-3i32) as u32),
            (ev::SYN, syn::REPORT, 0),
            (ev::KEY, btn::MOUSE, value::PRESSED),
            (ev::KEY, btn::MOUSE, value::RELEASED),
            (ev::KEY, 30, value::PRESSED),
            (ev::REL, rel::WHEEL, (-2i32) as u32),
            (ev::REL, rel::HWHEEL, 1),
            (ev::SYN, syn::REPORT, 0),
        ];
        let mut crossed = 0;
        // The two halves of the attestation, kept apart on purpose: `sent` is
        // folded from what this driver built, `arrived` from what came back out
        // of the bytes. `Outbound` cannot be stood up here — the ring needs
        // `unsafe` and this crate has none — so what this test covers is the
        // fold either side of an encode and a decode, and the boot covers the
        // fold either side of a real channel.
        let mut sent_fold = Crossing::new();
        let mut arrived_fold = Crossing::new();
        for record in records {
            for event in decoder.feed(&mut clock, record).events.iter().flatten() {
                // The offset a submitter would have filled in. Anything but zero
                // does, and a non-zero one is what makes the envelope comparison
                // in `decode` a check rather than a comparison of zeroes.
                let sent = Event { payload_offset: 3 * (input::PAYLOAD_BYTES as u32), ..*event };
                let (entry, payload) = sent.encode();
                let back = Event::decode(&entry, &payload).expect("a consumer accepts it");
                assert_eq!(back, sent, "what a consumer reads is what this driver wrote");
                assert_ne!(back.stamp_nanos, input::NOT_STAMPED);
                sent_fold.absorb(&sent);
                arrived_fold.absorb(&back);
                crossed += 1;
            }
        }
        assert_eq!(crossed, 5, "two reports and three key records produce five entries");
        assert!(
            arrived_fold.agrees_with(sent_fold.word()),
            "the stamps and the bodies this driver wrote are the ones a consumer reads back"
        );
        assert_eq!(arrived_fold.absorbed(), crossed);
    }

    #[test]
    fn a_report_this_driver_did_not_send_is_not_in_its_word() {
        // The clause `Outbound::put` folds after the submit for. A consumer
        // holds the second half of this word, so a producer that folded an
        // entry the ring refused would be attesting to something nobody was
        // sent — and the boot would then go red at the consumer, blaming a
        // relay for a drop that happened before the crossing. The two counts
        // are the difference between those two repairs.
        // Both decoders taken before either name shadows the helper, so the two
        // runs start from one seed and differ only in what was folded.
        let ((mut decoder, mut clock), (mut again, mut again_clock)) = (decoder(), decoder());
        let mut sent_fold = Crossing::new();
        let mut built = 0;
        for record in [
            (ev::REL, rel::X, 4),
            (ev::SYN, syn::REPORT, 0),
            (ev::REL, rel::Y, 9),
            (ev::SYN, syn::REPORT, 0),
        ] {
            for event in decoder.feed(&mut clock, record).events.iter().flatten() {
                built += 1;
                // The second report is the one the peer had no room for.
                if built < 2 {
                    sent_fold.absorb(event);
                }
            }
        }
        assert_eq!(built, 2);
        assert_eq!(sent_fold.absorbed(), 1, "one of the two entries crossed");

        let mut both = Crossing::new();
        for record in [
            (ev::REL, rel::X, 4),
            (ev::SYN, syn::REPORT, 0),
            (ev::REL, rel::Y, 9),
            (ev::SYN, syn::REPORT, 0),
        ] {
            for event in again.feed(&mut again_clock, record).events.iter().flatten() {
                both.absorb(event);
            }
        }
        assert!(
            !both.agrees_with(sent_fold.word()),
            "a word that counted a dropped entry is not the word a consumer folds"
        );
    }

    #[test]
    fn a_movement_that_would_wrap_the_position_saturates_instead() {
        // Not a clamp to a surface — the module comment says why there is none —
        // but the arithmetic refusing to wrap, which would move the pointer to
        // the other side of the world on a device that kept going one way.
        let (mut decoder, mut clock) = decoder();
        for _ in 0..8 {
            let _ = report(
                &mut decoder,
                &mut clock,
                &[(ev::REL, rel::X, i32::MAX as u32), (ev::SYN, syn::REPORT, 0)],
            );
        }
        assert_eq!(decoder.at().0, i32::MAX);
        let out = report(
            &mut decoder,
            &mut clock,
            &[(ev::REL, rel::X, (-1i32) as u32), (ev::SYN, syn::REPORT, 0)],
        );
        let event = out[0].expect("a motion");
        assert_eq!(
            event.body,
            Entry::PointerMotion(PointerMotion { x_x65536: i32::MAX - SCALE, y_x65536: 0 })
        );
    }
}
