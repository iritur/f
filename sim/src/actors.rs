// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The two actors stage one ships, and the shape every device model takes.
//!
//! # What this is a model of
//!
//! A client submitting operations to a service across a bounded queue, which is
//! the shape of every ring in this system and the shape of every virtio device
//! behind one. The three things that make it a model rather than a loop are the
//! three things a device really does and a naive harness never does:
//!
//! - **It can refuse.** The queue has a depth, and a submission into a full one
//!   is answered `full` rather than blocked. `f_ring::RingError::Full` is that
//!   answer on the real ring, and a client that has never seen it is a client
//!   whose back-pressure path has never run.
//! - **It chooses what to serve next.** The service takes whichever queued
//!   operation the seed picks, which is what a device with several outstanding
//!   descriptors is entitled to do: nothing in virtio says the used ring is in
//!   available-ring order. A harness that completed in submission order would
//!   never find the bug where a client assumes it is.
//! - **It takes a time the seed picks.** Service time is a base plus a spread,
//!   drawn from the run's randomness stream, so two operations submitted
//!   together do not finish together.
//!
//! What it is *not* is a virtqueue. There are no descriptors, no available ring
//! and no used ring here, and no ABI entry crosses between these two actors.
//! Those live in [`crate::virtq`], [`crate::dev`] and the three device models
//! beside them, with [`crate::client::App`] above — and that client, not this
//! one, is what the component-substitution property is a claim about. What this
//! file establishes, and still establishes, is that the machinery underneath
//! them all — the timeline, the per-channel order, the seeded service order —
//! carries a real exchange and produces an artefact. Its three scenarios are
//! that claim's evidence, and their digests are unchanged by everything built on
//! top, which is the point of keeping it.
//!
//! # Why the client is here too
//!
//! Because component substitution is a claim about the *client*: the same client
//! code runs against a modelled device or a real component, chosen at
//! construction. A file with only device models could not demonstrate that, and
//! a client written inside each device model would not be one client. This one
//! addresses its service by [`ActorId`] and knows nothing else about it, which
//! is the whole of what substitution needs.

use crate::{Actor, ActorId, Message, World};

/// The site the service records its choice of what to serve next at.
pub const NEXT: &str = "service.next";

/// Message kinds. Each is at most [`crate::LABEL_WIDTH`] bytes, and
/// [`tests::every_label_fits_the_trace_column`] is what says so.
pub mod kind {
    /// Begin. Sent to each client at the start of the run.
    pub const START: &str = "start";
    /// Client to service: please do this operation.
    pub const SUBMIT: &str = "submit";
    /// Service to client: the queue was full.
    pub const REFUSED: &str = "refused";
    /// Service to itself: this operation's service time has elapsed.
    pub const FINISH: &str = "finish";
    /// Service to client: done.
    pub const COMPLETE: &str = "complete";
    /// Client to itself: the back-off after a refusal has elapsed.
    pub const RETRY: &str = "retry";
}

/// What an actor writes into the trace.
///
/// Public because `snap::LABELS` has to name every label a run can put in a
/// record, and a label reachable only from inside this file is a label a
/// snapshot cannot write. See that table for why one list rather than several.
pub mod wrote {
    /// A client put an operation on the wire.
    pub const ISSUE: &str = "issue";
    /// A client was refused and will try again.
    pub const REFUSED: &str = "refused";
    /// A client saw one of its operations complete.
    pub const DONE: &str = "done";
    /// A client has no operations left to issue and none outstanding.
    pub const FINISHED: &str = "finished";
    /// The service accepted an operation into its queue.
    pub const QUEUE: &str = "queue";
    /// The service refused one, because the queue was full.
    pub const FULL: &str = "full";
    /// The service began serving one.
    pub const START: &str = "start";
    /// The service finished serving one.
    pub const SERVED: &str = "served";
}

/// A client with a fixed amount of work and a fixed number of operations it
/// will keep outstanding.
#[derive(Clone, Debug)]
pub struct Client {
    who: u32,
    service: ActorId,
    window: u32,
    operations: u32,
    retry_ns: u64,
    issued: u32,
    outstanding: u32,
    completed: u32,
}

impl Client {
    /// What this actor is called in the trace.
    pub const NAME: &'static str = "client";

    /// A client that will issue `operations` operations to `service`, keeping at
    /// most `window` of them outstanding, and waiting `retry_ns` nanoseconds
    /// after a refusal before trying again.
    ///
    /// `who` distinguishes this client's tokens from every other client's. It is
    /// the client's own number and not its [`ActorId`], because a token is the
    /// client's to mint and a scenario that installed its actors in a different
    /// order should not change what the tokens say.
    #[must_use]
    pub fn new(who: u32, service: ActorId, window: u32, operations: u32, retry_ns: u64) -> Self {
        Self {
            who,
            service,
            window: window.max(1),
            operations,
            retry_ns,
            issued: 0,
            outstanding: 0,
            completed: 0,
        }
    }

    /// The token for this client's `nth` operation.
    ///
    /// The client's number in the high half and the operation's index in the
    /// low: unique across the run, and legible in a trace without a lookup.
    #[must_use]
    fn token(&self, nth: u32) -> u64 {
        (u64::from(self.who) << 32) | u64::from(nth)
    }

    /// Issue as many operations as the window allows.
    fn pump(&mut self, world: &mut World, me: ActorId) {
        while self.outstanding < self.window && self.issued < self.operations {
            let token = self.token(self.issued);
            self.issued = self.issued.saturating_add(1);
            self.outstanding = self.outstanding.saturating_add(1);
            self.issue(world, me, token);
        }
        if self.issued == self.operations && self.outstanding == 0 {
            let completed = u64::from(self.completed);
            world.record(me, Self::NAME, wrote::FINISHED, u64::from(self.who), completed);
        }
    }

    /// Write this client out, tag first. Eight numbers and no ownership types,
    /// which is the whole difference between stage one's client and
    /// [`crate::client::App`].
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) {
        out.u32(crate::snap::tag::CLIENT);
        out.u32(self.who);
        out.u32(self.service.0);
        out.u32(self.window);
        out.u32(self.operations);
        out.u64(self.retry_ns);
        out.u32(self.issued);
        out.u32(self.outstanding);
        out.u32(self.completed);
    }

    /// Read one back.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        let who = input.u32();
        let service = ActorId(input.u32());
        let window = input.u32();
        let operations = input.u32();
        let retry_ns = input.u64();
        let mut client = Self::new(who, service, window, operations, retry_ns);
        client.issued = input.u32();
        client.outstanding = input.u32();
        client.completed = input.u32();
        client
    }

    /// Put one operation on the wire. Used by [`Client::pump`] for a new
    /// operation and by the retry path for one the service had no room for.
    fn issue(&mut self, world: &mut World, me: ActorId, token: u64) {
        world.record(me, Self::NAME, wrote::ISSUE, token, u64::from(self.outstanding));
        // Zero delay: a submission is visible to the service at the instant it
        // is made, and what happens when two of them are is the timeline's to
        // decide.
        world.send(0, self.service, Message { from: me, kind: kind::SUBMIT, token, detail: 0 });
    }
}

impl Actor for Client {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::START => self.pump(world, me),
            kind::COMPLETE => {
                self.outstanding = self.outstanding.saturating_sub(1);
                self.completed = self.completed.saturating_add(1);
                world.record(me, Self::NAME, wrote::DONE, message.token, message.detail);
                self.pump(world, me);
            }
            kind::REFUSED => {
                // The operation is still outstanding — it simply never reached
                // the queue — so nothing about the window changes and the same
                // token goes back on the wire after the back-off. Rolling the
                // issue counter back instead would let the client mint a token
                // that is already in flight, which is how a simulator produces a
                // trace that reproduces perfectly and describes work nobody did.
                world.record(me, Self::NAME, wrote::REFUSED, message.token, self.retry_ns);
                world.send(
                    self.retry_ns,
                    me,
                    Message { from: me, kind: kind::RETRY, token: message.token, detail: 0 },
                );
            }
            kind::RETRY => self.issue(world, me, message.token),
            // R04: an unknown kind is refused rather than ignored. There is no
            // peer here to answer, so the refusal is a record — and a record is
            // enough, because it changes the digest and therefore fails the
            // comparison the whole crate exists to make.
            other => world.record(me, Self::NAME, other, message.token, u64::MAX),
        }
    }

    fn save(&self, out: &mut crate::snap::Writer) -> Result<(), crate::snap::Broken> {
        Self::save(self, out);
        Ok(())
    }
}

/// A single-server queue with a depth, a service time and an order the seed
/// chooses.
#[derive(Clone, Debug)]
pub struct Service {
    depth: u32,
    base_ns: u64,
    spread_ns: u64,
    queued: Vec<(ActorId, u64)>,
    busy: bool,
}

impl Service {
    /// What this actor is called in the trace.
    pub const NAME: &'static str = "service";

    /// A service that will hold at most `depth` operations, and take between
    /// `base_ns` and `base_ns + spread_ns` nanoseconds over each one.
    #[must_use]
    pub fn new(depth: u32, base_ns: u64, spread_ns: u64) -> Self {
        Self { depth: depth.max(1), base_ns, spread_ns, queued: Vec::new(), busy: false }
    }

    /// Write this service out, tag first.
    ///
    /// The queue travels in arrival order, which is the order [`Service::begin`]
    /// hands to `World::decide` as the list it indexes into — the same reason
    /// `dev.rs` removes a job with `remove` and not `swap_remove`. A snapshot
    /// that reordered it would restore a service whose next seeded choice picked
    /// a different operation, and the divergence would be silent.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) {
        out.u32(crate::snap::tag::SERVICE);
        out.u32(self.depth);
        out.u64(self.base_ns);
        out.u64(self.spread_ns);
        out.bool(self.busy);
        out.count(self.queued.len());
        for (client, token) in &self.queued {
            out.u32(client.0);
            out.u64(*token);
        }
    }

    /// Read one back.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        // Refused rather than left for `Service::new` to clamp, for the reason
        // `service::Grants::load` states at length: a clamp is a repair, and a
        // repaired file restores into a world that is plausible and is not the
        // one the file described.
        let depth = input.u32();
        if depth == 0 {
            input.refuse(crate::snap::Broken::Bounds("a service that would hold no operation"));
        }
        let base_ns = input.u64();
        let spread_ns = input.u64();
        let mut service = Self::new(depth, base_ns, spread_ns);
        service.busy = input.bool();
        let count = input.count(12, "more queued operations than the file could hold");
        service.queued = Vec::with_capacity(count);
        for _ in 0..count {
            service.queued.push((ActorId(input.u32()), input.u64()));
        }
        service
    }

    /// Take one queued operation and begin it.
    ///
    /// Which one is the seed's, and that is the point: nothing in virtio says a
    /// device completes in the order it was asked, so a model that did would
    /// never find the client that assumes it does.
    fn begin(&mut self, world: &mut World, me: ActorId) {
        let arity = u32::try_from(self.queued.len()).unwrap_or(u32::MAX);
        if arity == 0 {
            return;
        }
        let taken = world.decide(NEXT, arity) as usize;
        // `get` rather than a subscript: `taken` is below `arity` by
        // construction, and a model that would panic if it were not is a model
        // whose failure mode is worse than the bug.
        let Some((client, token)) = self.queued.get(taken).copied() else {
            return;
        };
        self.queued.remove(taken);

        // `spread_ns + 1` so that a spread of zero is a constant service time
        // rather than a division by zero, and so that the draw happens either
        // way: a scenario's spread must not change how far along the randomness
        // stream a run is, or two scenarios would not be comparable.
        let service_ns =
            self.base_ns.saturating_add(world.draw() % self.spread_ns.saturating_add(1));
        self.busy = true;
        world.record(me, Self::NAME, wrote::START, token, service_ns);
        world.send(
            service_ns,
            me,
            Message { from: me, kind: kind::FINISH, token, detail: u64::from(client.0) },
        );
    }
}

impl Actor for Service {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::SUBMIT => {
                if u32::try_from(self.queued.len()).unwrap_or(u32::MAX) >= self.depth {
                    world.record(me, Self::NAME, wrote::FULL, message.token, u64::from(self.depth));
                    world.send(
                        0,
                        message.from,
                        Message {
                            from: me,
                            kind: kind::REFUSED,
                            token: message.token,
                            detail: u64::from(self.depth),
                        },
                    );
                    return;
                }
                self.queued.push((message.from, message.token));
                let depth = u64::try_from(self.queued.len()).unwrap_or(u64::MAX);
                world.record(me, Self::NAME, wrote::QUEUE, message.token, depth);
                if !self.busy {
                    self.begin(world, me);
                }
            }
            kind::FINISH => {
                self.busy = false;
                world.record(me, Self::NAME, wrote::SERVED, message.token, message.detail);
                let client = ActorId(u32::try_from(message.detail).unwrap_or(u32::MAX));
                world.send(
                    0,
                    client,
                    Message {
                        from: me,
                        kind: kind::COMPLETE,
                        token: message.token,
                        detail: world.clock(),
                    },
                );
                self.begin(world, me);
            }
            other => world.record(me, Self::NAME, other, message.token, u64::MAX),
        }
    }

    fn save(&self, out: &mut crate::snap::Writer) -> Result<(), crate::snap::Broken> {
        Self::save(self, out);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LABEL_WIDTH, Simulation};

    /// Every label this crate ships, so one test can check them all against the
    /// trace's column width rather than each being checked where it is written.
    const LABELS: &[&str] = &[
        kind::START,
        kind::SUBMIT,
        kind::REFUSED,
        kind::FINISH,
        kind::COMPLETE,
        kind::RETRY,
        wrote::ISSUE,
        wrote::REFUSED,
        wrote::DONE,
        wrote::FINISHED,
        wrote::QUEUE,
        wrote::FULL,
        wrote::START,
        wrote::SERVED,
        Client::NAME,
        Service::NAME,
    ];

    #[test]
    fn every_label_fits_the_trace_column() {
        // A label wider than the column would shift every field after it, so two
        // otherwise identical runs would disagree because one of them had a
        // longer word in it. Checked once for the whole set rather than at each
        // site, because the set is what stage two grows.
        for label in LABELS {
            assert!(
                label.len() <= LABEL_WIDTH,
                "`{label}` is {} bytes and the column is {LABEL_WIDTH}",
                label.len()
            );
        }
    }

    fn exchange(
        seed: u64,
        clients: u32,
        window: u32,
        depth: u32,
        operations: u32,
    ) -> crate::Outcome {
        let mut sim = Simulation::new(seed, 100_000);
        let service = sim.install(Box::new(Service::new(depth, 1_000, 500)));
        let mut ids = Vec::new();
        for who in 0..clients {
            ids.push(sim.install(Box::new(Client::new(who, service, window, operations, 2_000))));
        }
        for id in ids {
            sim.world().send(0, id, Message { from: id, kind: kind::START, token: 0, detail: 0 });
        }
        sim.run().expect("the exchange terminates")
    }

    #[test]
    fn every_operation_completes_exactly_once() {
        // The property that says the model moved the work it was given. A
        // simulator whose trace reproduces and whose work quietly evaporated is
        // the worst outcome available, because everything above it goes green.
        let outcome = exchange(0x1234, 3, 2, 2, 8);
        let mut done: Vec<u64> = outcome
            .trace
            .records()
            .iter()
            .filter(|r| r.kind == wrote::DONE)
            .map(|r| r.token)
            .collect();
        assert_eq!(done.len(), 24, "three clients times eight operations");
        done.sort_unstable();
        done.dedup();
        assert_eq!(done.len(), 24, "an operation completed twice");
    }

    #[test]
    fn a_full_queue_refuses_rather_than_swallowing() {
        // Depth one against three clients with a window of two: the queue is
        // full often, and the refusal path has to run. A back-pressure path that
        // never runs is a back-pressure path nobody has tested.
        let outcome = exchange(0x99, 3, 2, 1, 6);
        let refusals = outcome.trace.records().iter().filter(|r| r.kind == wrote::FULL).count();
        assert!(refusals > 0, "the queue was never full, so the refusal path never ran");
        let done = outcome.trace.records().iter().filter(|r| r.kind == wrote::DONE).count();
        assert_eq!(done, 18, "a refusal lost work instead of deferring it");
    }

    #[test]
    fn the_clock_never_goes_backwards_across_a_whole_exchange() {
        let outcome = exchange(0xBEEF, 4, 3, 4, 6);
        let mut last = 0;
        for record in outcome.trace.records() {
            assert!(record.at_ns >= last, "the clock went from {last} to {}", record.at_ns);
            last = record.at_ns;
        }
        assert_eq!(outcome.finished_ns, last);
    }

    #[test]
    fn the_service_order_is_the_seeds_and_not_the_submission_order() {
        // If completions came out in submission order the model would never find
        // a client that assumes they do. Two seeds must be able to produce two
        // orders over the same submissions.
        let order = |seed| {
            exchange(seed, 3, 4, 8, 4)
                .trace
                .records()
                .iter()
                .filter(|r| r.kind == wrote::SERVED)
                .map(|r| r.token)
                .collect::<Vec<_>>()
        };
        let first = order(1);
        assert_eq!(first, order(1), "one seed produced two service orders");
        let mut differs = false;
        for seed in 2..32u64 {
            if order(seed) != first {
                differs = true;
                break;
            }
        }
        assert!(differs, "no seed in thirty-one changed the service order");
    }

    #[test]
    fn a_client_that_finishes_says_so_once() {
        let outcome = exchange(0x77, 2, 1, 4, 3);
        let finished = outcome.trace.records().iter().filter(|r| r.kind == wrote::FINISHED).count();
        assert_eq!(finished, 2, "each client announces finishing exactly once");
    }
}
