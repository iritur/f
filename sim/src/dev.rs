// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What every modelled device has in common: a virtqueue, a registration table,
//! and a completion policy the seed drives.
//!
//! # Three devices, one machine, and why the split is here
//!
//! A block device, a network interface and a display controller differ in their
//! *protocol* — what a request looks like, what the device is allowed to refuse
//! it for, and which requests may overtake which. They do not differ in the
//! machinery underneath: all three sit behind a split virtqueue, all three
//! resolve buffer names through `f_ring::registry`, and all three are entitled
//! to complete work in an order the driver did not choose.
//!
//! So the machinery is written once, here, and the protocol is a [`Protocol`]
//! implementation per device. That is not tidiness for its own sake: it is what
//! makes the three comparable. If each device carried its own queue handling,
//! a difference between two of them could be a difference in the model rather
//! than in the device, and the whole value of a device model is that it is
//! *only* the device.
//!
//! # The four things a device does that a stub does not
//!
//! **It refuses.** A device with too much outstanding answers
//! `RESOURCE`/`DEVICE_FULL`, and a driver with no descriptors left answers
//! `RESOURCE`/`QUOTA_EXHAUSTED`. Two different limits with two different codes,
//! because a client that cannot tell *the device is busy* from *my ring is full*
//! cannot choose between waiting and shedding.
//!
//! **It completes in an order it chooses.** Nothing in virtio says the used ring
//! is in available-ring order, and a harness that completed in submission order
//! would never find the client that assumes it is. Which request goes next is
//! [`World::decide`] at [`Protocol::COMPLETE`], so a seed selects it and a
//! failing seed names it.
//!
//! **It coalesces.** A device may publish several used entries before it rings,
//! and a driver that harvested one per notification would still be correct but
//! would never see two. The decision is [`Protocol::COALESCE`].
//!
//! **It loses work.** A completion the device never publishes is
//! [`Protocol::DROP`], and it is **written into the trace** — a simulator that
//! quietly dropped work would produce a trace that reproduces perfectly and
//! describes nothing.
//!
//! Delay, reordering, coalescing and loss are four separate mechanisms and not
//! one knob with four settings, because a client can be wrong about each of them
//! independently.
//!
//! # Losing a completion ends the device, and that is the protocol
//!
//! RFC 0034 records this as a finding rather than a preference, because it is
//! what driving the real ownership types produced rather than something anyone
//! set out to model.
//!
//! RFC 0024 lets a client take a buffer back without a completion in exactly one
//! case: [`PeerGone`](f_ring::buffers::PeerGone), built from evidence that every
//! outstanding token is void. There is no timeout in that design and no way for
//! a client to give up on its own. So a device that lost a completion and
//! carried on would leave its client holding memory it can never touch and never
//! free — which is a hang, and a quiet one.
//!
//! The model therefore does what a real device does when its queue state and its
//! driver's have come apart: it **resets**. The registrations are retired, the
//! translations go with them — so a transfer it had already started faults
//! rather than landing — and the client is told, which is the one event that
//! makes its buffers reclaimable. `E1-P02`'s *peer death mid-operation* is this
//! path with a different cause, and it is already built.

use f_abi::buf::SetId;
use f_abi::{Cqe, Sqe, buf, error};
use f_ring::registry::Reach;
use f_ring::{completion, refusal};

use crate::proto::{kind, wrote};
use crate::service::Service;
use crate::virtq::{Chain, Part, Queue, Region, Trouble};
use crate::{Actor, ActorId, Message, World};

/// Where a device model puts its virtqueue. Unit: bytes, device space.
pub const QUEUE_BASE: u64 = 0x2000_0000;

/// Where it puts the control memory its request headers live in. Unit: bytes,
/// device space.
///
/// A region of its own, and not a corner of the queue, because that is how the
/// real path is laid out: `kernel/src/arch/x86_64/dma.rs` gives the request
/// header and the status byte a frame separate from both the queue and the data
/// buffer, and says why — the data buffer is the thing whose translation is the
/// experiment, and a page it shared with the header would make the header
/// untranslatable too.
pub const CONTROL_BASE: u64 = 0x3000_0000;

/// One request, as the driver half hands it to the protocol.
#[derive(Clone, Copy, Debug)]
pub struct Request {
    /// The client's own token. Unit: none — returned in the completion.
    pub token: u64,
    /// The position the operation works at. Unit: per-protocol — a sector, a
    /// frame, a display request — and the protocol that reads it is what states
    /// which.
    pub at: u64,
    /// The buffer, as the registration resolved it. Nothing dereferences it.
    pub reach: Reach,
    /// How many requests this device has taken before this one. Unit: requests.
    pub seq: u64,
}

/// What the device did with one chain.
///
/// Deliberately **not** the answer the client gets. The device writes a used
/// entry and whatever its protocol says it writes into the buffers it was given;
/// the driver reads those back and decides what to tell its client, in
/// [`Protocol::harvest`]. Collapsing the two would model a device that tells its
/// driver the answer directly, which no device does and which would hide every
/// bug in the reading.
#[derive(Clone, Copy, Debug)]
pub struct Served {
    /// What the device writes into the used ring. Unit: bytes — and it is
    /// *bytes the device wrote*, which for a transmit queue is zero and for a
    /// read is the payload plus whatever status the protocol appends.
    pub used_len: u32,
    /// What the trace calls this. One of [`crate::proto::wrote`].
    pub label: &'static str,
    /// Whether this request may be overtaken by a later one.
    ///
    /// A device is free to reorder its completions; a *fence* is the protocol
    /// saying it is not. Only the display controller has them today, and the
    /// machinery is here rather than there because the rule is about ordering
    /// and ordering is this file's subject.
    pub fenced: bool,
}

/// The part of a device that is the device rather than the machinery.
///
/// Three implementations: [`crate::blk`], [`crate::net`], [`crate::gpu`]. Each
/// states its own request layout, its own refusals, and its own three decision
/// sites — a site is a stable name a failing seed reports, so two devices
/// sharing one would be two failures a person could not tell apart.
pub trait Protocol {
    /// What the device is called in the trace. At most
    /// [`crate::LABEL_WIDTH`] bytes.
    const NAME: &'static str;
    /// Where the choice of what to complete next is recorded.
    const COMPLETE: &'static str;
    /// Where the choice to lose a completion is recorded.
    const DROP: &'static str;
    /// Where the choice to hold a notification back is recorded.
    const COALESCE: &'static str;

    /// Bytes of control memory one outstanding request needs. Unit: bytes.
    fn control_bytes(&self) -> u32;

    /// Write the request's headers into `control` at `at`, and answer the
    /// descriptor chain that names them.
    ///
    /// This is the *driver* half: it runs before the device sees anything, and
    /// what it writes is what the device will read back out of the same bytes.
    ///
    /// # Errors
    ///
    /// A packed [`f_abi::error`] result for a request this protocol cannot
    /// express — a length past what the control region holds, most likely.
    fn describe(
        &mut self,
        request: &Request,
        control: &mut Region,
        at: u32,
    ) -> Result<Vec<Part>, i32>;

    /// Read one chain as the device reads it, and answer what the device did.
    ///
    /// The *device* half. It sees descriptors and nothing else: it may read the
    /// control region a descriptor names, and it may not read the data buffer,
    /// because a [`Reach`] is an address and a length and this crate has no type
    /// that turns one into bytes — the same absence `f_ring::registry::Reach` is
    /// built around.
    fn serve(&mut self, chain: &Chain, bus: &mut Bus<'_>, extent: u64) -> Served;

    /// Read the device's answer back out of the memory it wrote, and say what
    /// the client is told.
    ///
    /// The *driver* half again, and the round trip closed: what the device wrote
    /// is read from the same bytes rather than passed along in a struct. A block
    /// device answers in a status byte, a display controller in a response
    /// header, and a network interface answers nothing at all — which is a
    /// protocol fact worth a model rather than a gap in one.
    ///
    /// `written` is what the used entry said, `at` is where this request's
    /// control memory starts, and `asked` is the length the client's entry named.
    /// Unit of the answer: bytes on success, a packed [`f_abi::error`] result on
    /// a refusal — which is `Cqe::result`'s own rule.
    fn harvest(&mut self, written: u32, control: &Region, at: u32, asked: u32) -> i32;
}

/// What a device model can address.
///
/// Handed to [`Protocol::serve`] so that a device decodes every descriptor
/// before it reads anything. An address in neither is the model's stand-in for
/// the fault a real remapping unit raises, and the reason it exists is the one
/// `dma.rs` states from the other side: a device that addressed memory the unit
/// cannot see is a device outside the protection, and a model of one would pass
/// for the wrong reason.
pub struct Bus<'d> {
    /// The request headers and status bytes, which the device may read and
    /// write.
    pub control: &'d mut Region,
    /// Every translation the component's domain holds.
    pub grants: &'d crate::service::Grants,
}

impl Bus<'_> {
    /// Where in the control region `at` falls, if it falls there at all.
    #[must_use]
    pub fn control_at(&self, at: u64, len: u32) -> Option<u32> {
        self.control.holds(at, len)
    }

    /// Does the device's domain translate `len` bytes at `at`?
    #[must_use]
    pub fn granted(&self, at: u64, len: u32) -> bool {
        self.grants.reaches(at, len)
    }
}

/// How a device behaves, as a scenario states it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Config {
    /// Requests the device will hold at once. Unit: requests. Past this it
    /// answers `RESOURCE`/`DEVICE_FULL`.
    pub depth: u32,
    /// The shortest the device takes over one request. Unit: nanoseconds.
    pub service_ns: u64,
    /// How much longer than that it may take. Unit: nanoseconds; zero is a
    /// constant service time, which is the scenario that isolates ordering from
    /// timing.
    pub spread_ns: u64,
    /// One completion in this many may be lost, chosen by the seed. Unit:
    /// completions. Zero is never; a value below two is raised to two, because
    /// a device that loses every completion is a device that is off and a
    /// scenario meaning that should say so with `operations`.
    pub lose_one_in: u32,
    /// How large the device is. Unit: per-protocol — sectors for a disk, bytes
    /// of frame for a link, resources for a display — and the protocol that
    /// reads it is what states which.
    pub extent: u64,
    /// Descriptors in the virtqueue. Unit: descriptors; a power of two at most
    /// [`crate::virtq::QUEUE_SIZE`].
    pub queue_size: u16,
    /// Translations the component's domain will hold. Unit: translations.
    pub domain: u32,
}

/// One request, from the moment the driver takes it to the moment its client
/// hears about it.
#[derive(Clone, Copy, Debug)]
struct Job {
    token: u64,
    set: SetId,
    index: u32,
    head: u16,
    slot: u16,
    /// What the client's entry asked for. Unit: bytes.
    len: u32,
    /// Set once the device has taken the chain and decided what it will answer.
    served: Option<Served>,
    /// Set once the used entry is in the ring.
    published: bool,
    /// Where in the arrival order this request sits. Unit: requests.
    seq: u64,
}

/// A device model: one virtqueue, one registration table, one protocol.
pub struct Device<P: Protocol> {
    proto: P,
    queue: Queue,
    control: Region,
    service: Service,
    cfg: Config,
    /// Who the driver answers. A virtqueue has exactly one driver, so a second
    /// client is refused rather than served — a second driver is a second queue,
    /// which is a second device.
    client: Option<ActorId>,
    jobs: Vec<Job>,
    /// One bit per control slot, set while a job holds it.
    slots: u64,
    arrivals: u64,
    /// The device lost work and reset. Everything after is refused, because a
    /// device whose queue state and driver's have come apart has nothing true
    /// to say.
    reset: bool,
}

impl<P: Protocol> Device<P> {
    /// A device with an empty queue and an empty table.
    ///
    /// # Errors
    ///
    /// [`Trouble::Layout`] for a queue size the layout cannot hold.
    pub fn new(proto: P, cfg: Config) -> Result<Self, Trouble> {
        let queue = Queue::new(cfg.queue_size, QUEUE_BASE)?;
        let slots = u32::from(queue.size());
        let control = Region::new(proto.control_bytes().saturating_mul(slots), CONTROL_BASE);
        Ok(Self {
            proto,
            queue,
            control,
            service: Service::new(cfg.domain),
            cfg,
            client: None,
            jobs: Vec::new(),
            slots: 0,
            arrivals: 0,
            reset: false,
        })
    }

    /// Answer one entry with a refusal, and tell the client.
    fn deny(&self, world: &mut World, me: ActorId, token: u64, packed: i32, detail: u64) {
        let Some(client) = self.client else {
            return;
        };
        let now = world.clock();
        world.record(me, P::NAME, wrote::DENIED, token, detail);
        world.wire().answer(me, client, refusal(token, packed, detail, now));
        world.send(0, client, Message { from: me, kind: kind::CQE, token, detail: 0 });
    }

    /// Answer one entry with a completion, and tell the client.
    fn answer(&self, world: &mut World, me: ActorId, cqe: Cqe) {
        let Some(client) = self.client else {
            return;
        };
        let token = cqe.user_data;
        world.wire().answer(me, client, cqe);
        world.send(0, client, Message { from: me, kind: kind::CQE, token, detail: 0 });
    }

    /// The lowest control slot nobody holds.
    fn claim_slot(&mut self) -> Option<u16> {
        for index in 0..self.queue.size() {
            let bit = 1u64 << (index & 63);
            if self.slots & bit == 0 {
                self.slots |= bit;
                return Some(index);
            }
        }
        None
    }

    /// Take one submission off the wire and put a chain on the queue.
    fn submit(&mut self, world: &mut World, me: ActorId, from: ActorId) {
        let Some(entry) = world.wire().take(from, me) else {
            // A doorbell with nothing behind it — ordinary on a real ring, and
            // recorded rather than treated as an error for that reason.
            world.record(me, P::NAME, kind::SUBMIT, 0, u64::MAX);
            return;
        };
        match self.client {
            None => self.client = Some(from),
            Some(known) if known == from => {}
            Some(_) => {
                // A virtqueue has one driver. A second client is refused with
                // the code for authority it does not hold, rather than served
                // into a queue whose descriptors the first client is also using.
                let now = world.clock();
                world.record(me, P::NAME, wrote::DENIED, entry.user_data, u64::MAX);
                world.wire().answer(
                    me,
                    from,
                    refusal(
                        entry.user_data,
                        error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                        0,
                        now,
                    ),
                );
                world.send(
                    0,
                    from,
                    Message { from: me, kind: kind::CQE, token: entry.user_data, detail: 0 },
                );
                return;
            }
        }

        if self.reset {
            self.deny(
                world,
                me,
                entry.user_data,
                error::pack(error::PEER, error::peer::EPOCH_CHANGED),
                0,
            );
            return;
        }

        if buf::opcode::is_registration(entry.opcode) {
            let now = world.clock();
            let cqe = self.service.register(&entry, now);
            world.record(me, P::NAME, wrote::REGISTER, entry.user_data, entry.len.into());
            self.answer(world, me, cqe);
            return;
        }

        self.accept(world, me, &entry);
    }

    /// Resolve one entry's buffer and build its chain, or refuse it.
    fn accept(&mut self, world: &mut World, me: ActorId, entry: &Sqe) {
        let token = entry.user_data;
        if u32::from(self.queue.outstanding()) >= self.cfg.depth {
            // The device is busy. Refused *before* the buffer is resolved, so a
            // refusal leaves the registration exactly as it found it.
            self.deny(
                world,
                me,
                token,
                error::pack(error::RESOURCE, error::resource::DEVICE_FULL),
                u64::from(self.cfg.depth),
            );
            return;
        }

        let (set, index, reach) = match self.service.resolve(entry) {
            Ok(resolved) => resolved,
            Err((packed, detail)) => {
                self.deny(world, me, token, packed, detail);
                return;
            }
        };

        let Some(slot) = self.claim_slot() else {
            self.service.release(set, index).ok();
            self.deny(
                world,
                me,
                token,
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                u64::from(self.queue.size()),
            );
            return;
        };

        let request = Request { token, at: entry.offset, reach, seq: self.arrivals };
        let at = u32::from(slot).saturating_mul(self.proto.control_bytes());
        let parts = match self.proto.describe(&request, &mut self.control, at) {
            Ok(parts) => parts,
            Err(packed) => {
                self.release_slot(slot);
                self.service.release(set, index).ok();
                self.deny(world, me, token, packed, 0);
                return;
            }
        };

        let head = match self.queue.chain(&parts) {
            Ok(head) => head,
            Err(_) => {
                // The driver's own ring is full. A different limit from the
                // device's, with a different code, because a client that cannot
                // tell them apart cannot choose between waiting and shedding.
                self.release_slot(slot);
                self.service.release(set, index).ok();
                self.deny(
                    world,
                    me,
                    token,
                    error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                    u64::from(self.queue.size()),
                );
                return;
            }
        };

        if self.queue.offer(head).is_err() {
            self.release_slot(slot);
            self.service.release(set, index).ok();
            self.deny(
                world,
                me,
                token,
                error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS),
                u64::from(head),
            );
            return;
        }

        self.arrivals = self.arrivals.saturating_add(1);
        self.jobs.push(Job {
            token,
            set,
            index,
            head,
            slot,
            len: entry.len,
            served: None,
            published: false,
            seq: request.seq,
        });
        world.record(me, P::NAME, wrote::QUEUED, token, u64::from(head));

        // The doorbell. A separate event from the submission, because the gap
        // between publishing an available index and a device reading it is
        // where the interesting orderings live.
        world.send(0, me, Message { from: me, kind: kind::POLL, token, detail: 0 });
    }

    /// The device reads the available ring and decides what each chain earns.
    fn poll(&mut self, world: &mut World, me: ActorId) {
        if self.reset {
            return;
        }
        loop {
            let chain = match self.queue.take() {
                Ok(Some(chain)) => chain,
                Ok(None) => return,
                Err(_) => {
                    // A ring the device cannot walk. Nothing to answer and
                    // nobody to answer it to, so the device stops taking work
                    // and says so — which is what a real one does with a
                    // malformed queue.
                    world.record(me, P::NAME, wrote::IOERR, 0, u64::MAX);
                    self.fall_over(world, me);
                    return;
                }
            };

            let mut bus = Bus { control: &mut self.control, grants: self.service.grants() };
            let served = self.proto.serve(&chain, &mut bus, self.cfg.extent);
            let Some(job) = self.jobs.iter_mut().find(|job| job.head == chain.head) else {
                world.record(me, P::NAME, wrote::IOERR, 0, u64::from(chain.head));
                continue;
            };
            job.served = Some(served);
            let token = job.token;
            world.record(me, P::NAME, wrote::TAKEN, token, u64::from(served.used_len));

            // The service time. `spread_ns + 1` so a spread of zero is a
            // constant time rather than a division by zero, and so that the
            // draw happens either way: a scenario's spread must not change how
            // far along the randomness stream a run is, or two scenarios would
            // not be comparable.
            let elapsed = self
                .cfg
                .service_ns
                .saturating_add(world.draw() % self.cfg.spread_ns.saturating_add(1));
            world.send(elapsed, me, Message { from: me, kind: kind::SERVICE, token, detail: 0 });
        }
    }

    /// One request's service time has elapsed: publish one completion, or lose
    /// it.
    fn service(&mut self, world: &mut World, me: ActorId) {
        if self.reset {
            return;
        }
        let candidates = self.publishable();
        let arity = u32::try_from(candidates.len()).unwrap_or(u32::MAX);
        if arity == 0 {
            return;
        }
        let taken = world.decide(P::COMPLETE, arity) as usize;
        let Some(head) = candidates.get(taken).copied() else {
            return;
        };
        let Some(job) = self.jobs.iter().find(|job| job.head == head).copied() else {
            return;
        };
        let Some(served) = job.served else {
            return;
        };

        if self.loses(world) {
            world.record(me, P::NAME, wrote::DROPPED, job.token, u64::from(job.head));
            self.fall_over(world, me);
            return;
        }

        if self.queue.publish(job.head, served.used_len).is_err() {
            world.record(me, P::NAME, wrote::IOERR, job.token, u64::from(job.head));
            self.fall_over(world, me);
            return;
        }
        if let Some(slot) = self.jobs.iter_mut().find(|j| j.head == head) {
            slot.published = true;
        }
        world.record(me, P::NAME, served.label, job.token, u64::from(served.used_len));

        // Coalescing: hold the notification back and let the next completion
        // carry both. Only legal while something else is still to be published,
        // because that is what guarantees another notification is coming — a
        // device that held the last one would have published a completion
        // nobody will ever harvest.
        let more = !self.publishable().is_empty();
        if more && world.decide(P::COALESCE, 2) == 1 {
            world.record(me, P::NAME, wrote::HELD, job.token, u64::from(job.head));
            return;
        }
        world.send(0, me, Message { from: me, kind: kind::REAP, token: job.token, detail: 0 });
    }

    /// The driver harvests the used ring and answers its client.
    fn reap(&mut self, world: &mut World, me: ActorId) {
        if self.reset {
            return;
        }
        loop {
            let (head, written) = match self.queue.harvest() {
                Ok(Some(entry)) => entry,
                Ok(None) => return,
                Err(_) => {
                    world.record(me, P::NAME, wrote::IOERR, 0, u64::MAX);
                    self.fall_over(world, me);
                    return;
                }
            };
            let Some(at) = self.jobs.iter().position(|job| job.head == head) else {
                world.record(me, P::NAME, wrote::IOERR, 0, u64::from(head));
                continue;
            };
            // `remove` and not `swap_remove`: the job list is in arrival order,
            // and `publishable` hands that order to `World::decide` as the list
            // it indexes into. A removal that shuffled it would leave the
            // decision meaningful only against the history of removals, which is
            // reproducible and unreadable.
            let job = self.jobs.remove(at);
            let control_at = u32::from(job.slot).saturating_mul(self.proto.control_bytes());
            let result = self.proto.harvest(written, &self.control, control_at, job.len);

            let _ = self.queue.release(head);
            self.release_slot(job.slot);
            self.service.release(job.set, job.index).ok();

            let now = world.clock();
            let cqe = if result < 0 {
                refusal(job.token, result, u64::from(written), now)
            } else {
                completion(job.token, result, now)
            };
            self.answer(world, me, cqe);
        }
    }

    /// Which chains have been served and not yet published.
    ///
    /// The fence rule lives here rather than in a protocol, because it is a
    /// statement about *ordering* and ordering is this file's subject: a fenced
    /// request may not be published while an earlier fenced request is still
    /// unpublished. Everything unfenced is a free choice, which is what the
    /// specification actually permits and what makes the fence worth having.
    fn publishable(&self) -> Vec<u16> {
        let earliest_fence = self
            .jobs
            .iter()
            .filter(|job| !job.published && job.served.is_some_and(|s| s.fenced))
            .map(|job| job.seq)
            .min();

        self.jobs
            .iter()
            .filter(|job| !job.published && job.served.is_some())
            .filter(|job| {
                let fenced = job.served.is_some_and(|s| s.fenced);
                !fenced || earliest_fence == Some(job.seq)
            })
            .map(|job| job.head)
            .collect()
    }

    /// Is this the completion the seed decided to lose?
    fn loses(&self, world: &mut World) -> bool {
        if self.cfg.lose_one_in == 0 {
            return false;
        }
        // Raised to two, because a choice with one alternative is not recorded
        // and does not consume an occurrence — so an arity of one would mean
        // *always*, silently, and a scenario would lose every completion while
        // its decision log said nothing happened.
        world.decide(P::DROP, self.cfg.lose_one_in.max(2)) == 0
    }

    /// The device and its driver have come apart. Reset, and tell the client.
    ///
    /// Everything the client holds becomes reclaimable at this instant and not
    /// before: [`Service::retire_all`] takes the translations with the
    /// registrations, which is what makes a transfer the device had already
    /// started fault rather than land in memory the client is about to reuse.
    fn fall_over(&mut self, world: &mut World, me: ActorId) {
        if self.reset {
            return;
        }
        self.reset = true;
        let retired = self.service.retire_all();
        self.jobs.clear();
        world.record(me, P::NAME, wrote::RESET, 0, u64::try_from(retired).unwrap_or(u64::MAX));
        if let Some(client) = self.client {
            world.send(0, client, Message { from: me, kind: kind::GONE, token: 0, detail: 0 });
        }
    }

    /// Give a control slot back.
    fn release_slot(&mut self, slot: u16) {
        self.slots &= !(1u64 << (slot & 63));
    }
}

impl<P: Protocol> Actor for Device<P> {
    fn name(&self) -> &'static str {
        P::NAME
    }

    fn deliver(&mut self, world: &mut World, me: ActorId, message: Message) {
        match message.kind {
            kind::SUBMIT => self.submit(world, me, message.from),
            kind::POLL => self.poll(world, me),
            kind::SERVICE => self.service(world, me),
            kind::REAP => self.reap(world, me),
            // R04: an unknown kind is refused rather than ignored. There is no
            // peer to answer here, so the refusal is a record — and a record is
            // enough, because it changes the digest and therefore fails the
            // comparison this crate exists to make.
            other => world.record(me, P::NAME, other, message.token, u64::MAX),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::blk::Blk;
    use crate::gpu::Gpu;
    use crate::net::Net;

    /// Every decision site the three devices declare.
    const SITES: &[&str] = &[
        Blk::COMPLETE,
        Blk::DROP,
        Blk::COALESCE,
        Net::COMPLETE,
        Net::DROP,
        Net::COALESCE,
        Gpu::COMPLETE,
        Gpu::DROP,
        Gpu::COALESCE,
    ];

    #[test]
    fn no_two_devices_share_a_decision_site() {
        // A site is what a failing seed reports and what a sweep aims at. Two
        // devices sharing one would be two failures a person could not tell
        // apart — and worse, adding the second device would move the first's
        // occurrence counts, so every seed recorded against it would stop
        // reproducing its run. `decide.rs` spends four paragraphs on exactly
        // that property; this is the check that the device models keep it.
        let mut sorted = SITES.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "two device models draw at one site");
    }

    #[test]
    fn every_site_names_the_device_it_belongs_to() {
        // A site read out of a minimised failure has to say where it struck
        // without a lookup. Checked rather than trusted, because the constant
        // and the name are two strings and nothing else holds them together.
        for (name, sites) in [
            (Blk::NAME, [Blk::COMPLETE, Blk::DROP, Blk::COALESCE]),
            (Net::NAME, [Net::COMPLETE, Net::DROP, Net::COALESCE]),
            (Gpu::NAME, [Gpu::COMPLETE, Gpu::DROP, Gpu::COALESCE]),
        ] {
            for site in sites {
                assert!(site.starts_with(name), "`{site}` does not name `{name}`");
            }
        }
    }
}
