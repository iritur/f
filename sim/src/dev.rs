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
use f_abi::deadline::{Admitted, Callee, Caller, Inherited};
use f_abi::{Cqe, Sqe, buf, class, error};
use f_ring::registry::Reach;
use f_ring::{completion, refusal};

use crate::fault::{Class, Fault};
use crate::proto::{kind, wrote};
use crate::service::Service;
use crate::virtq::{Chain, Part, Queue, Region, Trouble};
use crate::{Actor, ActorId, Message, World};

/// The class a modelled driver is admitted for. Unit: none — an `f_abi::class`
/// ordinal.
///
/// Soft, because that is what `user/virtio-blk/manifest.toml` declares and this
/// model is of that driver. It is a ceiling, so a hard-class request arriving
/// here is served as soft and the completion says so — which is the same
/// `SHORTFALL` the real boot prints, reached by the same call.
pub const ADMITTED: u16 = class::SOFT;

/// What a modelled channel says about the peer submitting on it.
/// Unit: none — an `f_abi::class` ordinal.
///
/// Hard, so that a client claiming the hard class is not refused for it and the
/// scenario is about the *order* rather than about bound 2. The refusal path is
/// `cargo xtask deadline unadmitted`'s and `abi/src/deadline.rs`'s; a scenario
/// that exercised it here would be a third copy of one assertion.
pub const CLIENT_ADMITTED: u16 = class::HARD;

/// The least a modelled driver claims to need from arrival to completion.
/// Unit: nanoseconds.
///
/// **The bound the boot cannot reach.** RFC 0025's third bound floors an
/// inherited deadline at arrival plus this, and arrival on the real driver is
/// zero because a component has no clock — `DEADLINE_GAP` in `xtask` is that
/// declared. Here the clock is the model's own and arrival is real, so a
/// deadline in the past or inside this window is floored exactly as the RFC
/// says. A sweep that explores the ordering therefore explores one bound more
/// than any boot does.
pub const FLOOR_NS: u64 = 2_000;

/// How many requests a modelled driver holds before it refuses. Unit: requests.
///
/// Larger than any scenario's `clients * window`, so that the refusal is
/// reachable by writing a scenario rather than by accident — the same reason
/// `Config::domain` keeps a spare translation.
const PENDING_MAX: usize = 64;

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
    /// Which [`crate::snap::tag`] a device over this protocol is written out
    /// under.
    ///
    /// On the protocol rather than on [`Device`] because `Device<P>` is one
    /// type and a snapshot has to name three, and a tag chosen by the loader
    /// would be a fourth place that has to agree with this one.
    const TAG: u32;
    /// Where the choice of what to complete next is recorded.
    const COMPLETE: &'static str;
    /// Where the choice to lose a completion is recorded.
    const DROP: &'static str;
    /// Where the choice to hold a notification back is recorded.
    const COALESCE: &'static str;

    /// The fault classes this protocol's [`Protocol::serve`] actually reads off
    /// the [`Bus`], and therefore the only ones a scenario may arm against it.
    ///
    /// A declaration and not a description. `Device::poll` consults exactly the
    /// classes named here, so a scenario that arms one a device ignores never
    /// strikes — and
    /// `fault::tests::every_armed_scenario_actually_strikes_and_writes_it_down`
    /// fails, which is the point. Without it such a scenario would strike, write
    /// the strike into the hashed artefact, and change nothing about the run:
    /// a site that is consulted but not exercised, passing for coverage.
    ///
    /// Only the two bus classes belong here. The rest — an allocation refused at
    /// the domain, a page fault added to the service time, a peer that stops,
    /// a torn doorbell — are the machinery's rather than the protocol's, and
    /// every device honours them by construction.
    const HONOURS: &'static [Class];

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

    /// Write whatever state this protocol keeps of its own into a snapshot.
    ///
    /// Nothing, by default, and that default is the honest one: a block device
    /// and a network interface are functions of the bytes in front of them and
    /// keep no state between chains. A display controller holds resources and
    /// overrides this.
    ///
    /// It is a pair of methods on the protocol rather than a field the machinery
    /// serialises because the machinery does not know what a protocol keeps —
    /// and a snapshot written by somebody who had to guess is the failure this
    /// whole module is against.
    fn save_state(&self, out: &mut crate::snap::Writer) {
        let _ = out;
    }

    /// Read it back onto a protocol built the way a scenario builds one.
    fn load_state(&mut self, input: &mut crate::snap::Reader<'_>) {
        let _ = input;
    }
}

/// What is broken about the machine, for the length of one chain.
///
/// Two of `E1-P02`'s seven classes are things a device *cannot detect*: a
/// translation the unit declines to answer, and a write of its own that does not
/// land. Neither is expressible as a refusal the model could return, because
/// from the device's side neither has happened — which is exactly why they are
/// worth injecting, and why they arrive on the bus rather than as a parameter to
/// a protocol.
///
/// Empty is the ordinary machine, and every scenario that arms nothing gets it.
/// `fault.rs` is where each class states the response it demands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Injured {
    /// The domain will not answer this transfer's translation, though it holds
    /// one. Unit: none. `fault::Class::MapFault`.
    pub translation: bool,
    /// The device's last write into control memory does not land. Unit: none.
    /// `fault::Class::Partial`.
    pub last_write: bool,
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
    /// What is broken about the machine for this chain.
    ///
    /// Private, and reached through [`Bus::granted`] and [`Bus::writes_land`],
    /// so that a protocol asks *may I* rather than *is a fault armed* — a
    /// protocol that branched on the second would be a device model that knew it
    /// was being tested.
    injured: Injured,
}

impl<'d> Bus<'d> {
    /// A bus on a machine with nothing wrong with it.
    #[must_use]
    pub fn new(control: &'d mut Region, grants: &'d crate::service::Grants) -> Self {
        Self { control, grants, injured: Injured::default() }
    }

    /// The same bus, on a machine with something wrong with it.
    #[must_use]
    pub const fn injured(mut self, injured: Injured) -> Self {
        self.injured = injured;
        self
    }
}

impl Bus<'_> {
    /// Where in the control region `at` falls, if it falls there at all.
    #[must_use]
    pub fn control_at(&self, at: u64, len: u32) -> Option<u32> {
        self.control.holds(at, len)
    }

    /// Does the device's domain translate `len` bytes at `at`?
    ///
    /// `false` under an injected translation fault, whatever the domain holds.
    /// The device cannot tell the two apart and must not be able to: on real
    /// silicon a transfer the unit declines and a transfer to an address nobody
    /// granted are the same fault, and a model where the device could
    /// distinguish them would be a model of a machine that reported more than
    /// the hardware does.
    #[must_use]
    pub fn granted(&self, at: u64, len: u32) -> bool {
        !self.injured.translation && self.grants.reaches(at, len)
    }

    /// Will the device's last write into control memory land?
    ///
    /// `false` under an injected partial write: the payload moved and the answer
    /// did not. A protocol whose answer is a byte in shared memory must then
    /// leave that byte alone, so the driver reads back what *it* wrote — which
    /// is the whole reason `blk.rs` writes `0xFF` there first.
    #[must_use]
    pub const fn writes_land(&self) -> bool {
        !self.injured.last_write
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
    /// Whether the driver half orders what it posts by what
    /// `f_abi::deadline::inherit` returned, rather than by arrival.
    ///
    /// `E1-B06` and RFC 0049. False is every scenario that shipped before it,
    /// and false is the path this file had: an entry taken off the wire goes
    /// straight into the virtqueue, and one that does not fit is refused
    /// `DEVICE_FULL`. True is the real driver's shape — the entry waits in a
    /// queue on the *driver's* side of the virtqueue, and what leaves that
    /// queue next is what the deadline field says.
    ///
    /// The two are one field rather than two models because a model that
    /// ordered differently from the driver would be a model whose sweep
    /// explored a system nobody ships.
    pub ordered: bool,
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

/// What one entry is served as at a modelled driver.
///
/// One call, so that the model and `user/virtio-blk` cannot drift: both ask
/// `f_abi::deadline::inherit`, with this file's [`ADMITTED`], [`CLIENT_ADMITTED`]
/// and [`FLOOR_NS`] standing in for what the real frame writes into a routing
/// page.
///
/// # Errors
///
/// What `inherit` refuses, as `(packed, detail)`.
fn admit(entry: &Sqe, arrival: u64) -> Result<Inherited, (i32, u64)> {
    // `expect` is unreachable and would be a panic in a model: both constants
    // are class ordinals this file wrote, and `Admitted::new` refuses only a
    // value that is not one. Written as a fallback rather than an unwrap so
    // that a future edit to either constant is a wrong answer rather than a
    // crash in a fuzzer.
    let (Some(mine), Some(client)) = (Admitted::new(ADMITTED), Admitted::new(CLIENT_ADMITTED))
    else {
        return Err((error::pack(error::ARGUMENT, error::argument::BAD_CLASS), 0));
    };
    f_abi::deadline::inherit(
        &Caller::of(entry, client),
        Callee { admitted: mine, arrival, floor: FLOOR_NS },
    )
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
    /// Requests the driver half has taken off the wire and not yet put in the
    /// virtqueue, each with the moment it arrived.
    ///
    /// Empty unless [`Config::ordered`]. The arrival is kept rather than the
    /// `Inherited` it produces, because `f_abi::deadline::inherit` is pure: two
    /// fields on the wire plus this instant give the same answer every time,
    /// and a snapshot that carried the answer could carry one the rule no
    /// longer gives. Unit of the second: nanoseconds on the model's clock.
    pending: Vec<(Sqe, u64)>,
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
            pending: Vec::new(),
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
            // `E1-P02`'s allocation failure. The domain is told to refuse and
            // the registration then goes through the real table, so what comes
            // back is the refusal `Table::register` builds when a domain
            // declines — including the part worth asserting, that a refused
            // registration leaves no slot and no generation spent.
            if world.strike(me, Class::Alloc, entry.user_data).is_some() {
                self.service.starve();
            }
            let now = world.clock();
            let cqe = self.service.register(&entry, now);
            // Armed and disarmed around this one call. `Table::register` refuses
            // a malformed geometry or a full slot table without ever asking the
            // domain, and a starve left armed after one of those would refuse
            // the next operation's translation — a fault recorded against one
            // token and suffered by another. `Grants::starve` argues it.
            self.service.relent();
            world.record(me, P::NAME, wrote::REGISTER, entry.user_data, entry.len.into());
            self.answer(world, me, cqe);
            return;
        }

        if self.cfg.ordered {
            // The driver's own queue, on the driver's side of the virtqueue.
            // Everything above this line is the same for both shapes — a
            // registration is answered where it always was, because a
            // registration touches no queue and ordering one would be ordering
            // the thing that makes ordering possible.
            self.enqueue(world, me, entry);
            self.pump(world, me);
            return;
        }
        self.accept(world, me, &entry);
    }

    /// Take one entry into the driver's queue, or refuse it there.
    ///
    /// The admission is `f_abi::deadline::inherit`'s and this adds nothing to
    /// it: RFC 0025 decided the rule, `abi/src/deadline.rs` is the rule, and a
    /// model with a second opinion about it would be a model whose sweep
    /// explored a system nobody ships. `user/virtio-blk/src/pending.rs` makes
    /// the same call at the same moment for the same reason.
    fn enqueue(&mut self, world: &mut World, me: ActorId, entry: Sqe) {
        let token = entry.user_data;
        let now = world.clock();
        if let Err((packed, detail)) = admit(&entry, now) {
            // A peer claiming a class it does not hold, or a class field no
            // conforming service wrote. Refused rather than demoted, which is
            // the bound that makes urgency cost something to claim.
            world.record(me, P::NAME, wrote::DENIED, token, detail);
            self.deny(world, me, token, packed, detail);
            return;
        }
        if self.pending.len() >= PENDING_MAX {
            self.deny(
                world,
                me,
                token,
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                PENDING_MAX as u64,
            );
            return;
        }
        self.pending.push((entry, now));
    }

    /// Hand the virtqueue as much of the driver's queue as it has room for,
    /// most urgent first.
    ///
    /// **This is where the ordering happens, and it is the only place it can
    /// be.** A virtqueue is consumed in the order the driver posts, so a
    /// request that has already been offered cannot be overtaken by anything —
    /// which makes `Config::depth` the granularity of every reordering below,
    /// exactly as `f_virtio_blk::pending::IN_FLIGHT` is on the real driver.
    fn pump(&mut self, world: &mut World, me: ActorId) {
        while u32::from(self.queue.outstanding()) < self.cfg.depth {
            let Some(at) = self.most_urgent() else { return };
            let (entry, _) = self.pending.remove(at);
            self.accept(world, me, &entry);
        }
    }

    /// Which waiting request the device should be given next.
    ///
    /// Each rank is re-derived from the entry and the instant it arrived, and
    /// never from the clock now: `inherit` is pure, so this answers what it
    /// answered when the entry turned up. The tie-break is that arrival instant
    /// and then the position in the queue, which is first-come-first-served
    /// within a rank — `user/virtio-blk/src/pending.rs` argues at length why
    /// that is load-bearing rather than tidy.
    fn most_urgent(&self) -> Option<usize> {
        let mut best: Option<(usize, (u16, u64), u64)> = None;
        for (at, (entry, arrival)) in self.pending.iter().enumerate() {
            let Ok(order) = admit(entry, *arrival) else { continue };
            let rank = order.rank();
            let better = match best {
                None => true,
                Some((_, held, when)) => (rank, *arrival) < (held, when),
            };
            if better {
                best = Some((at, rank, *arrival));
            }
        }
        best.map(|(at, _, _)| at)
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

            // The token before the chain is served, so that an injected fault
            // is written into the trace against the operation it struck rather
            // than against a descriptor index. It is a second lookup of the
            // same job the harvest below finds, and it is worth it: `E1-P03`
            // reports the token, and a report naming a queue head would be a
            // report nobody could match to a request.
            //
            // `u64::MAX` when there is no job behind the chain, and not zero:
            // zero is client zero's first operation, so a fault attributed to it
            // would name a real request that had nothing to do with it. The same
            // spelling `Device::submit` uses for a doorbell with nothing behind
            // it, for the same reason — a missing value must not be a valid one
            // (R04).
            let token =
                self.jobs.iter().find(|job| job.head == chain.head).map_or(u64::MAX, |j| j.token);

            // Only the classes this protocol actually reads off the bus are
            // consulted here, and that is the check rather than tidiness: a
            // class armed against a device that ignores it would strike, be
            // written into the hashed artefact, and change nothing about the run
            // — an unexercised site reporting green, which is the row
            // `docs/test-taxonomy.md` calls *a fault-injection site that is
            // never exercised*. Gated, such a scenario never strikes at all, and
            // `fault::tests::every_armed_scenario_actually_strikes_and_writes_it_down`
            // turns it into a red suite. The classes the machinery injects — the
            // three below and around this loop — belong to every device and are
            // not listed.
            let injured = Injured {
                translation: P::HONOURS.contains(&Class::MapFault)
                    && world.strike(me, Class::MapFault, token).is_some(),
                last_write: P::HONOURS.contains(&Class::Partial)
                    && world.strike(me, Class::Partial, token).is_some(),
            };

            let mut bus = Bus::new(&mut self.control, self.service.grants()).injured(injured);
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

            // `E1-P02`'s device page-fault latency: the translation was there
            // and fetching it cost more than the transfer. Added to the service
            // time rather than replacing it, because the transfer still happens
            // — the class is a cost and not a failure, and the assertion is that
            // nothing but the clock moved.
            let elapsed = match world.strike(me, Class::FaultIn, token) {
                Some(Fault::Delay(nanos)) => elapsed.saturating_add(nanos),
                _ => elapsed,
            };
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

        // `E1-P02`'s peer death mid-operation: the device stops with this job
        // and everything behind it still outstanding. The same path a lost
        // completion takes — RFC 0024 leaves a client no other way to take a
        // buffer back — reached by a different cause, which is what makes it a
        // class rather than a second name for `lose_one_in`.
        if world.strike(me, Class::PeerGone, job.token).is_some() {
            self.fall_over(world, me);
            return;
        }

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
        // `E1-P02`'s delayed completion: the used entry is published and the
        // driver is told late. After the publish and not before the service, so
        // that this class and `Class::FaultIn` are distinguishable in a trace by
        // where they sit rather than only by their labels.
        let held = match world.strike(me, Class::LateCqe, job.token) {
            Some(Fault::Delay(nanos)) => nanos,
            _ => 0,
        };
        world.send(held, me, Message { from: me, kind: kind::REAP, token: job.token, detail: 0 });
    }

    /// The driver harvests the used ring and answers its client.
    ///
    /// Where an ordering driver looks at its queue again: a completion is what
    /// frees the slot the next request goes into, so it is the only other
    /// moment at which the choice `pump` makes can be made. Without it a
    /// request that arrived while the device was full would wait for the next
    /// doorbell rather than for the next completion — which is a driver that
    /// stalls when its client stops submitting.
    fn reap(&mut self, world: &mut World, me: ActorId) {
        if self.reset {
            return;
        }
        // How many entries this harvest has drained, and the token the first of
        // them answered. Read by [`crossed`] and by nothing else, and both are
        // zero-cost when that function is the identity.
        let mut drained: u32 = 0;
        let mut first: Option<u64> = None;
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
            // The deliberate defect's one call site. Off by default, in which
            // case this is the identity on `job.token` and the compiler emits
            // nothing for it; on, it answers a coalesced pair's second entry
            // with the first's token. See [`crossed`].
            let answered = crossed(drained, first, job.token);
            first.get_or_insert(job.token);
            drained = drained.saturating_add(1);
            let cqe = if result < 0 {
                refusal(answered, result, u64::from(written), now)
            } else {
                completion(answered, result, now)
            };
            self.answer(world, me, cqe);
            // A slot just came free, so the choice `pump` makes is available
            // again. Inside the loop rather than after it: a harvest that
            // drained two completions has two slots to fill, and filling them
            // one at a time is what a driver does.
            if self.cfg.ordered {
                self.pump(world, me);
            }
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
            // The second deliberate defect's one call site. Off by default, in
            // which case this is `true` and the compiler emits nothing for it;
            // on, the device resets and never says so. See [`tells_the_client`].
            if tells_the_client() {
                world.send(0, client, Message { from: me, kind: kind::GONE, token: 0, detail: 0 });
            }
        }
    }

    /// Give a control slot back.
    fn release_slot(&mut self, slot: u16) {
        self.slots &= !(1u64 << (slot & 63));
    }

    /// Write this device out, tag first.
    ///
    /// Every field, in the order they are declared, so that the two functions
    /// below read as one list beside the struct they are about. The one that is
    /// not a plain copy is [`Service`], which travels as the registrations that
    /// made it — `service.rs` argues why — and the `jobs` below are what puts
    /// its lent bitmap back, because a job *is* a buffer this device has
    /// resolved and not released.
    pub(crate) fn save(&self, out: &mut crate::snap::Writer) -> Result<(), crate::snap::Broken> {
        out.u32(P::TAG);
        self.proto.save_state(out);
        out.u32(self.cfg.depth);
        out.u64(self.cfg.service_ns);
        out.u64(self.cfg.spread_ns);
        out.u32(self.cfg.lose_one_in);
        out.u64(self.cfg.extent);
        out.u16(self.cfg.queue_size);
        out.u32(self.cfg.domain);
        out.bool(self.cfg.ordered);
        self.queue.save(out);
        self.control.save(out);
        self.service.save(out);
        out.bool(self.client.is_some());
        out.u32(self.client.map_or(0, |id| id.0));
        out.count(self.jobs.len());
        for job in &self.jobs {
            out.u64(job.token);
            out.u32(job.set.bits());
            out.u32(job.index);
            out.u16(job.head);
            out.u16(job.slot);
            out.u32(job.len);
            out.bool(job.served.is_some());
            let served = job.served.unwrap_or(Served {
                used_len: 0,
                label: crate::proto::wrote::SERVED,
                fenced: false,
            });
            out.u32(served.used_len);
            out.label(served.label);
            out.bool(served.fenced);
            out.bool(job.published);
            out.u64(job.seq);
        }
        out.count(self.pending.len());
        for (entry, arrival) in &self.pending {
            out.sqe(entry);
            out.u64(*arrival);
        }
        out.u64(self.slots);
        out.u64(self.arrivals);
        out.bool(self.reset);
        Ok(())
    }

    /// Read one back, on a protocol built the way a scenario builds one.
    pub(crate) fn load(mut proto: P, input: &mut crate::snap::Reader<'_>) -> Self {
        proto.load_state(input);
        let cfg = Config {
            depth: input.u32(),
            service_ns: input.u64(),
            spread_ns: input.u64(),
            lose_one_in: input.u32(),
            extent: input.u64(),
            queue_size: input.u16(),
            domain: input.u32(),
            ordered: input.bool(),
        };
        let queue = Queue::load(input);
        let control = Region::load(input);
        let mut service = Service::load(input);
        let known = input.bool();
        let who = input.u32();
        let client = known.then_some(ActorId(who));
        let count = input.count(44, "more jobs than the file could hold");
        let mut jobs = Vec::with_capacity(count);
        for _ in 0..count {
            let token = input.u64();
            let set = SetId::from_bits(input.u32());
            let index = input.u32();
            let head = input.u16();
            let slot = input.u16();
            let len = input.u32();
            let was_served = input.bool();
            let served =
                Served { used_len: input.u32(), label: input.label(), fenced: input.bool() };
            jobs.push(Job {
                token,
                set,
                index,
                head,
                slot,
                len,
                served: was_served.then_some(served),
                published: input.bool(),
                seq: input.u64(),
            });
        }
        let waiting = input.count(72, "more queued requests than the file could hold");
        let mut pending = Vec::with_capacity(waiting);
        for _ in 0..waiting {
            pending.push((input.sqe(), input.u64()));
        }
        let slots = input.u64();
        let arrivals = input.u64();
        let reset = input.bool();

        // The lent bitmap, put back through the real `Table::resolve` rather
        // than copied: a buffer this device could not resolve again is a file
        // that describes a device state the table cannot hold, and the refusal
        // is the point.
        if !input.faulted() {
            for job in &jobs {
                if service.relend(job.set, job.index, job.len).is_err() {
                    input.refuse(crate::snap::Broken::Diverged(
                        "a buffer the device held, which its table would not lend again",
                    ));
                    break;
                }
            }
        }

        Self { proto, queue, control, service, cfg, client, jobs, pending, slots, arrivals, reset }
    }
}

/// **A deliberate defect, off by default.** Which token a coalesced pair's
/// second completion is answered with.
///
/// # Why there is a defect in the shipped source at all
///
/// RFC 0017 argues it for the kernel and RFC 0040 extends the argument to here:
/// a sweep that has only ever printed *clean* is indistinguishable from a sweep
/// that cannot print anything else, and the only way to tell the two apart is to
/// break something on purpose and require the sweep to find it. A patch somebody
/// applies would be a patch somebody forgets to apply; a feature is a thing
/// `cargo xtask lint-mutations` can refuse to let become a default, and
/// `cargo xtask sweep --mutate` is the harness that turns it on, requires the
/// sweep to go red, turns it off, and requires the sweep to go green.
///
/// # Why *this* defect
///
/// Three properties, and it was chosen for all three rather than for being easy
/// to write.
///
/// **It is silent everywhere else.** `cargo xtask sim` stays green on it: every
/// scenario still reproduces byte for byte at its seed and still moves when the
/// seed moves, because the defect is deterministic like everything else here. So
/// is `cargo test`, `cargo xtask lint` and `cargo xtask verify`. That is the
/// shape of bug this whole apparatus exists for and it is the same shape
/// `mutate-unseeded-time` has one layer down — nothing *fails*, and the only
/// thing wrong is a property nobody was checking.
///
/// **It needs an ordering to show itself.** The device only harvests two entries
/// in one turn of the loop when it coalesced, and coalescing is a seeded
/// decision at `Protocol::COALESCE`. So a sweep of one seed is very likely to
/// miss it and a sweep of many is very likely to find it — which is the property
/// that makes *sweep* the right word rather than *run*.
///
/// **What it breaks is client-visible without any check written for it.** The
/// second completion carries a token the client has already had back, so the
/// client is told about a token it does not hold — `check::held` — and the
/// operation the entry really finished is never answered, so a buffer is left
/// out and its client never finishes. Two independent oracle properties catch
/// it, and neither was written with this defect in mind.
///
/// # The cost, stated
///
/// A reader of `Device::reap` sees two versions of one line, exactly as a reader
/// of `kernel/src/cap.rs` sees two versions of one lookup. That is the trade
/// RFC 0017 made once and this makes a second time, and the reason it is
/// acceptable is that the alternative — a mutation harness with nothing to
/// mutate — is a harness whose green result means nothing.
#[cfg(not(feature = "mutate-crossed-completion"))]
const fn crossed(_drained: u32, _first: Option<u64>, token: u64) -> u64 {
    token
}

/// The defect itself: from the third entry of one harvest onward, answer with
/// the token the first entry answered.
///
/// The third and not the second, and the number is the whole reason this defect
/// is worth a sweep rather than a run. Draining three entries in one turn takes
/// two consecutive coalescing decisions with work still behind them, which is
/// two seeded choices at `Protocol::COALESCE` and a queue deep enough to hold
/// the work — so most seeds never reach it and the ones that do are not the
/// same seeds in two scenarios. A defect that fired on the first completion
/// would be found by `cargo xtask sim` on its default seed, and a mutation
/// harness that only demonstrated *the checks can fail* would say nothing about
/// whether sweeping is worth its overnight.
#[cfg(feature = "mutate-crossed-completion")]
const fn crossed(drained: u32, first: Option<u64>, token: u64) -> u64 {
    match first {
        Some(head) if drained >= 2 => head,
        _ => token,
    }
}

/// **A deliberate defect, off by default.** Whether a device that has reset
/// tells its client.
///
/// # Why a second one, when there is already `crossed`
///
/// Because five properties with one defect between them is a table with one
/// property under test and four decorations, and the review that found it said
/// so in those terms. `crossed` trips `check::held` — the first check in the
/// list, so it is the only signature it ever produces — and a harness built on
/// it alone can say nothing about whether the other four can fail on a run of
/// the models rather than on a hand-built `Record` vector. This one trips
/// `check::balance`, and `check::bound` in the runs where nothing was in flight
/// when the device fell over, which is two more properties shown to work end to
/// end. RFC 0042 is the record and RFC 0017 is still the argument for why it
/// lives here rather than in a patch.
///
/// # What it is
///
/// `fall_over` exists because RFC 0024 leaves a client exactly one way to take a
/// buffer back without a completion — `PeerGone`, built from evidence that the
/// peer's outstanding tokens are void — and `kind::GONE` is that evidence
/// arriving. This defect withholds it. The device is reset, the registrations
/// are retired and the translations are gone, and the client is never told: it
/// waits for completions that the device has already decided will never come,
/// the timeline runs out of events, and the run ends with buffers lent and
/// operations unanswered.
///
/// `client.rs`'s own module documentation calls this out as the protocol hazard
/// rather than a modelling one — *a device that loses a completion and stays
/// alive leaves its client holding memory it may never touch again* — which is
/// why it is the right second defect: it is a bug somebody could write.
///
/// **It is silent everywhere the oracle is not.** The trace still reproduces at
/// its seed and still moves when the seed moves, so `cargo xtask sim` is green
/// on it. Nothing panics and nothing runs out of budget: a hang here looks like
/// a short trace, which is exactly why `bound` and `balance` exist as checks
/// rather than as a stopwatch.
///
/// **It needs a fall-over to show itself**, so it is found in the scenarios that
/// arm `dropcqe` or `peergone` and in the ones whose device loses a completion
/// by its own seeded choice. That makes it a cheaper defect than `crossed` — it
/// does not need two consecutive coalescing decisions — and cheaper is the
/// point: what it is here to demonstrate is a *different check firing*, not a
/// second argument for sweeping.
#[cfg(not(feature = "mutate-silent-reset"))]
const fn tells_the_client() -> bool {
    true
}

/// The defect itself: reset, and do not tell the client.
#[cfg(feature = "mutate-silent-reset")]
const fn tells_the_client() -> bool {
    false
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

    fn save(&self, out: &mut crate::snap::Writer) -> Result<(), crate::snap::Broken> {
        Self::save(self, out)
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
