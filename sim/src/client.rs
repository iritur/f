// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One client, and the substitution claim it exists to collect.
//!
//! # What component substitution actually asks for
//!
//! `docs/design/proving-ground.html` gives the reason drivers in user space are
//! worth something to a simulator: *hardware already sits behind a component
//! boundary, so a simulated device is a component substitution rather than a
//! kernel patch*. That sentence is a promise about the **client**, not about the
//! device — it says the code above the boundary does not change when what is
//! below it does. A file with only device models could not demonstrate it, and a
//! client written inside each device model would not be one client.
//!
//! So there is exactly one of these. It addresses its peer by
//! [`ActorId`](crate::ActorId) and knows nothing else about it: not what
//! protocol the peer speaks below the ring, not whether there is a queue behind
//! it, not whether there is a device at all. [`crate::blk`], [`crate::net`],
//! [`crate::gpu`] and [`crate::native`] are four things it can be pointed at,
//! the scenario picks which, and the source below does not mention any of them.
//! [`crate::scenario::Peer`] is where the choice is visible, which is where a
//! reader looking for it will go.
//!
//! # The buffers are the real ones
//!
//! [`BufferSet`], [`Idle`] and [`InFlight`] are `f_ring::buffers`, unmodified.
//! That is the point of RFC 0024 collected rather than restated: while a buffer
//! is in flight there is no method here that reaches its bytes, because the type
//! does not have one, and this file could not have been written otherwise.
//! `f_ring::registry::Table` is the other half, in the peer.
//!
//! The one thing that had to be arranged is lifetime. A `BufferSet` owns its
//! region for `'m` and [`BufferSet::carve`] borrows the set for the same `'m`,
//! so a set and the buffers carved from it cannot both be fields of a value that
//! moves — which every actor is. The region and the set are therefore *leaked*,
//! at `'static`, and the leak is not a workaround: a component's buffer region
//! is granted for the life of the component and never handed back, and a
//! simulated component's life is the run. RFC 0008 is what reclaims it on the
//! real path, and the run ending is what reclaims it here.
//!
//! RFC 0034 is where the leak, the drop and the reason for both are argued in
//! full, including the finding below.
//!
//! # A dropped completion is a buffer nobody can take back
//!
//! The first thing driving the real types found. RFC 0024 lets a client take a
//! buffer back without a completion in exactly one situation — `PeerGone`,
//! built only from evidence that the peer's outstanding tokens are all void —
//! so a device that loses a completion and stays alive leaves its client holding
//! memory it may never touch again and may never free. That is not a modelling
//! artefact; it is the protocol, and it is why every device model here follows a
//! lost completion with a reset ([`kind::GONE`]) rather than with silence.

use std::collections::VecDeque;

use f_abi::{ABI_VERSION, Cqe, Negotiated, Sqe, error};
use f_ring::RingError;
use f_ring::buffers::{BufferSet, Fixed, Idle, InFlight, PeerGone, Refused, Submitter};
use f_ring::registry::registration;

use crate::fault::Class;
use crate::proto::{kind, wrote};
use crate::wire::Post;
use crate::{Actor, ActorId, Message, World};

/// Buffers in a client's set. Unit: buffers.
///
/// A compile-time constant because [`BufferSet::carve`] is const-generic: the
/// count is part of the set's geometry and a set whose geometry could change is
/// a set the compiler cannot check the carve against. Eight, so that a window
/// of eight has a buffer each and a scenario can still exhaust the ring under
/// it. What a scenario varies instead is the *window* — how many the client
/// keeps outstanding — which is the number that decides how much reordering a
/// device is entitled to.
pub const BUFFERS: usize = 8;

/// The widest buffer a snapshot may claim a client had. Unit: bytes.
///
/// Not a limit on the model — [`App::new`] takes whatever a scenario asks for,
/// and a scenario is compiled-in source rather than a file. This is a limit on
/// what [`App::load`] will believe, because that number arrives from a file
/// nothing in this process wrote and is immediately multiplied by [`BUFFERS`]
/// and handed to an allocator. A `u32` that is one bit different from `512`
/// asks for four gigabytes, and an allocation nobody survives is not a refusal:
/// it is the process dying with a message about `alloc`, which is exactly the
/// *misread rather than refused* R04 forbids.
///
/// Sixty-four kilobytes, which is a hundred and twenty-eight times the widest
/// buffer any shipped scenario asks for and half a megabyte once carved.
/// [`tests::no_scenario_asks_for_a_buffer_a_snapshot_would_refuse`] is what
/// keeps the two facts in agreement, so the day a scenario wants a wider buffer
/// than this the *test* says so rather than a restore refusing a file that was
/// perfectly good.
///
/// *Reversal:* raise it, with that test as the reason. It exists to separate
/// *large* from *impossible*, not to police a scenario.
pub const MAX_BUFFER_BYTES: u32 = 64 * 1024;

/// The sequence number the registration's token carries.
///
/// The top of the range, so that no operation's token can collide with it: an
/// operation's completion and the registration's completion arrive on one wire
/// and are told apart by this number alone.
const REGISTRATION: u32 = u32::MAX;

/// A client that registers a buffer set, submits operations against it, and
/// takes every buffer back before it stops.
pub struct App {
    who: u32,
    peer: ActorId,
    window: u32,
    operations: u32,
    retry_ns: u64,
    /// Bytes in each buffer of the set. Unit: bytes.
    buffer_bytes: u32,
    /// How many submissions the wire holds before it refuses. Unit: entries.
    depth: u32,

    idle: Vec<Idle<'static, Fixed>>,
    flight: Vec<InFlight<'static, Fixed>>,
    naming: Option<Fixed>,
    /// Tokens the peer refused for a reason worth waiting out, in the order
    /// they were refused.
    ///
    /// A queue and not a rolled-back counter: with a window above one, the
    /// refused operation is not necessarily the most recent, so decrementing
    /// the sequence would mint a token that is already in flight — and two
    /// buffers on one token means the first completion returns whichever is
    /// asked first, which may be the one the device is still writing.
    /// `Idle::submit` states that obligation as the caller's; this is the
    /// caller meeting it.
    pending: VecDeque<u64>,
    issued: u32,
    completed: u32,
    ended: bool,
    /// Whether one operation in [`URGENT_EVERY`] carries the hard class and a
    /// deadline.
    ///
    /// A property of the client and not of a scenario's whole population, and
    /// the reason is this model's own shape: a virtqueue has exactly one
    /// driver, so `install_peers` gives every client a device of its own and
    /// two clients never share a queue. The contention an ordering needs has to
    /// come from *one* client's own mix — which is also the honest case, because
    /// a component with a hard-class client and a batch one behind the same
    /// storage ring is what RFC 0025's inversion is made of.
    urgent: bool,
}

/// How far ahead of arrival an urgent operation's deadline sits.
/// Unit: nanoseconds on the model's clock.
///
/// Ten microseconds, which is above `dev::FLOOR_NS` by a factor of five — so an
/// urgent operation's deadline is not floored and the scenario measures the
/// *order* rather than RFC 0025's third bound. A scenario that wanted to
/// measure the floor would ask for a deadline inside it, which is a scenario
/// away rather than a code change.
const URGENT_SLACK_NS: u64 = 10_000;

/// One operation in this many carries the hard class, on a client that carries
/// any. Unit: operations.
///
/// Four, and the urgent one is the *last* of each group rather than the first.
/// That is the shape `cargo xtask deadline` submits and it is the shape the
/// failure has: batch work is already queued and the urgent request arrives
/// behind it, so arrival order and deadline order disagree. A client where
/// every operation were urgent would be a client with no batch work to overtake,
/// which is a scenario that cannot fail.
const URGENT_EVERY: u32 = 4;

impl App {
    /// What this actor is called in the trace.
    pub const NAME: &'static str = "app";

    /// A client that will issue `operations` operations to `peer`, keeping at
    /// most `window` outstanding, over a set of [`BUFFERS`] buffers of
    /// `buffer_bytes` each.
    ///
    /// `who` distinguishes this client's tokens from every other client's, and
    /// is the client's own number rather than its [`ActorId`]: a token is the
    /// client's to mint, and a scenario that installed its actors in a different
    /// order should not change what the tokens say.
    #[must_use]
    pub fn new(
        who: u32,
        peer: ActorId,
        window: u32,
        operations: u32,
        buffer_bytes: u32,
        retry_ns: u64,
        depth: u32,
    ) -> Self {
        Self {
            who,
            peer,
            // A window past the set is a window the client could never fill,
            // because each outstanding operation holds a buffer. Clamped rather
            // than refused: a scenario asking for more is asking for *as much as
            // possible*, and the set size is what that means.
            window: window.clamp(1, BUFFERS as u32),
            operations,
            retry_ns,
            buffer_bytes: buffer_bytes.max(1),
            depth: depth.max(1),
            idle: Vec::new(),
            flight: Vec::new(),
            naming: None,
            pending: VecDeque::new(),
            issued: 0,
            completed: 0,
            ended: false,
            urgent: false,
        }
    }

    /// The same client, with one operation in [`URGENT_EVERY`] carrying the
    /// hard class and a deadline.
    ///
    /// A method rather than a seventh argument to [`App::new`], because six
    /// call sites take that constructor positionally and a seventh `bool`
    /// among them is a call nobody can read.
    #[must_use]
    pub const fn urgent(mut self, yes: bool) -> Self {
        self.urgent = yes;
        self
    }

    /// Write this client out, tag first.
    ///
    /// # The awkward half, and why it is the honest one
    ///
    /// Everything above `naming` is a number. Below it are `f_ring::buffers`
    /// types, and those cannot be copied: an [`Idle`] borrows the set it was
    /// carved from, an [`InFlight`] has a drop bomb, and neither has a
    /// constructor a caller can reach. That is RFC 0024 working — *writing an id
    /// down should not be the shortest path to a naming* — and a snapshot is not
    /// an exception to it.
    ///
    /// So what travels is the **name** of each buffer and the token it is out
    /// under, and [`App::load`] puts them back by doing what this client did:
    /// binding the set, carving it, and submitting. Nothing here fabricates an
    /// ownership type, and a restored client's buffers are as real as the ones
    /// it had.
    ///
    /// The buffers' *bytes* are not written and do not need to be. The only byte
    /// this client cares about is the pattern it stamps before lending, it
    /// stamps it again on every submission, and [`App::load`] stamps it for
    /// every buffer that is out — so the one thing `reap` reads back is exactly
    /// what it would have read. A model that wrote into a client's data buffer
    /// would change that, and `check::intact` is the property that would notice;
    /// RFC 0043 records the dependency rather than leaving it implicit.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) {
        out.u32(crate::snap::tag::APP);
        out.u32(self.who);
        out.u32(self.peer.0);
        out.u32(self.window);
        out.u32(self.operations);
        out.u64(self.retry_ns);
        out.u32(self.buffer_bytes);
        out.u32(self.depth);
        out.bool(self.urgent);
        out.bool(self.naming.is_some());
        out.u32(self.naming.map_or(0, |naming| naming.set().bits()));
        out.count(self.idle.len());
        for buffer in &self.idle {
            out.u32(buffer.index());
        }
        out.count(self.flight.len());
        for lent in &self.flight {
            out.u32(lent.index());
            out.u64(lent.token());
        }
        out.count(self.pending.len());
        for token in &self.pending {
            out.u64(*token);
        }
        out.u32(self.issued);
        out.u32(self.completed);
        out.bool(self.ended);
    }

    /// Read one back, rebuilding its buffer set the way it was built.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        let who = input.u32();
        let peer = ActorId(input.u32());
        let window = input.u32();
        // `App::new` clamps a window to the set, so a saved one is always
        // between one and `BUFFERS` and a file outside that range is a file this
        // crate did not write. Refused for the reason the width below is: a
        // clamp applied on the way in restores a client that keeps a different
        // number of operations outstanding from the one the file described, and
        // every seeded decision after the cut would then be taken over a
        // different set.
        if window == 0 || window > BUFFERS as u32 {
            input.refuse(crate::snap::Broken::Bounds("a window no client could have kept"));
        }
        let operations = input.u32();
        let retry_ns = input.u64();
        let buffer_bytes = input.u32();
        // Bounded here rather than where it is allocated, because this is the
        // first moment the value exists and R04 refuses at the boundary the
        // value crosses. Zero is refused as well as too-large: `App::new`
        // clamps a zero to one for a scenario that means *the smallest useful
        // buffer*, and a file is not a scenario — a file saying zero is a file
        // describing a client this crate cannot have produced, and repairing it
        // would restore a run nobody had. See [`MAX_BUFFER_BYTES`].
        if buffer_bytes == 0 || buffer_bytes > MAX_BUFFER_BYTES {
            input.refuse(crate::snap::Broken::Bounds("a buffer size no scenario can ask for"));
        }
        let depth = input.u32();
        if depth == 0 {
            // The same asymmetry as `buffer_bytes` above, for the same reason.
            input.refuse(crate::snap::Broken::Bounds("a client whose wire holds no submission"));
        }
        let urgent = input.bool();
        let bound = input.bool();
        let bits = input.u32();
        let idle_count = input.count(4, "more idle buffers than the file could hold");
        let mut idle_at = Vec::with_capacity(idle_count);
        for _ in 0..idle_count {
            idle_at.push(input.u32());
        }
        let flight_count = input.count(12, "more buffers in flight than the file could hold");
        let mut flight_at = Vec::with_capacity(flight_count);
        for _ in 0..flight_count {
            flight_at.push((input.u32(), input.u64()));
        }
        let waiting = input.count(8, "more refused tokens than the file could hold");
        let mut pending = VecDeque::with_capacity(waiting);
        for _ in 0..waiting {
            pending.push_back(input.u64());
        }

        let mut app =
            Self::new(who, peer, window, operations, buffer_bytes, retry_ns, depth).urgent(urgent);
        app.pending = pending;
        app.issued = input.u32();
        app.completed = input.u32();
        app.ended = input.bool();
        if !bound || input.faulted() {
            // A client that never bound a set holds no buffers, which is the
            // state it is in between `register` and the completion that answers
            // it. Nothing to rebuild.
            return app;
        }

        // A completion carrying the id and nothing else, which is the only door
        // into `Fixed`. It is the same door the live path uses, and the id came
        // out of a table that issued it — a restore is not a peer, and this is
        // not a way to name a set nobody registered.
        let cqe = Cqe { ext: u64::from(bits), ..Cqe::ZERO };
        let Ok(naming) = Fixed::from_completion(&cqe) else {
            input.refuse(crate::snap::Broken::Bounds("a buffer set id nothing could have issued"));
            return app;
        };
        app.naming = Some(naming);

        // `checked_mul` rather than `*`, and a refusal rather than a clamp: the
        // bound above already rules this out on every path that reaches here, so
        // this is the second door on the same room. It costs one branch and it
        // is the difference between a bound that is *stated* and a bound that is
        // *enforced* — a future edit that moves the check above should fail to
        // build a run, not silently start trusting the file again.
        let Some(len) = (app.buffer_bytes as usize).checked_mul(BUFFERS) else {
            input.refuse(crate::snap::Broken::Bounds("a buffer set larger than memory can hold"));
            return app;
        };
        let region: &'static mut [u8] = Box::leak(vec![0u8; len].into_boxed_slice());
        let agreed = Negotiated { version: ABI_VERSION, features: 0 };
        let Ok(set) = BufferSet::bind(naming, agreed, region) else {
            input.refuse(crate::snap::Broken::Diverged("a buffer set that would not bind again"));
            return app;
        };
        let set: &'static mut BufferSet<'static, Fixed> = Box::leak(Box::new(set));
        let Ok(buffers) = set.carve::<BUFFERS>() else {
            input.refuse(crate::snap::Broken::Diverged("a region that would not carve again"));
            return app;
        };
        // `carve` answers buffer `i` at position `i`, so a slot here is named by
        // the same index the snapshot recorded.
        let mut slots: Vec<Option<Idle<'static, Fixed>>> = buffers.into_iter().map(Some).collect();

        for (index, token) in flight_at {
            let Some(mut buffer) = slots.get_mut(index as usize).and_then(Option::take) else {
                input.refuse(crate::snap::Broken::Bounds("a buffer index outside the set"));
                return app;
            };
            // The pattern this client stamps before lending, stamped again — see
            // `App::save` for why the rest of the bytes do not travel.
            if let Some(first) = buffer.bytes_mut().first_mut() {
                *first = pattern(token);
            }
            let entry = Sqe {
                user_data: token,
                len: app.buffer_bytes,
                offset: u64::from(Self::nth(token)),
                ..Sqe::ZERO
            };
            match buffer.submit(&mut Discard, entry) {
                Ok((lent, _rang)) => app.flight.push(lent),
                Err((_refused, back)) => {
                    // Cannot happen — `Discard` never refuses and the length is
                    // the buffer's own — and handled rather than unwrapped,
                    // because the buffer coming back has a drop bomb on it and
                    // an `expect` here would turn a corrupt file into a panic.
                    slots[index as usize] = Some(back);
                    input.refuse(crate::snap::Broken::Diverged("a buffer that would not lend"));
                    return app;
                }
            }
        }
        for index in idle_at {
            let Some(buffer) = slots.get_mut(index as usize).and_then(Option::take) else {
                input.refuse(crate::snap::Broken::Bounds("a buffer index outside the set"));
                return app;
            };
            app.idle.push(buffer);
        }
        app
    }

    /// The token for this client's `nth` operation.
    ///
    /// The client's number in the high half and the sequence in the low: unique
    /// across the run, and legible in a trace without a lookup.
    const fn token(&self, nth: u32) -> u64 {
        ((self.who as u64) << 32) | nth as u64
    }

    /// The sequence [`App::token`] minted a token from.
    ///
    /// The exact inverse of the low half, and it exists so that an operation's
    /// position is a property of the *operation* rather than of when it happened
    /// to be submitted. A retry re-submits the same position, which is what a
    /// driver does; deriving the position from a counter meant a retry moved.
    /// Does this client's operation `token` carry the hard class?
    ///
    /// Derived from the token rather than counted, so that a *retried* token
    /// carries the same class it carried the first time. A counter would have
    /// made a request change class between attempts, which is the same defect
    /// `Sqe::offset` already records having had here.
    #[must_use]
    pub const fn is_urgent(urgent: bool, token: u64) -> bool {
        urgent && Self::nth(token) % URGENT_EVERY == URGENT_EVERY - 1
    }

    const fn nth(token: u64) -> u32 {
        (token & 0xFFFF_FFFF) as u32
    }

    /// How many operations completed. Unit: operations.
    #[must_use]
    pub const fn completed(&self) -> u32 {
        self.completed
    }

    /// Ask the peer for a buffer set.
    ///
    /// Goes on the wire directly rather than through [`Idle::submit`], because
    /// there is no buffer to name yet — which is RFC 0028's whole shape:
    /// registration is an *operation*, on the same ring, answered by a
    /// completion carrying the id every later entry names.
    fn ask_for_buffers(&mut self, world: &mut World, me: ActorId) {
        let token = self.token(REGISTRATION);
        let len = self.buffer_bytes.saturating_mul(BUFFERS as u32);
        // `cap` zero: the capability index the component would name for the
        // memory it is registering. Every client here holds one capability and
        // it is slot zero — `Sqe::cap`'s own documentation says zero is a valid
        // slot and not a null.
        let entry = registration(token, 0, len, BUFFERS as u32);
        world.wire().post(me, self.peer, entry);
        world.record(me, Self::NAME, wrote::REGISTER, token, u64::from(len));
        world.send(0, self.peer, Message { from: me, kind: kind::SUBMIT, token, detail: 0 });
    }

    /// Bind the set the peer issued and carve it, or stop.
    ///
    /// The naming comes from the peer's own completion and from nowhere else —
    /// `Fixed::from_completion` is the only constructor — which is what
    /// `E1-B10` added to that type so that writing an id down is no longer the
    /// shortest path to a buffer name.
    fn bind(&mut self, world: &mut World, me: ActorId, cqe: &Cqe) {
        let Ok(naming) = Fixed::from_completion(cqe) else {
            // The registration was refused. Nothing was carved, nothing is
            // outstanding, and there is no work this client can do — so it
            // finishes here rather than submitting entries naming a set it does
            // not hold, which the peer would refuse one at a time.
            world.record(me, Self::NAME, wrote::REFUSED, cqe.user_data, cqe.ext);
            self.finish(world, me);
            return;
        };
        self.naming = Some(naming);

        let region: &'static mut [u8] =
            Box::leak(vec![0u8; self.buffer_bytes as usize * BUFFERS].into_boxed_slice());
        // Zero features: this client binds `Fixed`, whose `REQUIRES` is zero, so
        // the channel needs nothing negotiated. A client wanting the
        // shared-virtual-memory path would bind `Virtual` here and the service
        // would refuse it on a channel that did not agree the bit — one line and
        // one refusal, which is what `E1-B10` measures.
        let agreed = Negotiated { version: ABI_VERSION, features: 0 };
        let Ok(set) = BufferSet::bind(naming, agreed, region) else {
            world.record(me, Self::NAME, wrote::REFUSED, cqe.user_data, u64::MAX);
            self.finish(world, me);
            return;
        };
        let set: &'static mut BufferSet<'static, Fixed> = Box::leak(Box::new(set));
        let Ok(buffers) = set.carve::<BUFFERS>() else {
            world.record(me, Self::NAME, wrote::REFUSED, cqe.user_data, u64::MAX);
            self.finish(world, me);
            return;
        };
        self.idle.extend(buffers);

        world.record(me, Self::NAME, wrote::BOUND, cqe.user_data, u64::from(naming.set().bits()));
        self.pump(world, me);
    }

    /// Issue as many operations as the window and the set allow.
    ///
    /// Refused work first, then new work. A client that minted new tokens while
    /// old ones waited would grow its backlog under exactly the pressure the
    /// back-off exists to relieve.
    fn pump(&mut self, world: &mut World, me: ActorId) {
        while u32::try_from(self.flight.len()).unwrap_or(u32::MAX) < self.window {
            let token = if let Some(again) = self.pending.pop_front() {
                again
            } else if self.issued < self.operations {
                let fresh = self.token(self.issued);
                self.issued = self.issued.saturating_add(1);
                fresh
            } else {
                break;
            };
            let Some(mut buffer) = self.idle.pop() else {
                // Every buffer is out. Not a refusal and not an error: the
                // window and the set are the same bound seen twice, and the
                // completion that frees one is what resumes this.
                self.pending.push_front(token);
                break;
            };

            // Written *before* the submission and read after the completion:
            // the one place this client touches its own bytes, and the reason
            // the ownership types are worth driving. There is no second place,
            // because `InFlight` has no method that reaches them.
            if let Some(first) = buffer.bytes_mut().first_mut() {
                *first = pattern(token);
            }

            let entry = Sqe {
                user_data: token,
                len: self.buffer_bytes,
                // The operation's own sequence, which every peer reads as the
                // position it works at: a sector for a disk, a frame for a
                // link, a request for a display. Unit: per-peer, and the peer
                // that reads it is what states what it means — the same rule
                // `Sqe::offset` itself carries.
                //
                // Carried on the token rather than read off the issue counter,
                // which is what this line used to do and which was neither
                // quantity: `self.issued` has already been incremented for
                // fresh work, so the first request landed at sector one and
                // sector zero was never reachable, and a *retried* token was
                // re-submitted at whatever the counter had reached by then —
                // a request that moved between attempts, which no driver does
                // and no device should have to tolerate. `Blk::describe` picks
                // its direction from this number, so a retry could even flip
                // from a read to a write.
                offset: u64::from(Self::nth(token)),
                // RFC 0025's two fields, written by the component that
                // originates the request: the class ordinal at depth zero,
                // because nothing forwarded this, and a deadline relative to
                // *now* on the model's own clock. A batch client writes
                // neither, which is what `Sqe::ZERO` already says.
                class: if Self::is_urgent(self.urgent, token) {
                    f_abi::deadline::pack(f_abi::class::HARD, 0)
                } else {
                    Sqe::ZERO.class
                },
                deadline: if Self::is_urgent(self.urgent, token) {
                    world.clock().saturating_add(URGENT_SLACK_NS)
                } else {
                    0
                },
                ..Sqe::ZERO
            };

            let mut post = Post::new(world, me, self.peer, self.depth);
            match buffer.submit(&mut post, entry) {
                Ok((lent, _rang)) => {
                    self.flight.push(lent);
                    world.record(
                        me,
                        Self::NAME,
                        wrote::ISSUE,
                        token,
                        u64::try_from(self.flight.len()).unwrap_or(u64::MAX),
                    );
                    // `E1-P02`'s torn doorbell. Publishing the entry and
                    // ringing the bell are two stores, and a torn pair is a bell
                    // with nothing behind it — so the bell rings twice for one
                    // entry and the peer finds an empty wire on one of them.
                    //
                    // The other tear, an entry with no bell, is deliberately not
                    // produced here: this model's peer takes one entry per
                    // doorbell, so a missing bell would be a lost entry rather
                    // than a late one, and what that would exercise is the
                    // model's shape rather than the system's response. A peer
                    // that lies about its cursors is `E1-P04`'s.
                    if world.strike(me, Class::Doorbell, token).is_some() {
                        world.send(
                            0,
                            self.peer,
                            Message { from: me, kind: kind::SUBMIT, token, detail: 0 },
                        );
                    }
                    world.send(
                        0,
                        self.peer,
                        Message { from: me, kind: kind::SUBMIT, token, detail: 0 },
                    );
                }
                Err((Refused::Ring(RingError::Full), back)) => {
                    // The ring had no room, so the buffer came straight back and
                    // nothing was issued. A full ring is a retry, not a loss, so
                    // the token goes back on the queue rather than being spent.
                    self.idle.push(back);
                    self.pending.push_front(token);
                    world.record(me, Self::NAME, wrote::FULL, token, u64::from(self.depth));
                    world.send(
                        self.retry_ns,
                        me,
                        Message { from: me, kind: kind::RETRY, token, detail: 0 },
                    );
                    return;
                }
                Err((refused, back)) => {
                    // Anything else is this client having built an entry the
                    // ownership types refuse — a length past the buffer, most
                    // likely. Not a peer's doing and not a retry: recorded, and
                    // the client stops rather than repeating it forever.
                    self.idle.push(back);
                    world.record(me, Self::NAME, wrote::REFUSED, token, refused_as(&refused));
                    self.ended = true;
                    return;
                }
            }
        }

        if self.issued == self.operations && self.flight.is_empty() && self.pending.is_empty() {
            self.finish(world, me);
        }
    }

    /// Match a completion against the buffers this client has out.
    ///
    /// Asking each in-flight buffer *is this yours?* is what
    /// [`InFlight::complete`] is built for, and its own documentation says so:
    /// a client reaping a completion ring sees every token, and the token is the
    /// whole of the test.
    fn reap(&mut self, world: &mut World, me: ActorId, cqe: &Cqe) {
        let mut rest = Vec::with_capacity(self.flight.len());
        let mut returned = None;
        for lent in self.flight.drain(..) {
            if returned.is_some() {
                rest.push(lent);
                continue;
            }
            match lent.complete(cqe) {
                Ok(idle) => returned = Some(idle),
                Err(still) => rest.push(still),
            }
        }
        self.flight = rest;

        let Some(buffer) = returned else {
            // R04: a completion for a token this client does not hold is
            // recorded rather than ignored. It changes the digest, which fails
            // the comparison this whole crate exists to make — and a peer that
            // answered a token nobody lent is a peer with a bug worth seeing.
            world.record(me, Self::NAME, wrote::REFUSED, cqe.user_data, u64::MAX);
            return;
        };

        // The bytes are reachable again, and they are the ones this client
        // wrote: the buffer came back from the same place it went. Read rather
        // than assumed, because *the buffer was returned* and *the buffer was
        // returned intact* are different claims and only one of them is what a
        // ownership rule is worth.
        let intact = buffer.bytes().first().copied() == Some(pattern(cqe.user_data));
        self.idle.push(buffer);

        if let Some((domain, code)) = cqe.error() {
            world.record(me, Self::NAME, wrote::REFUSED, cqe.user_data, packed(domain, code));
            if domain == error::RESOURCE {
                // The peer is busy rather than broken, so this is back-pressure
                // and the answer is to wait and submit again — with this token,
                // which names work that has not happened yet.
                self.pending.push_back(cqe.user_data);
                world.send(
                    self.retry_ns,
                    me,
                    Message { from: me, kind: kind::RETRY, token: cqe.user_data, detail: 0 },
                );
                return;
            }
            // A refusal the client will not retry. Counted as finished, because
            // the operation is over: `completed` is *operations this client will
            // not submit again*, not *operations that succeeded*, and a trace
            // reader has the refusal record beside it to tell the two apart.
            self.completed = self.completed.saturating_add(1);
        } else {
            self.completed = self.completed.saturating_add(1);
            world.record(
                me,
                Self::NAME,
                wrote::DONE,
                cqe.user_data,
                if intact { cqe.result as u32 as u64 } else { u64::MAX },
            );
        }
        self.pump(world, me);
    }

    /// The peer restarted: take every buffer back, because no completion is
    /// coming for any of them.
    ///
    /// The evidence is [`PeerGone`], and what makes it sound is what RFC 0008
    /// makes the frame do when a component ends — revoke its buffer sets and
    /// tear down its IOMMU domain, so a transfer the dead peer had started
    /// faults rather than landing in memory this side is about to reuse.
    /// [`crate::service::Service::retire_all`] is the model of that half, and
    /// the device models call it before they send this.
    fn reclaim(&mut self, world: &mut World, me: ActorId) {
        let Some(gone) = PeerGone::of(RingError::EpochChanged) else {
            return;
        };
        for lent in self.flight.drain(..) {
            let token = lent.token();
            self.idle.push(lent.reclaim(gone));
            world.record(me, Self::NAME, wrote::RECLAIM, token, 0);
        }
        self.finish(world, me);
    }

    /// Say what this client managed, once.
    fn finish(&mut self, world: &mut World, me: ActorId) {
        if self.ended {
            return;
        }
        self.ended = true;
        world.record(
            me,
            Self::NAME,
            wrote::FINISHED,
            u64::from(self.who),
            u64::from(self.completed),
        );
    }
}

impl Actor for App {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::START => self.ask_for_buffers(world, me),
            kind::CQE => {
                let Some(cqe) = world.wire().reap(self.peer, me) else {
                    // A doorbell with nothing behind it. Recorded rather than
                    // ignored: on the real ring a spurious wake-up is ordinary
                    // and costs a poll, and a model that treated it as an error
                    // would report a bug where the system has a design.
                    world.record(me, Self::NAME, message.kind, message.token, u64::MAX);
                    return;
                };
                if cqe.user_data == self.token(REGISTRATION) {
                    self.bind(world, me, &cqe);
                } else {
                    self.reap(world, me, &cqe);
                }
            }
            kind::RETRY => self.pump(world, me),
            kind::GONE => self.reclaim(world, me),
            other => world.record(me, Self::NAME, other, message.token, u64::MAX),
        }
    }

    fn save(&self, out: &mut crate::snap::Writer) -> Result<(), crate::snap::Broken> {
        Self::save(self, out);
        Ok(())
    }
}

/// A [`Submitter`] that takes the entry and does nothing with it.
///
/// The one thing [`App::load`] needs that the live path does not: an
/// [`InFlight`] is made only by [`Idle::submit`], and a restore has to make one
/// **without** putting a second copy of the entry on a wire that already carries
/// it. `f_ring::buffers`'s own documentation names this shape — *a test can put
/// a recorder behind it and exercise the ownership rules with no ring at all* —
/// and this is that recorder with nothing to record.
struct Discard;

impl Submitter for Discard {
    fn submit(&mut self, _entry: Sqe) -> Result<bool, RingError> {
        Ok(true)
    }
}

impl Drop for App {
    /// The component ends, and every buffer it still holds ends with it.
    ///
    /// `InFlight`'s drop is a bomb, deliberately: a *live* component that
    /// abandons a buffer the device is writing into is the bug RFC 0024 exists
    /// to make unwritable, and ending the component is what makes that write
    /// fault instead of land. This is the other case — the component is the
    /// thing ending — and RFC 0008 says the frame revokes its buffer sets and
    /// tears down its IOMMU domain when it does, which is exactly the condition
    /// `PeerGone` attests to. So the buffers are reclaimed rather than dropped,
    /// and a run that ends with work outstanding reports [`crate::Trouble`]
    /// rather than panicking with a message about a bug nobody wrote.
    ///
    /// The awkwardness is real and worth writing down: `PeerGone::of` takes a
    /// `RingError`, so this passes `EpochChanged` to state a fact that is not a
    /// ring error. The type wants a second constructor — *this component is
    /// ending* — and whoever owns `ring/src/buffers.rs` is who would add it.
    fn drop(&mut self) {
        let Some(gone) = PeerGone::of(RingError::EpochChanged) else {
            return;
        };
        for lent in self.flight.drain(..) {
            let _ = lent.reclaim(gone);
        }
    }
}

/// The byte a client stamps into a buffer before lending it.
///
/// A function of the token, so that a buffer coming back under the wrong token
/// is visible rather than plausible. Never zero — a zeroed region is what the
/// frame hands a component, so a pattern of zero would be indistinguishable
/// from memory nothing has touched.
const fn pattern(token: u64) -> u8 {
    ((token & 0x7F) as u8) | 0x80
}

/// Pack a domain and a code the way a completion carries them, for the trace.
const fn packed(domain: u8, code: u16) -> u64 {
    ((domain as u64) << 16) | code as u64
}

/// A refusal from the ownership types, as one number for the trace.
fn refused_as(refused: &Refused) -> u64 {
    match refused {
        Refused::Ring(RingError::Full) => 1,
        Refused::Ring(RingError::Corrupt) => 2,
        Refused::Ring(RingError::EpochChanged) => 3,
        Refused::Misuse(_) => 4,
    }
}
