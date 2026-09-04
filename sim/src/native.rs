// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The other thing a client can be pointed at: a component with no device
//! behind it.
//!
//! # What this is for
//!
//! Component substitution is a claim with two halves, and a crate containing
//! only device models can state one of them. This is the other: the same client,
//! the same submissions, the same buffer-ownership rules, and **no model at
//! all** — a peer built out of `f_ring::registry` and nothing else, which
//! resolves the buffer a submission names, does the work, releases it, and
//! answers.
//!
//! It is what a service inside this system looks like when the thing it serves
//! is not hardware. `user/store` is one, the supervisor's control ring is
//! another, and every future component that answers a ring without touching a
//! device is a third. So this is not a stub standing in for a device: it is the
//! shape of most of the system's peers, and the one the device models have to be
//! substitutable *for* if the property is worth anything.
//!
//! # The comparison this makes possible
//!
//! [`crate::scenario`]'s substitution test runs one client against a modelled
//! block device with no reordering, no coalescing and no loss, and against this,
//! and requires the client's own records to be identical. What that says is that
//! **every difference between the two is something the scenario asked for** — a
//! latency, an interleaving, a lost completion — rather than an accident of
//! which peer was installed. A client whose control flow depended on that would
//! be a client the user-space-driver argument does not hold for.
//!
//! What the comparison deliberately does not cover is `gpu`, and that is not a
//! gap: a display controller answers a different question from a disk, so the
//! same client asks it and gets a different number back. Substitution is a claim
//! about a client and its service's protocol, not a claim that every service
//! answers alike.
//!
//! # Why it still refuses, and still takes time
//!
//! Both are deliberate. A service with no limit never refuses, and a client
//! whose back-pressure path runs against one peer and not the other is a client
//! the substitution claim is quietly weaker for. A service that answered inside
//! its own submission would collapse the interleaving the timeline exists to
//! choose, and the comparison would then be against something that is not a peer
//! at all.

use f_abi::buf::SetId;
use f_abi::{buf, error};
use f_ring::{completion, refusal};

use crate::proto::{kind, wrote};
use crate::service::Service;
use crate::{Actor, ActorId, Message, World};

/// One submission this peer has accepted and not yet answered.
#[derive(Clone, Copy, Debug)]
struct Job {
    token: u64,
    set: SetId,
    index: u32,
    /// What the client's entry asked for. Unit: bytes.
    len: u32,
}

/// A component that serves submissions with no device under it.
pub struct Native {
    service: Service,
    /// How many submissions it will hold at once. Unit: submissions.
    depth: u32,
    /// The shortest it takes over one submission. Unit: nanoseconds.
    service_ns: u64,
    /// How much longer than that it may take. Unit: nanoseconds.
    spread_ns: u64,
    /// Who it answers. One client, for the reason a virtqueue has one driver:
    /// a channel has two ends. A second one is refused `AUTHORITY/NO_SUCH_CAP`
    /// in [`Native::submit`] rather than admitted — the rule is the code's and
    /// not this sentence's, which is what R01 asks of a field like this.
    client: Option<ActorId>,
    jobs: Vec<Job>,
}

impl Native {
    /// What this actor is called in the trace.
    pub const NAME: &'static str = "native";

    /// Where it records which of several accepted submissions is answered next.
    ///
    /// Its own site, not shared with any device: a site is what a failing seed
    /// reports, and a peer that borrowed another's would move that other's
    /// occurrence counts the moment it was installed — invalidating every seed
    /// recorded against it. `decide.rs` is where that property is argued.
    pub const COMPLETE: &'static str = "native.complete";

    /// A component holding at most `depth` submissions, taking between
    /// `service_ns` and `service_ns + spread_ns` over each, over a domain
    /// holding `domain` translations.
    #[must_use]
    pub fn new(depth: u32, service_ns: u64, spread_ns: u64, domain: u32) -> Self {
        Self {
            service: Service::new(domain),
            depth: depth.max(1),
            service_ns,
            spread_ns,
            client: None,
            jobs: Vec::new(),
        }
    }

    /// Write this peer out, tag first.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) {
        out.u32(crate::snap::tag::NATIVE);
        out.u32(self.depth);
        out.u64(self.service_ns);
        out.u64(self.spread_ns);
        self.service.save(out);
        out.bool(self.client.is_some());
        out.u32(self.client.map_or(0, |id| id.0));
        out.count(self.jobs.len());
        for job in &self.jobs {
            out.u64(job.token);
            out.u32(job.set.bits());
            out.u32(job.index);
            out.u32(job.len);
        }
    }

    /// Read one back.
    ///
    /// The jobs are what put the registration table's lent bitmap back, exactly
    /// as they are for a device: a job is a buffer this peer has resolved and
    /// not released, so the fact lives here and does not need a second copy.
    pub(crate) fn load(input: &mut crate::snap::Reader<'_>) -> Self {
        // Refused rather than clamped, for the reason `Grants::load` states: a
        // repaired file restores into a world that is plausible and is not the
        // world the file described. A wire of no depth accepts nothing, which is
        // a peer no scenario can build and therefore a file this crate did not
        // write.
        let depth = input.u32();
        if depth == 0 {
            input.refuse(crate::snap::Broken::Bounds("a peer that would hold no submission"));
        }
        let service_ns = input.u64();
        let spread_ns = input.u64();
        let mut service = Service::load(input);
        let known = input.bool();
        let who = input.u32();
        let client = known.then_some(ActorId(who));
        let count = input.count(20, "more jobs than the file could hold");
        let mut jobs = Vec::with_capacity(count);
        for _ in 0..count {
            jobs.push(Job {
                token: input.u64(),
                set: SetId::from_bits(input.u32()),
                index: input.u32(),
                len: input.u32(),
            });
        }
        if !input.faulted() {
            for job in &jobs {
                if service.relend(job.set, job.index, job.len).is_err() {
                    input.refuse(crate::snap::Broken::Diverged(
                        "a buffer the peer held, which its table would not lend again",
                    ));
                    break;
                }
            }
        }
        Self { service, depth, service_ns, spread_ns, client, jobs }
    }

    /// Answer one entry and tell the client.
    fn answer(&self, world: &mut World, me: ActorId, to: ActorId, cqe: f_abi::Cqe) {
        let token = cqe.user_data;
        world.wire().answer(me, to, cqe);
        world.send(0, to, Message { from: me, kind: kind::CQE, token, detail: 0 });
    }

    /// Take one submission off the wire.
    fn submit(&mut self, world: &mut World, me: ActorId, from: ActorId) {
        let Some(entry) = world.wire().take(from, me) else {
            world.record(me, Self::NAME, kind::SUBMIT, 0, u64::MAX);
            return;
        };
        let token = entry.user_data;
        let now = world.clock();

        // One client, and the refusal is what makes that a mechanism rather
        // than a sentence in the field's documentation. This used to be an
        // unconditional `self.client = Some(from)`, which is not *one client*
        // but *the most recent one*: a second client's submission would have
        // redirected every completion still owed to the first, and the first
        // would have waited forever on tokens it could no longer be told about.
        // `Device::submit` in `dev.rs` refuses the same way for the same reason
        // — a channel has two ends — and the peer the substitution test compares
        // the device models against is the one that should be weakest on it.
        match self.client {
            None => self.client = Some(from),
            Some(known) if known == from => {}
            Some(_) => {
                world.record(me, Self::NAME, wrote::DENIED, token, u64::MAX);
                let cqe = refusal(
                    token,
                    error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                    0,
                    now,
                );
                self.answer(world, me, from, cqe);
                return;
            }
        }

        if buf::opcode::is_registration(entry.opcode) {
            let cqe = self.service.register(&entry, now);
            world.record(me, Self::NAME, wrote::REGISTER, token, u64::from(entry.len));
            self.answer(world, me, from, cqe);
            return;
        }

        if u32::try_from(self.jobs.len()).unwrap_or(u32::MAX) >= self.depth {
            world.record(me, Self::NAME, wrote::DENIED, token, u64::from(self.depth));
            let cqe = refusal(
                token,
                error::pack(error::RESOURCE, error::resource::DEVICE_FULL),
                u64::from(self.depth),
                now,
            );
            self.answer(world, me, from, cqe);
            return;
        }

        let (set, index, reach) = match self.service.resolve(&entry) {
            Ok(resolved) => resolved,
            Err((packed, detail)) => {
                world.record(me, Self::NAME, wrote::DENIED, token, detail);
                let cqe = refusal(token, packed, detail, now);
                self.answer(world, me, from, cqe);
                return;
            }
        };

        self.jobs.push(Job { token, set, index, len: reach.len });
        world.record(me, Self::NAME, wrote::QUEUED, token, u64::from(reach.len));

        // The work takes time, and the answer is scheduled rather than given —
        // which is what makes this a peer rather than a function call.
        // `spread_ns + 1` for the reason the device models give: the draw must
        // happen either way, or a scenario's spread would change how far along
        // the randomness stream a run is.
        let elapsed =
            self.service_ns.saturating_add(world.draw() % self.spread_ns.saturating_add(1));
        world.send(elapsed, me, Message { from: me, kind: kind::SERVICE, token, detail: 0 });
    }

    /// A service time has elapsed: answer one of the submissions being held.
    ///
    /// *One of*, and the seed chooses which — the same freedom the device models
    /// take, for the same reason. A service that answered in arrival order would
    /// be a service a client could assume things about, and the assumption would
    /// hold here and fail against a disk.
    fn finish(&mut self, world: &mut World, me: ActorId) {
        let arity = u32::try_from(self.jobs.len()).unwrap_or(u32::MAX);
        if arity == 0 {
            return;
        }
        let taken = world.decide(Self::COMPLETE, arity) as usize;
        if taken >= self.jobs.len() {
            return;
        }
        let job = self.jobs.remove(taken);

        let now = world.clock();
        let cqe = if self.service.release(job.set, job.index).is_err() {
            // This peer's own bookkeeping gone wrong rather than a client
            // misbehaving — a buffer released twice. Refused to the client
            // rather than answered, because a service that answered anyway
            // would be making a live buffer look free.
            world.record(me, Self::NAME, wrote::IOERR, job.token, u64::from(job.index));
            refusal(
                job.token,
                error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS),
                u64::from(job.index),
                now,
            )
        } else {
            world.record(me, Self::NAME, wrote::SERVED, job.token, u64::from(job.len));
            completion(job.token, i32::try_from(job.len).unwrap_or(i32::MAX), now)
        };

        let Some(client) = self.client else {
            return;
        };
        self.answer(world, me, client, cqe);
    }
}

impl Actor for Native {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::SUBMIT => self.submit(world, me, message.from),
            kind::SERVICE => self.finish(world, me),
            // R04: an unknown kind is recorded rather than ignored. It changes
            // the digest, which is what makes the record an answer rather than
            // a note.
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
    use crate::client::App;
    use crate::{Simulation, Trouble};

    /// One client against one native peer, with a window of one.
    fn run(seed: u64, operations: u32) -> Result<crate::Outcome, Trouble> {
        let mut sim = Simulation::new(seed, 100_000);
        let peer = sim.install(Box::new(Native::new(4, 1_000, 0, 4)));
        let app = sim.install(Box::new(App::new(0, peer, 1, operations, 256, 2_000, 4)));
        sim.world().send(0, app, Message { from: app, kind: kind::START, token: 0, detail: 0 });
        sim.run()
    }

    #[test]
    fn a_client_registers_binds_and_completes_every_operation() {
        // The whole exchange through the real types: a registration answered by
        // a real `Table`, a naming built from that completion and from nothing
        // else, a set carved into `BUFFERS` buffers, and every one of them lent
        // and returned.
        let outcome = run(0x1234, 6).expect("the exchange terminates");
        let records = outcome.trace.records();
        let done = records.iter().filter(|r| r.kind == wrote::DONE).count();
        assert_eq!(done, 6, "an operation was lost between the client and the peer");
        assert_eq!(records.iter().filter(|r| r.kind == wrote::BOUND).count(), 1);
        assert_eq!(records.iter().filter(|r| r.kind == wrote::FINISHED).count(), 1);
    }

    #[test]
    fn a_second_client_is_refused_rather_than_answered() {
        // The field said *one client* and the code took whoever spoke last,
        // which is a different thing: the second client's submission would have
        // pointed every completion still owed to the first at the wrong actor,
        // and the first would have waited forever on tokens nobody would
        // mention again. Nothing in the shipped scenarios builds that — one peer
        // per client, always — so this is the arrangement made on purpose,
        // because a rule nothing exercises is a rule that quietly stops holding.
        let mut sim = Simulation::new(0x5EED, 100_000);
        let peer = sim.install(Box::new(Native::new(4, 1_000, 0, 4)));
        let first = sim.install(Box::new(App::new(0, peer, 1, 4, 256, 2_000, 4)));
        let second = sim.install(Box::new(App::new(1, peer, 1, 4, 256, 2_000, 4)));
        sim.world().send(0, first, Message { from: first, kind: kind::START, token: 0, detail: 0 });
        // The second starts later rather than at the same instant, so which of
        // the two the peer accepts is a fact about the arrangement and not about
        // the seed. The seed's freedom to choose between two channels due at one
        // instant is the machinery working, and a test that depended on which
        // way it went would be a test of the seed.
        sim.world().send(
            5_000,
            second,
            Message { from: second, kind: kind::START, token: 0, detail: 0 },
        );
        let outcome = sim.run().expect("the exchange terminates");
        let records = outcome.trace.records();

        // The refusal happened, and it happened to the second client's tokens:
        // `App::token` puts the client's own number in the high half, so a
        // denial carrying a token above the boundary is the second client being
        // told it holds no channel here.
        let denied: Vec<u64> = records
            .iter()
            .filter(|r| r.actor == Native::NAME && r.kind == wrote::DENIED)
            .map(|r| r.token)
            .collect();
        assert!(!denied.is_empty(), "a second client was admitted to a peer that has one");
        assert!(
            denied.iter().all(|token| token >> 32 == 1),
            "the peer refused the client it had already accepted"
        );
        // And the first client was not disturbed by any of it: it finished its
        // work, which is the property the refusal exists to protect.
        assert!(
            records.iter().any(|r| r.kind == wrote::FINISHED && r.token == 0),
            "the client that held the channel did not finish"
        );
    }

    #[test]
    fn no_buffer_comes_back_altered() {
        // The ownership property, read out of the trace rather than asserted
        // about the types: the client stamps a byte derived from the token
        // before it lends a buffer and checks it on the way back, and a `done`
        // carrying `u64::MAX` is what a mismatch looks like. It cannot happen —
        // there is no method on `InFlight` that reaches the bytes — which is
        // exactly why it is worth checking that the model did not find a way.
        for seed in [1u64, 2, 3, 0xF00D] {
            let outcome = run(seed, 8).expect("the exchange terminates");
            assert!(
                outcome
                    .trace
                    .records()
                    .iter()
                    .all(|r| r.kind != wrote::DONE || r.detail != u64::MAX),
                "seed {seed}: a buffer came back with bytes nobody in this model can write"
            );
        }
    }
}
