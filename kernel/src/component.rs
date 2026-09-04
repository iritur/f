// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Components, places and the lifecycle: spawn from a manifest, connect to a
//! place, stop an occupant, tear it down, and put a new one in.
//!
//! # What a place is, and why it is not an instance
//!
//! A **place** is one manifest's slot under one supervisor: a content hash, an
//! `Endpoint` capability, and at most one occupant at a time. An **instance** is
//! whichever component currently occupies it. Clients hold the endpoint, which
//! survives every occupant, so a client that lost its peer reconnects through
//! the handle it already has rather than needing a fresh grant from a supervisor
//! that would otherwise have to know every client.
//!
//! That distinction is the whole of gate G1's mechanism — *a driver is killed
//! under sustained load and the system does not notice* — and RFC 0008 argues
//! it against the simpler alternative at length. What this module adds is the
//! part a document cannot: a connect to an *empty* place does not fail. It
//! pends, and has exactly three outcomes, because two of them and a silence is
//! how the builder and its test each invent the third:
//!
//! | outcome | when |
//! | --- | --- |
//! | a channel | a spawn refills the place |
//! | `PEER/GONE` | the place is retired — its restart budget ran out |
//! | `PEER/EMPTY` | the connect's own deadline passed with the place still empty |
//!
//! `PEER/EMPTY` is deliberately not `GONE`: the place may yet be refilled, so a
//! client that can wait longer may submit again.
//!
//! # What runs here and what RFC 0030 says is deferred
//!
//! RFC 0008 is explicit that restart is the **supervisor's** act and that the
//! frame provides only the mechanism. The policy in this file is the frame
//! holding that ground until a supervisor can stand on it, and **the reason it
//! is still holding is not the reason it used to be.**
//!
//! What used to be written here is that a component cannot drive a control ring,
//! because adopting a mapped channel is `unsafe` and a `user/` crate may not
//! write one. RFC 0037 ended that, and RFC 0047 showed it working in both
//! directions on a real component: `user/virtio-blk` adopts its control ring in
//! safe code, submits operations on it, and is answered by the frame from a
//! polling loop.
//!
//! Three things are missing and they are all one thing — **there is no
//! supervisor component**:
//!
//! - Nothing implements `f_abi::control::op::SPAWN` or `op::STOP`. The opcodes
//!   are named and the ring that would carry them works; what would submit them
//!   does not exist.
//! - A supervisor would have to be *told* its occupant died. The frame already
//!   posts `notice::PEER_GONE` with a cause — [`tear_down`] does it on every
//!   teardown — so the notice is there and the reader is not.
//! - A supervisor is a component, so it needs a place, an account and a
//!   manifest, and the `Untyped` behind that account is what would replace
//!   [`PLACES_MAX`] and [`SUPERVISOR_ORDER`]. RFC 0044 names all three as the
//!   same deviation.
//!
//! So [`policy`] is written as one function over a record and a tally, taking no
//! kernel state at all, and moving it above the frame is a move rather than a
//! rewrite. `cargo xtask lint-owed` is what keeps that honest: it holds
//! `policy::decide(` in this file as a declared quantity and goes red the day
//! the call goes, which is the day this paragraph and RFC 0008's status both
//! have to change.
//!
//! # What a spawn checks, and what it does not
//!
//! RFC 0008 gives the spawn entry its refusals and this file implements them:
//! a handle of the declared type, carrying at least the declared rights and
//! `GRANT` beside them, for every need the manifest declares at spawn. A need
//! not supplied refuses, a handle for a need the manifest does not declare
//! refuses, and a handle naming less than the need's stated quantity refuses.
//! Fail closed, R04, and every one of them is provoked on purpose at boot so
//! that a refusal nobody has watched is not mistaken for one that cannot
//! happen. [`check_needs`] is the whole of it, and it runs before the first
//! frame is charged.
//!
//! Two further gaps are stated here rather than discovered:
//!
//! - **A spawn grants into the child's table; it does not derive across
//!   tables.** RFC 0008 wants the child's capability to be a descendant of the
//!   supervisor's, so that revoking the supervisor's `Untyped` reaches it. The
//!   cross-table parent link is E1-B13's and RFC 0029 says in its own affects
//!   line that it deliberately did not land. Until it does, what ends a
//!   component is the frame ending it — [`tear_down`] — and not the
//!   revocation walk reaching across. *Reversal:* a cross-table parent link in
//!   `cap.rs`, at which point the grants below become derives and this
//!   paragraph goes.
//! - **The address space's page tables are not charged to the account.** Text,
//!   stack and the control ring are retyped from the supplied `Untyped`, which
//!   is the account RFC 0008 names; the page tables under them come from
//!   `paging::user_space`, which allocates. They are still returned exactly —
//!   `UserSpace::tables()` is the list, the same one `process::reap` uses — so
//!   nothing leaks; what is not yet true is that a supervisor's quota bounds
//!   them. *Reversal:* `paging::user_space` taking frames rather than an
//!   allocator, which is a change to that module and not to this one.

use f_abi::cap::{CapType, Handle, rights};
use f_abi::control::{cause, notice};
use f_abi::manifest::{ContentId, Need, Record, Refusal, restart, route};
use f_abi::reserve::{Demand, Grant, Table as Reservations};
use f_abi::{Cqe, error};
use f_ring::{Collector, Mapping, Poster, RingError};

use crate::arch::x86_64::multiboot::BootInfo;
use crate::arch::x86_64::paging::{self, Features, UserPage, UserSpace};
use crate::cap::{Direct, Table};
use crate::mem::{FRAME_SIZE, Frame, FrameAllocator, Order};

/// Entries in a control ring.
///
/// Sixteen, the same as the frame's own channel and for the same reason: the
/// whole region has to fit in one frame, which the account pays for a page at a
/// time. A component that needs a deeper control ring is a component owed more
/// notices at once than it holds slots, which RFC 0008 says cannot happen.
const CONTROL_ENTRIES: u32 = 16;

/// How much untyped memory the frame stakes the *supervisor's own* account
/// with.
///
/// Order two: four frames. It pays for one thing, and that thing is why it is a
/// separate account rather than a corner of a place's — the pages the
/// supervisor's own capability table grows by. [`Table::grow`] charges the
/// lowest-indexed `Untyped` in the table that carries `DERIVE`, so this is
/// granted before any place's account, and a supervisor whose table growth was
/// charged to a place would be one whose quota moved when a *sibling* was
/// spawned. Four frames is [`crate::cap::MAX_PAGES`], which is the most a table
/// in this build can ever buy.
const SUPERVISOR_ORDER: u8 = 2;

/// The account a place is staked with, as an allocator order.
///
/// Sized from the manifest's own `memory_bytes`, and that is the change that let
/// this frame hold more than one place. What it replaced was a constant hundred
/// and twenty-eight kibibytes; `user/virtio-blk/manifest.toml` declares two
/// mebibytes; the two met in [`admit`] as `ADMISSION/MEMORY`, which is a
/// component refused for the size of a number in the frame rather than for
/// anything about itself. E1-B02's own note records that refusal, and this
/// function is where it stops happening.
///
/// The next power of two at or above [`account_bytes`], because the allocator is
/// a buddy allocator and an account is one block. Rounding *up* is the only safe
/// direction: an account holding less than what was declared is a spawn that
/// fails partway through, which is the state [`admit`] exists to refuse before
/// anything is spent. The slack that leaves is printed on the admission line
/// beside what was declared rather than hidden in here — a figure only this
/// function knew would be a quota nobody could audit.
fn account_order(record: &Record) -> Option<Order> {
    let pages = account_bytes(record).max(FRAME_SIZE).div_ceil(FRAME_SIZE).next_power_of_two();
    Order::new(u8::try_from(pages.trailing_zeros()).ok()?)
}

/// What one place's account has to hold: the component's declared footprint,
/// **plus** what its declared needs are carved out of.
///
/// The sum is here rather than assumed, and getting it wrong is what an
/// exactly-sized account made visible. `memory_bytes` is the footprint and
/// nothing else — `user/virtio-blk/manifest.toml` says so in as many words: *the
/// memory is the account its whole footprint is retyped from — address space,
/// page tables, text, stack, control ring, state tree, capability table — and
/// not the queues above, which are a routed need.* In this build there is
/// nowhere else for a routed need to come from, because the supervisor is the
/// frame and the frame stakes one region per place, so the needs are carved out
/// of the same account and the account has to be that much larger.
///
/// That is what makes the *second* [`admit`], the one inside [`spawn`], a check
/// rather than a formality. It runs after the offer has carved the needs out,
/// and what it asks is whether the account still holds the footprint the
/// manifest declared. Under the constant this replaced the answer was yes for
/// the same reason it is yes for a stopped clock: there was a hundred and
/// twenty-eight kibibytes of slack and nobody was comparing anything.
///
/// *Reversal:* a topology in which a need is routed from somewhere other than
/// the spawning supervisor's own account — a sibling's endpoint, a device
/// region the frame owns — at which point that need stops being part of this
/// sum and the manifest's `from` field is what says so.
/// Unit: bytes.
fn account_bytes(record: &Record) -> u64 {
    let mut bytes = record.memory_bytes;
    for need in record.needs() {
        if need.route == route::POWERBOX {
            continue;
        }
        bytes = bytes.saturating_add(least_extent(need));
    }
    bytes
}

/// How many places this build's supervisor can hold.
///
/// Four, which is what this tree's two component files fit in with room for two
/// more. When a supervisor is a component, this bound is its `Untyped` rather
/// than a constant, which is the direction everything else in this tree has
/// already gone.
///
/// It is also the bound on [`modules`], and the two being one number is the
/// point: the loader hands over component files, and every one of them gets a
/// place. A build that carried more component files than places would be a
/// build where the boot half of RFC 0035's pair silently covered less than the
/// workload half, which is exactly the drift `JOIN_GAP` was built to make
/// visible.
const PLACES_MAX: usize = 4;

/// Why the lifecycle could not do what it was asked.
///
/// Every variant is a bug in the frame or a manifest the loader handed over, and
/// every one of them fails the boot: there is no second supervisor to fall back
/// to and a component half-built is worse than no component.
#[derive(Clone, Copy, Debug)]
pub enum Failure {
    /// No boot module carried a component file.
    NoComponent,
    /// A component file was refused. Carries the refusal, which names which
    /// field of the record was disbelieved.
    Manifest(f_abi::manifest::Refusal),
    /// The frame allocator had nothing left to stake an account with.
    NoMemory,
    /// The address space could not be built or a page could not be mapped.
    Space(paging::BuildError),
    /// The account could not pay for something the manifest declared.
    Account,
    /// A capability operation the frame made of its own tables was refused,
    /// which means the frame asked for something it was not entitled to.
    Capability(i32),
    /// A control ring's header was refused by the code that wrote it.
    Ring(i32),
    /// The image does not fit the one page a component's text is mapped in.
    ImageTooLarge,
    /// A place was asked to take an occupant it already had, or to take one
    /// from a manifest that is not the one that created it.
    WrongPlace,
    /// A pending connect completed in a way the demonstration did not expect.
    Connect(i32),
    /// The spawn was refused before anything was spent: the account holds less
    /// than the manifest declares, or the reservation is one this build cannot
    /// promise. Carries the packed refusal, which is in the `ADMISSION` domain
    /// and not `RESOURCE` — see [`admit`].
    Admission(i32),
    /// A spawn's supplied handles did not satisfy the manifest's needs: one
    /// missing, one too many, one of the wrong type, one carrying less than the
    /// declared rights, or one naming less than the declared quantity. Carries
    /// the packed refusal, which names which of the five it was. R04, and
    /// [`check_needs`] is where each is decided.
    Need(i32),
    /// A notice could not be published onto a control ring, or what came back
    /// off one was not the notice that went on. Carries the packed refusal.
    Notice(i32),
    /// The frame's own count of what it built and what it gave back disagreed.
    Leaked,
}

impl Failure {
    /// A line for the boot log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::NoComponent => "no boot module carried a component file",
            Self::Manifest(_) => "a component file was refused",
            Self::NoMemory => "nothing left to stake a component's account with",
            Self::Space(_) => "a component's address space could not be built",
            Self::Account => "the account could not pay for what the manifest declared",
            Self::Capability(_) => "the frame was refused a capability operation of its own",
            Self::Ring(_) => "a control ring refused the header written into it",
            Self::ImageTooLarge => "the image does not fit the page its text is mapped in",
            Self::WrongPlace => "a spawn named a place it may not occupy",
            Self::Connect(_) => "a pending connect completed unexpectedly",
            Self::Admission(_) => "the spawn was refused admission before anything was spent",
            Self::Need(_) => "a spawn's supplied handles did not satisfy the manifest's needs",
            Self::Notice(_) => "a notice could not be published on a control ring",
            Self::Leaked => "a component's frames did not all come back",
        }
    }
}

/// Every boot module that is a component file, in the order the loader placed
/// them.
///
/// By magic and not by position, which is what makes adding a component a change
/// to a module list and not to the kernel: a module whose first eight bytes are
/// not [`f_abi::manifest::MAGIC`] — `user/init`'s flat image, a firmware blob —
/// is skipped rather than interpreted. RFC 0030 argues it, and the fail-closed
/// direction is the one that produces a smaller topology rather than a component
/// built out of the wrong bytes.
///
/// # Safety
///
/// The direct map must be live and `frames` must already have been rebound onto
/// it, which is [`crate::arch::x86_64::multiboot::Module::bytes`]'s obligation
/// and is the same one `main::component` discharges for module one.
#[must_use]
pub unsafe fn modules(boot: &BootInfo) -> ([&'static [u8]; PLACES_MAX], usize) {
    let mut found: [&'static [u8]; PLACES_MAX] = [&[]; PLACES_MAX];
    let mut count = 0;
    for module in boot.modules() {
        if count == PLACES_MAX {
            break;
        }
        // SAFETY: the caller's guarantee, and every module is in the reserved
        // list — see `main::reserved_ranges` — so nothing else owns these bytes.
        let bytes = unsafe { module.bytes() };
        if Record::read(bytes).is_ok()
            && let Some(slot) = found.get_mut(count)
        {
            *slot = bytes;
            count += 1;
        }
    }
    (found, count)
}

/// What a supervisor decides, as a function of a manifest and a tally.
///
/// The whole of RFC 0008's restart rule, in one place, taking no kernel state:
/// this is the code that moves above the frame when E1-B08 lands a safe channel
/// adoption, and it is written this way so that the move is a move.
pub mod policy {
    use f_abi::manifest::Record;

    /// What the supervisor does next about a place whose occupant has ended.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Verdict {
        /// Leave the place empty. The policy says so, or the death was a stop —
        /// which is the supervisor's own decision, and restarting after one
        /// would be the supervisor arguing with itself.
        Leave,
        /// Spawn again, after this many timer ticks.
        /// Unit of the payload: timer ticks, at the frame's own tick rate.
        Restart(u32),
        /// Retire the place: the budget ran out. Its endpoint is revoked in
        /// every holder's table and pending connects complete `PEER/GONE`.
        Retire,
    }

    /// How many restarts have happened inside the current budget window, and
    /// when the window opened.
    ///
    /// The window is read from `Env` rather than from a clock, which is RFC
    /// 0008's sentence and RFC 0004's substrate keeping a call site it would
    /// otherwise have lost: under the simulator a restart storm is a seeded
    /// scenario and not a wall-clock accident.
    #[derive(Clone, Copy, Debug, Default)]
    pub struct Budget {
        /// Restarts inside the window. Unit: restarts.
        pub used: u32,
        /// When the window opened. Unit: timer ticks, at the frame's own rate,
        /// as `Env` reports them to the supervisor.
        pub opened: u64,
    }

    /// Decide.
    ///
    /// `faulted` and `exited` are how the occupant ended; `now` is the
    /// supervisor's own tick count, read through `Env`. A window that has
    /// elapsed resets the count *before* the decision, which is the order that
    /// matters: a component that fails once a day forever is restarted forever,
    /// and `docs/manifest.md` says at length that a lifetime cap beside the
    /// window is schema 2's and needs a workload — E1-P06 — to justify it.
    #[must_use]
    pub fn decide(
        record: &Record,
        budget: &mut Budget,
        faulted: bool,
        exited: bool,
        now: u64,
    ) -> Verdict {
        if !record.restarts_after(faulted, exited) {
            return Verdict::Leave;
        }
        if now.saturating_sub(budget.opened) >= u64::from(record.budget_window_ticks) {
            budget.used = 0;
            budget.opened = now;
        }
        if budget.used >= record.max_restarts {
            return Verdict::Retire;
        }
        let pause = record.backoff_ticks(budget.used);
        budget.used += 1;
        Verdict::Restart(pause)
    }
}

/// An account: an untyped region a component's parts are retyped from.
///
/// It is a capability in the supervisor's table and not a number here, which is
/// the point — every frame an instance is made of advances that capability's
/// watermark, so a supervisor that runs out of account cannot spawn, and the
/// refusal is `RESOURCE/QUOTA_EXHAUSTED` rather than the frame serving out of
/// something it kept back.
struct Account {
    /// The untyped capability, in the supervisor's table.
    handle: Handle,
    /// The physical address the region started at, so a refund cannot take it
    /// below where it began.
    floor: u64,
}

/// One occupant of a place: everything it is made of, and everything owed back.
struct Instance {
    /// Which occupant of the place this is. Unit: instances, counting from
    /// zero, and it is what a channel to this instance carries in its header's
    /// `epoch` field.
    epoch: u32,
    /// Its address space. Never activated on any core in this build, which is
    /// why its teardown has no shootdown in it — see [`tear_down`].
    space: UserSpace,
    /// Every capability the account was charged for on this instance's behalf,
    /// in the order it was charged, held in the *supervisor's* table.
    ///
    /// The order matters and is the whole reason this is a list rather than a
    /// count: a refund can only take back the top of a watermark, so a teardown
    /// gives them back last-charged-first. See [`Table::refund`].
    charged: [Handle; CHARGED_MAX],
    /// How many of `charged` are real. Unit: capabilities.
    charges: usize,
    /// The names the supervisor minted to make the offer with: one per need
    /// supplied at spawn, held in the *supervisor's* table.
    ///
    /// Given up at teardown and **not** refunded. The memory behind them is the
    /// same memory `charged` gives back, and refunding it twice is the bug this
    /// split exists to not have.
    supplied: [Handle; f_abi::manifest::CAPABILITIES_MAX],
    /// How many of `supplied` are real. Unit: capabilities.
    supplies: usize,
    /// Where its control ring lives, as a kernel address.
    control: u64,
    /// The frame's end of that ring, which is the only producer on it.
    ring: Mapping,
    /// Its capability table.
    table: Table,
}

/// What every instance is made of besides its text: a stack and a control ring.
///
/// It was three, with text as the third, and text stopped being one frame the
/// day a component's image stopped fitting in one page — RFC 0047. So the fixed
/// part is two and the variable part is [`text_pages`], and the two are added
/// by [`parts`] rather than by each caller, because the number appears in three
/// places: what [`charges_for`] predicts, what [`admit`] refuses on, and what
/// [`spawn`] actually takes. Three copies of one sum is how a manifest gets
/// admitted for a footprint it then overruns.
const FIXED_PARTS: usize = 2;

/// How many frames one instance of an image this long is made of.
/// Unit: frames.
const fn parts(text_pages: usize) -> usize {
    text_pages + FIXED_PARTS
}

/// How many pages of text an image occupies.
///
/// Rounded up, and refused above the frame's own reservation rather than
/// silently truncated — a component whose text was mapped short would jump into
/// its own guard page partway through its first function, which is a fault with
/// no explanation in it.
///
/// # Errors
///
/// [`Failure::ImageTooLarge`].
fn text_pages(image: &[u8]) -> Result<usize, Failure> {
    let pages = (image.len() as u64).div_ceil(FRAME_SIZE) as usize;
    if pages == 0 || pages > crate::process::TEXT_PAGES {
        return Err(Failure::ImageTooLarge);
    }
    Ok(pages)
}

/// The most frames one instance can charge to an account.
///
/// Sixty-four. It was *as many frames as the account holds*, which said
/// something true while every account was one size and stopped saying anything
/// the moment accounts were sized from manifests: the largest account in this
/// tree is five hundred and twelve frames and the largest *instance* charges
/// twenty-six of them, so the old form would have made every [`Supply`] and
/// every [`Instance`] twenty times the size of what they hold — on the stack of
/// a boot processor, once per place.
///
/// Twenty-six and not twenty-three, and the three are worth naming because they
/// are the only part of this that moves with a commit: RFC 0047 let a
/// component's text be more than one page, and `user/virtio-blk`'s image is
/// four. The worst case is [`crate::process::TEXT_PAGES`] plus [`FIXED_PARTS`]
/// plus whatever the manifest's needs carve out, which for this tree is
/// thirty-eight — still well inside sixty-four, and the refusal below is what
/// happens if a manifest ever pushes past it.
///
/// A fixed bound owes a refusal with a domain rather than an array bound with
/// none, which is what [`charges_for`] and [`admit`] are: a manifest whose
/// declared parts and needs would not fit is refused `ADMISSION/MEMORY` before
/// anything is spent, and the refusal names the same domain as an account too
/// small — because it is the same statement, made about the frame's list rather
/// than about the supervisor's memory.
///
/// *Reversal:* a manifest that legitimately declares more. At that point the
/// list stops being an array on a stack and the account it gives back to stops
/// being a watermark, because a refund can only take back the top of one.
const CHARGED_MAX: usize = 64;

/// What the supervisor adds to a need's declared rights when it makes the
/// offer.
///
/// `GRANT` because handing the capability on is exactly what the supervisor is
/// doing, and RFC 0008 has the frame *check* for it rather than assume it.
/// `REVOKE` because a name the supervisor cannot give up again is a name that
/// outlives the instance it was minted for — which is the one thing a restart
/// may not leave behind, and is the same right [`charge`] asks for and for the
/// same reason. Neither reaches the child: the child is granted the manifest's
/// declared rights and nothing beside them.
const OFFERED: u8 = rights::GRANT | rights::REVOKE;

/// What a supervisor offers a spawn: one handle per need, and the frames its
/// own account's watermark moved for while making them.
///
/// A structure rather than two arguments because the two halves have to travel
/// together: a supply the frame refuses is a supply the supervisor has to
/// refund, and a refusal that handed back only the handles would leave the
/// account short by however many frames the offer cost.
struct Supply {
    /// One handle per need supplied at spawn, in the manifest's order.
    /// [`Handle::NULL`] is *not supplied*, which is legal only for a need the
    /// manifest marks optional.
    handles: [Handle; f_abi::manifest::CAPABILITIES_MAX],
    /// How many of `handles` the supervisor claims to have supplied.
    /// Unit: capabilities. A count past what the manifest declares refuses the
    /// spawn, which is the second of the refusals R04 asks for here.
    count: usize,
    /// Every frame the account was charged for while building the offer, in
    /// charge order.
    charged: [Handle; CHARGED_MAX],
    /// How many of `charged` are real. Unit: capabilities.
    charges: usize,
}

impl Supply {
    /// An offer of nothing, which is what a spawn against a manifest that
    /// declares needs is refused for.
    const EMPTY: Self = Self {
        handles: [Handle::NULL; f_abi::manifest::CAPABILITIES_MAX],
        count: 0,
        charged: [Handle::NULL; CHARGED_MAX],
        charges: 0,
    };
}

/// A place in the topology.
struct Place {
    /// What may occupy it. A spawn naming a different manifest is refused: a
    /// different manifest is a different place, and E2-D04's state-transfer
    /// protocol is where a newer one may lawfully take over an older one's.
    manifest: ContentId,
    /// The component file the manifest was read out of.
    module: &'static [u8],
    /// The endpoint, in the supervisor's table.
    endpoint: Handle,
    /// The ordinal the next occupant opens at.
    epoch: u32,
    /// The occupant, if there is one.
    occupant: Option<Instance>,
    /// Whether the budget ran out.
    retired: bool,
    /// The reservation this **place** holds, once its first occupant has been
    /// admitted.
    ///
    /// On the place and not on the occupant, because that is the lifetime RFC
    /// 0007 and RFC 0041 give it between them: a reservation's pre-faulted
    /// pages are never reclaimed for its life, and a place survives its
    /// occupant, so a restart into this place keeps these cores and these pages
    /// rather than asking for them again. What ends it is the place being
    /// destroyed, which nothing in a boot does.
    ///
    /// It is what [`admit`] checks a demand against before spending the
    /// machine: without it a restart re-tests its own place's demand against a
    /// table that has already been charged for it, and a hard-class place on a
    /// part that can grant one would be refused `ADMISSION/NO_CORE` against its
    /// own cores on the first fault. `f_abi::reserve::Grant::answers` is the
    /// predicate, and it compares what the record asks for rather than trusting
    /// the field to belong to it.
    reservation: Option<Grant>,
    /// The restart budget.
    budget: policy::Budget,
    /// Deaths by cause, for the boot log and for the state tree E1-P06 reads
    /// its blast-radius number out of.
    faults: u32,
    /// Deaths by exit.
    exits: u32,
    /// Deaths by a stop whose deadline passed.
    stops: u32,
    /// Restarts performed.
    restarts: u32,
}

/// A connect that has not been answered yet.
///
/// A place a client is waiting on, and the deadline it is prepared to wait to.
/// One per client in this build, because the demonstration has one client; a
/// supervisor holds as many as its own account pays for.
struct PendingConnect {
    /// Which place.
    place: usize,
    /// The endpoint handle the client presented, which is what a completion
    /// carries back.
    endpoint: Handle,
    /// How long it will wait. Unit: timer ticks, at the frame's own rate,
    /// because a deadline the boot log carries has to be a count rather than a
    /// duration — `process.rs` argues that at length and this is the same
    /// argument.
    deadline: u64,
}

/// What the demonstration observed, for the boot log to report and for the
/// caller to assert on.
#[derive(Clone, Copy, Debug, Default)]
pub struct Report {
    /// Component files the loader carried. Unit: modules.
    pub components: usize,
    /// Places the supervisor holds. Unit: places.
    pub places: usize,
    /// Instances spawned, across every place. Unit: instances.
    pub spawns: u32,
    /// Needs satisfied with a capability of the declared type that names no
    /// object this machine has. Unit: capabilities.
    ///
    /// **The gap this change leaves, as a number rather than a paragraph.**
    /// Today it is the `irq` need in `user/virtio-blk/manifest.toml`: nothing in
    /// this build routes a device interrupt to a component, so what the spawn
    /// supplies is a capability of the right type, carrying the right rights,
    /// naming no vector. It is enough for the lifecycle — the spawn is real,
    /// the account is real, the table is real — and it is not enough for the
    /// component to wait on the device. E1-B09 is what makes it zero.
    /// [`unbound_needs`] is the arithmetic and [`offer`] is the reason.
    pub unbound: u32,
    /// Deaths by fault. Unit: deaths.
    pub faults: u32,
    /// Restarts performed. Unit: restarts.
    pub restarts: u32,
    /// Places whose budget ran out. Unit: places.
    pub retired: u32,
    /// Connects that pended and were later answered with a channel.
    /// Unit: connects.
    pub resumed: u32,
    /// Connects answered `PEER/GONE` or `PEER/EMPTY` for a *client*. **The
    /// number gate G1's sentence is about**: a client that observed anything
    /// except added latency. Unit: connects.
    ///
    /// Zero on this boot by design and not by construction: the branch that
    /// increments it is the same branch [`Report::probed`] increments, and the
    /// frame drives that branch on purpose against a retired place on every
    /// run. What separates the two counters is one flag saying who submitted,
    /// so a zero here is a claim about the client rather than about the code
    /// being unreachable.
    pub lost: u32,
    /// Connects the frame submitted on purpose to take an outcome the client
    /// under test must not take: a deadline that passed, and a retired place.
    /// Counted apart from [`Report::lost`] because a probe that was refused is
    /// the mechanism working. Unit: connects.
    pub probed: u32,
    /// Notices published, in the order `f_abi::control::ORDER` fixes.
    /// Unit: notices.
    pub notices: u32,
    /// Which notice kinds were delivered, as a bit per
    /// `f_abi::control::notice` value.
    ///
    /// Counted rather than asserted, because *the seven kinds exist* and *six
    /// of them run on this boot* are two different claims and only the second
    /// is worth a line in a log. The seventh is *reclaim*, which is per core in
    /// a component's allocation, and nothing here holds an allocation because
    /// nothing here is scheduled.
    /// Unit: none — a bitmask, one bit per kind, counted from bit zero.
    pub kinds: u8,
    /// Notices read back off a control ring at a polling point, as flagged
    /// completion entries. Equal to [`Report::notices`] on a boot that
    /// finished, and counted apart because *published* and *delivered* are the
    /// two halves R05 is about. Unit: notices.
    pub collected: u32,
    /// How many post-then-drain rounds the notices took.
    ///
    /// More than one is the property worth seeing: a control ring is sixteen
    /// entries and a table owes more notices than that, so the ring's depth
    /// bounds how much is *visible* and never how much is *true*. Unit: rounds.
    pub rounds: u32,
    /// Notices still owed when the demonstration finished. Never non-zero — a
    /// non-zero here fails the boot — and carried so the log says the drain ran
    /// rather than implying it. Unit: notices.
    pub owed: u32,
    /// The epoch the resumed client's channel opened at. Unit: instances,
    /// counting from zero; one after a single restart, which is the whole of
    /// what a reconnecting client can see of a peer it did not have before.
    pub epoch: u32,
}

/// Build a place per component file the loader carried, put a component in each,
/// and then script the whole lifecycle against the first: connect a client, kill
/// it, and put a new one in — with the client's connect pending across the gap
/// and resuming at the higher epoch.
///
/// # A place per file, which is what this used to get wrong
///
/// It built one place, from `*modules.first()`, and RFC 0036 made the difference
/// between that and the component set the simulator drives a *declared* quantity
/// — `JOIN_GAP` in `xtask` — rather than a silence. The frame's half of closing
/// it is here, and it was two numbers rather than an argument: the account was a
/// constant hundred and twenty-eight kibibytes, and `virtio-blk` declares two
/// mebibytes, so a second place could not have been admitted even if one had
/// been built. [`account_order`] is the first, [`CHARGED_MAX`] the second.
///
/// Every place is filled and every place is torn down. **Only the first is
/// scripted**, and that is a claim about what a demonstration is worth rather
/// than about the places: killing a second occupant and watching the same three
/// branches would run the same code against the same mechanism and prove
/// nothing the first did not. What the others establish is the thing `JOIN_GAP`
/// is about — that this boot instantiated them at all, from their own records,
/// against accounts their own manifests sized.
///
/// # Why this is a demonstration and not a test harness
///
/// Because every step of it is the mechanism E1-P06 will drive, running against
/// real memory on the boot core: real records read out of real modules, real
/// address spaces, real capability tables paying a real account, a real channel
/// whose header carries a real epoch. What it is not is a *load* — nothing here
/// is scheduled, because there is no scheduler until E1-B08 — and that is the
/// one sentence separating this from gate G1.
///
/// The log it prints is a fixture: every number in it is a count rather than a
/// duration, so two runs of one commit produce the same bytes on machines two
/// orders of magnitude apart in speed. That is why the backoff below is stated
/// in ticks and never in milliseconds, and why nothing here reads a clock.
///
/// # Errors
///
/// A [`Failure`] naming which step did not hold.
///
/// # Safety
///
/// Call on the boot processor, once, with the kernel's address space in `CR3`,
/// `frames` rebound onto its direct map, and the direct map covering every boot
/// module. No process may be running: this builds address spaces and capability
/// tables of its own.
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    boot: &BootInfo,
    now: u64,
) -> Result<Report, Failure> {
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let (modules, count) = unsafe { modules(boot) };
    let module = *modules.first().filter(|_| count > 0).ok_or(Failure::NoComponent)?;
    let record = Record::read(module).map_err(Failure::Manifest)?;

    let before = frames.free_count();
    let mut report = Report { components: count, places: count, ..Report::default() };
    let mut now = now;
    // Every place after the first: built, filled, held for the whole of this
    // function, and given back at the end. They are here rather than in an array
    // beside `place` because the first place is *scripted* and the others are
    // not, and a single array would have made that difference invisible.
    let mut extras: [Option<Extra>; PLACES_MAX] = [const { None }; PLACES_MAX];

    // The supervisor's own table, on this stack rather than in a per-CPU slot.
    // A supervisor is a component and its table is an object it paid for; the
    // frame holding one for the length of a boot-time demonstration is the
    // smallest thing that is not a lie, and it is what keeps this file free of
    // kernel-global state entirely.
    let mut supervisor = Table::EMPTY;
    // The reservations this supervisor has been granted, on its stack beside
    // its capability table and for the same reason that one is there: a
    // reservation is part of what a component's account paid for, and a table
    // in a `static` would be kernel-global mutable state the frame's own rule
    // forbids. RFC 0050; RFC 0007 is the arithmetic it holds.
    //
    // One table for the whole demonstration, and that is the load-bearing part.
    // A table built per admission would answer every demand as if it were the
    // first, so a second hard-class component would be admitted onto cores the
    // first already holds — a schedulability test that passes because it has
    // forgotten.
    let mut reservations = crate::admit::table().map_err(|why| Failure::Admission(why.code()))?;
    // The supervisor's own account, and it is granted before anything else in
    // this table on purpose: [`Table::grow`] charges the lowest-indexed
    // `Untyped` that carries `DERIVE`, so this is the slot that pays for every
    // page of slots the supervisor buys. Granted rather than derived because
    // there is nothing above a supervisor to derive it from — which is the
    // shape of the whole file: the frame is standing in for a component that
    // does not exist yet, and where it has to mint something out of nothing it
    // says so. [`grant_into`] argues the choice this account exists to make
    // affordable.
    let own = frames
        .alloc_zeroed(Order::new(SUPERVISOR_ORDER).ok_or(Failure::NoMemory)?)
        .ok_or(Failure::NoMemory)?;
    supervisor
        .grant(
            CapType::Untyped,
            rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE,
            own.addr(),
            own.bytes(),
        )
        .map_err(Failure::Capability)?;
    // A supervisor is a component, so it is owed notices like any other: the
    // peer-gone that tells it its child died is the ordinary route and there is
    // no separate wait-for-child. `Table::posts_notices` says why this is a flag
    // at all and when it stops being one.
    supervisor.owes_notices();
    // And it needs somewhere for those notices to be *published to*. Owing one
    // is a debt and not an event; R05 says every event a component receives is
    // a completion entry on its control ring, drained at a polling point, so a
    // supervisor with pending state and no ring would be a frame keeping a
    // ledger nobody can read. One frame, given back with the account below.
    let ledger = frames.alloc_zeroed(Order::FRAME).ok_or(Failure::NoMemory)?;
    let ledger_at = frames.virt(ledger);
    // SAFETY: allocated zeroed a line ago, frame-aligned — which is stronger than
    // the cache-line alignment the layout asks for — and `FRAME_SIZE` bytes, with
    // no pointer into it held anywhere else.
    let ledger_ring = unsafe {
        Mapping::describe(
            ledger_at,
            FRAME_SIZE as u32,
            CONTROL_ENTRIES,
            0,
            f_abi::feature::CONTROL_EVENTS,
            f_abi::feature::CONTROL_EVENTS,
        )
    }
    .map_err(Failure::Ring)?;

    crate::kprintln!(
        "  supervisor    {} place(s) from {} component file(s), one per file, each staked with \
         an account its own manifest sized",
        report.places,
        count,
    );

    let region = frames
        .alloc_zeroed(account_order(record).ok_or(Failure::Manifest(Refusal::Value))?)
        .ok_or(Failure::NoMemory)?;
    let account = Account {
        handle: grant_into(
            &mut supervisor,
            frames,
            CapType::Untyped,
            rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE | rights::GRANT,
            region.addr(),
            region.bytes(),
        )?,
        floor: region.addr(),
    };

    // The endpoint carries the five rights defined on one and not the sixth.
    // RFC 0008 refuses to mint `EXECUTE` here rather than leaving it to mean
    // nothing: `rights::narrows` would route an undefined bit down every path,
    // and a later ABI that gave it a meaning would widen authority already
    // granted everywhere, with no derivation and no notice.
    let endpoint = grant_into(
        &mut supervisor,
        frames,
        CapType::Endpoint,
        rights::ALL & !rights::EXECUTE,
        0,
        0,
    )?;

    let mut place = Place {
        manifest: ContentId::of(module),
        module,
        endpoint,
        epoch: 0,
        occupant: None,
        retired: false,
        reservation: None,
        budget: policy::Budget::default(),
        faults: 0,
        exits: 0,
        stops: 0,
        restarts: 0,
    };

    declared_line(record, region.bytes());
    // The supervisor's own admission test, before it spends anything building
    // an offer. The frame runs the same one again inside `spawn`, and that is
    // not redundancy: the supervisor is checking whether it can afford to ask,
    // and the frame is checking what it was asked, from a table it does not
    // trust. RFC 0030's *there is no path by which a supervisor's belief
    // becomes the frame's belief* is this line and the one in `spawn` being
    // two lines.
    admit(
        record,
        &supervisor,
        &account,
        text_pages(record.image(module).map_err(Failure::Manifest)?)?,
        &reservations,
        place.reservation,
    )?;
    admitted_line(record, region.bytes());

    // ---------------------------------------------------------------- spawn
    let offered = offer(&mut supervisor, &account, &place, record, frames)?;
    // SAFETY: the caller's guarantee, passed down.
    let spawned = unsafe {
        spawn(
            frames,
            kernel,
            features,
            &mut place,
            &account,
            &mut supervisor,
            &reservations,
            offered,
        )
    }?;
    report.spawns += 1;
    report.unbound += unbound_needs(record);
    spawned_line(record, &place, spawned);

    // And the reservation the *place* holds, kept once and not per occupant.
    // RFC 0007's memory is never reclaimed for the life of the reservation and
    // RFC 0041's place survives its occupant, so a restart into this place
    // keeps these cores and these pages rather than asking for them again — and
    // `admit` above therefore tests without keeping. This is the one line that
    // spends, and it spends after the spawn rather than before it, so a spawn
    // refused for a need it was not offered leaves nothing behind.
    //
    // The grant goes **onto the place**, which is the half that was missing:
    // the keeping is only half of *a spawn tests, a place keeps* if the next
    // spawn into this place can tell that it is the holder rather than a second
    // claimant. `Place::reservation` is what `admit` reads to tell them apart.
    place.reservation = Some(
        reservations.grant(&Demand::of(record)).map_err(|why| Failure::Admission(why.code()))?,
    );

    // The first polling point. Everything the two tables owe goes onto their
    // control rings and comes back off, which is the half of R05 a pending-state
    // machine does not by itself provide: state nobody publishes is a debt.
    publish(&mut place, &mut supervisor, &ledger_ring, &mut report)?;

    // ------------------------------------------------------ the other places
    //
    // One per remaining component file, in the order the loader placed them.
    // This is the loop `JOIN_GAP` was a declaration about: until it existed, the
    // boot half of RFC 0035's pair covered `{store}` while the workload half
    // covered every record the build produced, and RFC 0036 made the difference
    // a set somebody had to write down rather than a sentence nobody re-read.
    for index in 1..count {
        let Some(module) = modules.get(index).copied() else { break };
        // SAFETY: the caller's guarantee, passed down; `module` came out of
        // `modules` above and is one of the boot modules the direct map covers.
        let mut extra = unsafe {
            fill(frames, kernel, features, &mut supervisor, &mut reservations, module, &mut report)
        }?;
        // This place's own polling point, not the first place's: what a spawn
        // owes is owed to *its* occupant, and pumping the wrong ring would leave
        // a component holding capabilities it was never told about while the
        // counters said every notice had been delivered.
        publish(&mut extra.place, &mut supervisor, &ledger_ring, &mut report)?;
        let Some(slot) = extras.get_mut(index) else { return Err(Failure::WrongPlace) };
        *slot = Some(extra);
    }

    // -------------------------------------------------------------- connect
    // A client holds the endpoint with `WRITE`, which is what `write` means on
    // one: the right to connect. It is a derivation of the supervisor's, so
    // revoking the supervisor's reaches it.
    let client = supervisor
        .derive(place.endpoint, rights::READ | rights::WRITE, &mut backing(frames))
        .map_err(Failure::Capability)?;
    let mut pending: Option<PendingConnect> = None;
    let opened = connect(frames, &mut place, client, 0, &mut pending, &mut report, false)?;
    if !matches!(opened, Answer::Channel(_)) {
        return Err(Failure::Connect(0));
    }
    crate::kprintln!("  connect       client -> place {}: {}", Name(record.label()), opened);

    // ------------------------------------------------------------- the fault
    // RFC 0008's first way in: *a fault at ring 3 — any exception, and also a
    // control ring whose header the component corrupted, which the frame treats
    // as the component having stopped speaking*. The second is the one this
    // demonstration can provoke without a scheduler, and it is not a lesser
    // one: a component that scribbles the sixty-four bytes its supervisor talks
    // to it through has stopped being reachable, and every notice the frame
    // owes it is now unpayable.
    let occupant = place.occupant.as_ref().ok_or(Failure::WrongPlace)?;
    // SAFETY: `control` is the kernel address of a frame this function retyped
    // out of the account and handed to nobody else, and the instance has never
    // run, so nothing is reading it.
    unsafe { (occupant.control as *mut u64).write_volatile(!f_abi::CHANNEL_MAGIC) };
    // SAFETY: as above. The bytes are hostile now, which is the subject rather
    // than a safety obligation: `adopt` dereferences nothing derived from them
    // unless it returns, and here it must not.
    let speaking = unsafe { Mapping::adopt(occupant.control as *mut u8, FRAME_SIZE as u32, 0, 0) };
    if speaking.is_ok() {
        return Err(Failure::Ring(0));
    }
    let cause = cause::pack(cause::FAULT, u64::from(error::argument::MALFORMED_HEADER));
    let torn = tear_down(frames, &mut place, &mut supervisor, &account, cause, &mut report)?;
    crate::kprintln!(
        "  fault         place {} epoch {} stopped speaking: its control ring header no longer \
         validates",
        Name(record.label()),
        spawned.0,
    );
    crate::kprintln!(
        "  teardown      {} capabilit(ies) revoked of {} slot(s), {} frame(s) refunded to \
         the account, {} peer-gone notice(s)",
        torn.0,
        torn.1,
        torn.2,
        torn.3,
    );
    // The second polling point: what the teardown revoked, and the peer-gone
    // that is how a supervisor learns its child died.
    publish(&mut place, &mut supervisor, &ledger_ring, &mut report)?;

    // ---------------------------------------------- the connect that pends
    let opened = connect(frames, &mut place, client, PEND_TICKS, &mut pending, &mut report, false)?;
    if !matches!(opened, Answer::Pending) {
        return Err(Failure::Connect(0));
    }
    crate::kprintln!(
        "  connect       client -> place {}: the place is empty, the connect pends to a deadline \
         {} tick(s) out",
        Name(record.label()),
        PEND_TICKS,
    );

    // The other outcome a connect has while a place is empty, taken on purpose,
    // because two outcomes and a silence is how E1-B05 and E1-P06 would each
    // invent the third. A probe submitted with a deadline already behind it is
    // answered `PEER/EMPTY` — *not* `GONE`, because the place may yet be
    // refilled and a client that can wait longer may submit again. This is the
    // frame's own probe and never the client under test, which is why it is
    // counted apart from `Report::lost`.
    let mut probe = Some(PendingConnect { place: 0, endpoint: client, deadline: 0 });
    let expired = expire(&place, &mut probe, 1).ok_or(Failure::Connect(0))?;
    if error::unpack(expired) != Some((error::PEER, error::peer::EMPTY)) {
        return Err(Failure::Connect(expired));
    }
    report.probed += 1;
    crate::kprintln!(
        "  outcomes      a connect whose own deadline had passed earned PEER/EMPTY, which is \
         not GONE: the place may yet be refilled"
    );

    // ------------------------------------------------------- the refusals
    // SAFETY: the caller's guarantee, passed down; the place is empty here, so
    // nothing below can displace an occupant.
    let refusals = unsafe {
        probe_refusals(
            frames,
            kernel,
            features,
            &mut place,
            &account,
            &mut supervisor,
            &reservations,
            record,
        )
    }?;
    report.probed += refusals;
    crate::kprintln!(
        "  refusals      {} spawn(s) refused on purpose, one per way a supply can be wrong: \
         missing, undeclared, wrong type, short rights, short quantity",
        refusals,
    );

    // -------------------------------------------------------------- restart
    // The supervisor's act, under the manifest's declared policy. `now` is a
    // tick count read once through `Env` and advanced by the backoff the policy
    // itself returns — which is what a supervisor does, and is why nothing in
    // this log moves between a fast host and a slow one.
    let verdict = policy::decide(record, &mut place.budget, true, false, now);
    // The demonstration's manifest declares a policy that restarts after a
    // fault and a budget with room in it, so any other verdict here is the
    // frame and the record disagreeing about what was declared — which is a
    // boot failure rather than a smaller result.
    let policy::Verdict::Restart(pause) = verdict else { return Err(Failure::WrongPlace) };
    now = now.saturating_add(u64::from(pause));
    place.restarts += 1;
    report.restarts += 1;
    crate::kprintln!(
        "  restart       place {} under {} — restart {} of {}, backoff {} tick(s)",
        Name(record.label()),
        restart::label(record.restart),
        place.budget.used,
        record.max_restarts,
        pause,
    );

    let offered = offer(&mut supervisor, &account, &place, record, frames)?;
    // SAFETY: as the first spawn.
    let spawned = unsafe {
        spawn(
            frames,
            kernel,
            features,
            &mut place,
            &account,
            &mut supervisor,
            &reservations,
            offered,
        )
    }?;
    report.spawns += 1;
    crate::kprintln!(
        "  spawn         place {} epoch {} — nothing carried over: new table, new memory, \
         new control ring",
        Name(record.label()),
        spawned.0,
    );

    // --------------------------------------------------------------- resume
    // The pending connect is answered by the refill, which is the first of its
    // three outcomes and the one gate G1's sentence rests on.
    let resumed = resume(frames, &mut place, &mut pending, &mut report)?;
    report.epoch = resumed;
    crate::kprintln!(
        "  resume        the pending connect completed: a channel to epoch {}, and the client \
         observed only the wait",
        resumed,
    );
    publish(&mut place, &mut supervisor, &ledger_ring, &mut report)?;

    // ------------------------------------------------------------------ stop
    // The third way a component dies, and the one a supervisor chooses. RFC
    // 0008: a stop with no deadline is a promise nothing can refuse and the
    // frame refuses to make it; a stop whose deadline has already passed is a
    // kill, and it is spelled the same way as a polite stop so that the
    // simulator's *kill this driver at a seeded moment* is one opcode rather
    // than two paths through the frame.
    //
    // The promise is made before the kill so that the word it lives in is
    // exercised: a second stop may only move it earlier, and a promise that
    // could be relaxed by whoever made it is what R08 refuses to call a
    // deadline. The two grades go on beside it for the same reason — the
    // publication order `f_abi::control::ORDER` fixes is only an order if
    // something is pending in more than one of its phases at once.
    let occupant = place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
    if !occupant.table.stop_by(STOP_DEADLINE) {
        return Err(Failure::Connect(0));
    }
    if occupant.table.stop_by(STOP_DEADLINE + 1) {
        // A later deadline moved an earlier one, which is the one thing a stop
        // may never do.
        return Err(Failure::Connect(0));
    }
    // Latest wins, so setting a grade twice is one notice and the *second*
    // value — which is the whole difference between a grade and a queue, and
    // is worth driving rather than describing.
    occupant.table.pressure_is(1);
    if !occupant.table.pressure_is(PRESSURE_GRADE) {
        return Err(Failure::Notice(0));
    }
    occupant.table.generation_is(GENERATION_GRADE);
    let kept = occupant.table.stop_deadline().unwrap_or_default();
    // Published before the teardown, because a teardown owes nothing to a
    // component that no longer exists — `Table::clear_all` says so and it is
    // right. A stop notice a component is never told is a stop it cannot obey.
    publish(&mut place, &mut supervisor, &ledger_ring, &mut report)?;

    let cause = cause::pack(cause::STOPPED, kept);
    let torn = tear_down(frames, &mut place, &mut supervisor, &account, cause, &mut report)?;
    crate::kprintln!(
        "  stop          place {} epoch {} stopped against a deadline already behind it — a \
         kill, and a second stop could not move it later",
        Name(record.label()),
        spawned.0,
    );
    crate::kprintln!(
        "  teardown      {} capabilit(ies) revoked of {} slot(s), {} frame(s) refunded to \
         the account, {} peer-gone notice(s)",
        torn.0,
        torn.1,
        torn.2,
        torn.3,
    );

    // ------------------------------------------------------------- retirement
    // The budget's far end. RFC 0008 gives a place three fates and the third is
    // the one a demonstration that stopped at *restart* would leave to E1-P06
    // to discover: a place whose budget ran out is *retired*, and a connect to
    // a retired place is `PEER/GONE` rather than a wait.
    let exhausted = loop {
        match policy::decide(record, &mut place.budget, true, false, now) {
            policy::Verdict::Restart(pause) => now = now.saturating_add(u64::from(pause)),
            other => break other,
        }
    };
    if exhausted != policy::Verdict::Retire {
        return Err(Failure::WrongPlace);
    }
    let cause = cause::pack(cause::RETIRED, u64::from(place.epoch));
    let torn = tear_down(frames, &mut place, &mut supervisor, &account, cause, &mut report)?;
    crate::kprintln!(
        "  retire        place {} spent its budget of {} restart(s) — retired, and {} \
         peer-gone notice(s) went to the endpoint's holders",
        Name(record.label()),
        record.max_restarts,
        torn.3,
    );

    // Both ways a client meets a retired place, taken as probes: a fresh
    // connect, and a connect that was already pending when the place went.
    // `GONE` and not `EMPTY`, because a client told `GONE` is right to give up.
    let gone = connect(frames, &mut place, client, 0, &mut pending, &mut report, true)?;
    let Answer::Refused(code) = gone else { return Err(Failure::Connect(0)) };
    if error::unpack(code) != Some((error::PEER, error::peer::GONE)) {
        return Err(Failure::Connect(code));
    }
    let mut probe = Some(PendingConnect { place: 0, endpoint: client, deadline: 0 });
    let waited = expire(&place, &mut probe, 1).ok_or(Failure::Connect(0))?;
    if error::unpack(waited) != Some((error::PEER, error::peer::GONE)) {
        return Err(Failure::Connect(waited));
    }
    // One, not two: `connect` counted its own probe on the way past, because it
    // is the branch a *client* would have taken and the counter it lands in is
    // the only thing that differs.
    report.probed += 1;
    crate::kprintln!(
        "  outcomes      a connect to a retired place earned PEER/GONE, arriving and already \
         waiting: the place is not coming back"
    );

    // The window the budget is counted over, driven at its edge. Its own
    // arithmetic, on a tally of its own, because the place above is retired and
    // a retired place is not restarted whatever a window says: what is under
    // test here is that a count exhausted *inside* the window is not exhausted
    // once the window has elapsed, which is the difference between a budget and
    // a lifetime cap — `docs/manifest.md` says the second is schema 2's.
    let mut window = policy::Budget { used: record.max_restarts, opened: now };
    let inside = policy::decide(record, &mut window, true, false, now);
    let after = policy::decide(
        record,
        &mut window,
        true,
        false,
        now.saturating_add(u64::from(record.budget_window_ticks)),
    );
    if inside != policy::Verdict::Retire || !matches!(after, policy::Verdict::Restart(_)) {
        return Err(Failure::WrongPlace);
    }
    crate::kprintln!(
        "  budget        a window {} tick(s) wide: {} restart(s) inside it retires the place, \
         and the same count once it has elapsed does not",
        record.budget_window_ticks,
        record.max_restarts,
    );

    // ------------------------------------------------- the other places, back
    //
    // Uniform teardown, and *uniform* is the word doing the work: this is the
    // same [`tear_down`] the scripted place above reached three times by three
    // different routes, called here on an occupant that did nothing wrong. RFC
    // 0008 asks for one path out for a fault, an exit and a stop, and a
    // demonstration in which the unscripted places were dismantled some other
    // way would have two.
    for index in (1..PLACES_MAX).rev() {
        let Some(extra) = extras.get_mut(index).and_then(Option::as_mut) else { continue };
        let record = Record::read(extra.place.module).map_err(Failure::Manifest)?;
        let cause = cause::pack(cause::STOPPED, STOP_DEADLINE);
        let torn = tear_down(
            frames,
            &mut extra.place,
            &mut supervisor,
            &extra.account,
            cause,
            &mut report,
        )?;
        crate::kprintln!(
            "  teardown      place {} — {} capabilit(ies) revoked, {} frame(s) refunded to its \
             own account, {} peer-gone notice(s)",
            Name(record.label()),
            torn.0,
            torn.2,
            torn.3,
        );
        publish(&mut extra.place, &mut supervisor, &ledger_ring, &mut report)?;
    }

    // -------------------------------------------------------------- notices
    publish(&mut place, &mut supervisor, &ledger_ring, &mut report)?;
    report.owed = supervisor.owes();
    if report.owed != 0 || report.collected != report.notices {
        return Err(Failure::Leaked);
    }
    crate::kprintln!(
        "  notices       {} published in slot-then-stop-then-grade order over {} round(s), {} \
         drained back at a polling point as {} of 7 kind(s), {} still owed",
        report.notices,
        report.rounds,
        report.collected,
        report.kinds.count_ones(),
        report.owed,
    );

    supervisor.clear_all();
    // SAFETY: the account was allocated here, every frame retyped out of it has
    // been refunded, and no address space names any of it: the instances are
    // gone and their spaces were never in `CR3`.
    unsafe { frames.free(region) };
    for index in 1..PLACES_MAX {
        let Some(extra) = extras.get(index).and_then(Option::as_ref) else { continue };
        // SAFETY: as `region`. Each was allocated by `fill`, its occupant has
        // been torn down and every frame refunded to it, and the table naming
        // it was cleared a line ago.
        unsafe { frames.free(extra.region) };
    }
    // SAFETY: allocated here; the only thing that ever charged it is the
    // supervisor's own table, which was cleared a line ago, and no address
    // space names any of it.
    unsafe { frames.free(own) };
    // SAFETY: allocated here, and the mapping over it is past its last use —
    // the publish above is the last thing that touched it.
    unsafe { frames.free(ledger) };

    if frames.free_count() != before {
        return Err(Failure::Leaked);
    }

    report.faults = place.faults;
    Ok(report)
}

/// A place the frame fills and holds without scripting it.
///
/// The three things that have to travel together for a place to be given back
/// exactly: what it is, the account its occupant was retyped out of, and the
/// block that account was staked from. Holding two of the three is how a
/// teardown comes to refund frames to an account and leak the region under it.
struct Extra {
    /// The place, with its occupant while it has one.
    place: Place,
    /// The account, in the supervisor's table.
    account: Account,
    /// The block the account was staked from, so it can be freed.
    region: Frame,
}

/// Build a place from one component file, stake it with an account its own
/// manifest sized, and put its first occupant in.
///
/// The unscripted half of [`demonstrate`], and it is a function rather than
/// inline code for one reason: the first place and the others have to be built
/// the same way. A second body here would be a second set of refusals, and the
/// day they disagreed the boot log would say two places were filled and mean two
/// different things by it.
///
/// # Errors
///
/// [`Failure`], naming which step did not hold. Every one of them fails the
/// boot: a place half-built is worse than a place not built.
///
/// # Safety
///
/// As [`demonstrate`], and `module` must be one of the boot modules the direct
/// map covers.
unsafe fn fill(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    supervisor: &mut Table,
    reservations: &mut Reservations,
    module: &'static [u8],
    report: &mut Report,
) -> Result<Extra, Failure> {
    // `fill` keeps the table by value below; every call it makes downwards
    // takes a shared reference, because a spawn tests and does not keep.
    let record = Record::read(module).map_err(Failure::Manifest)?;
    let region = frames
        .alloc_zeroed(account_order(record).ok_or(Failure::Manifest(Refusal::Value))?)
        .ok_or(Failure::NoMemory)?;
    let account = Account {
        handle: grant_into(
            supervisor,
            frames,
            CapType::Untyped,
            rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE | rights::GRANT,
            region.addr(),
            region.bytes(),
        )?,
        floor: region.addr(),
    };
    let endpoint =
        grant_into(supervisor, frames, CapType::Endpoint, rights::ALL & !rights::EXECUTE, 0, 0)?;
    let mut place = Place {
        manifest: ContentId::of(module),
        module,
        endpoint,
        epoch: 0,
        occupant: None,
        retired: false,
        reservation: None,
        budget: policy::Budget::default(),
        faults: 0,
        exits: 0,
        stops: 0,
        restarts: 0,
    };

    declared_line(record, region.bytes());
    admit(
        record,
        supervisor,
        &account,
        text_pages(record.image(module).map_err(Failure::Manifest)?)?,
        reservations,
        place.reservation,
    )?;
    admitted_line(record, region.bytes());

    let offered = offer(supervisor, &account, &place, record, frames)?;
    // SAFETY: the caller's guarantee, passed down. The place was built a few
    // lines ago and is empty.
    let spawned = unsafe {
        spawn(frames, kernel, features, &mut place, &account, supervisor, reservations, offered)
    }?;
    report.spawns += 1;
    report.unbound += unbound_needs(record);
    spawned_line(record, &place, spawned);

    // And the reservation the *place* holds, kept once and not per occupant.
    // RFC 0007's memory is never reclaimed for the life of the reservation and
    // RFC 0041's place survives its occupant, so a restart into this place
    // keeps these cores and these pages rather than asking for them again — and
    // `admit` above therefore tests without keeping. This is the one line that
    // spends, and it spends after the spawn rather than before it, so a spawn
    // refused for a need it was not offered leaves nothing behind.
    //
    // The grant goes **onto the place**, which is the half that was missing:
    // the keeping is only half of *a spawn tests, a place keeps* if the next
    // spawn into this place can tell that it is the holder rather than a second
    // claimant. `Place::reservation` is what `admit` reads to tell them apart.
    place.reservation = Some(
        reservations.grant(&Demand::of(record)).map_err(|why| Failure::Admission(why.code()))?,
    );

    Ok(Extra { place, account, region })
}

/// What the manifest declares about a place, before anything has been spent on
/// it.
///
/// Three lines per place rather than one, because they are printed at three
/// moments and the order is the argument: what was declared, then the admission
/// test that could have refused it, then what the spawn did. One line printed
/// afterwards would say the same words about a decision nobody could watch being
/// made.
fn declared_line(record: &Record, account_bytes: u64) {
    crate::kprintln!(
        "  place         {}: {}, {}, {} restart(s) in {} tick(s), {} B account",
        Name(record.label()),
        f_abi::manifest::domain::label(record.domain),
        restart::label(record.restart),
        record.max_restarts,
        record.budget_window_ticks,
        account_bytes,
    );
}

/// What admission compared, and what it left over.
///
/// The slack is printed rather than left inside [`account_order`], because an
/// account is a buddy block and a manifest is a byte count: they meet at a power
/// of two, and a supervisor that could not see the difference could not tell a
/// quota it chose from one the allocator rounded it into.
fn admitted_line(record: &Record, staked: u64) {
    crate::kprintln!(
        "  admission     {} class, {} B footprint + {} B of declared need(s) against a {} B \
         account — refused before anything is spent, never after",
        f_abi::manifest::class::label(record.class),
        record.memory_bytes,
        account_bytes(record).saturating_sub(record.memory_bytes),
        staked,
    );
}

/// What a spawn put in a place.
///
/// The manifest's content hash is on **this** line and on no other, and that is
/// load-bearing rather than tidy: `xtask`'s `spawned_from` reads the boot log
/// for it, and RFC 0036's join is a comparison of that set against the set the
/// simulator runs. A hash printed twice per place would make the boot claim to
/// have spawned twice as many components as it did.
fn spawned_line(record: &Record, place: &Place, spawned: (u32, usize, usize)) {
    crate::kprintln!(
        "  spawn         place {} epoch {} — manifest {:#018x}, {} need(s) supplied, type, \
         rights and quantity checked; {} frame(s) from the account; control ring {} B",
        Name(record.label()),
        spawned.0,
        place.manifest.bits(),
        spawned.1,
        spawned.2,
        FRAME_SIZE,
    );
}

/// The pressure grade the demonstration publishes.
///
/// Two, which is a number with no meaning yet: RFC 0008 reserves the word and
/// E1-B07 is where an account acquires a pressure scale. What is under test is
/// the *latest-wins* rule and the publication order, and both need a value that
/// is distinguishable from the one written a line earlier.
/// Unit: none — a grade ordinal.
const PRESSURE_GRADE: u64 = 2;

/// The system generation the demonstration publishes.
///
/// One. RFC 0006 and RFC 0012 say what a generation change is; this is the
/// frame proving the word reaches a component through the same fixed order as
/// everything else.
/// Unit: none — a generation ordinal.
const GENERATION_GRADE: u64 = 1;

/// The deadline the demonstration stops its component against.
///
/// One, which is a nanosecond into the control channel's epoch and therefore
/// already behind every component this frame has ever run — which is the point.
/// RFC 0008 says a stop whose deadline has already passed *is* a kill, and
/// spelling it the same way is what makes the simulator's "kill this driver at a
/// seeded moment" one opcode. Not zero, because zero is `NO_DEADLINE` and a stop
/// carrying it is an `ARGUMENT` error rather than a kill: a promise nothing can
/// refuse is not one the frame will make.
/// Unit: nanoseconds, monotonic, in the control channel's epoch.
const STOP_DEADLINE: u64 = 1;

/// How long the demonstration's client is prepared to wait for a refill.
///
/// Two hundred ticks, which is twice the backoff the worked manifest declares
/// and is a *count* rather than a duration — a deadline in a boot log has to be
/// the same number on an emulator and on a machine, and `process.rs` argues that
/// at length. Nothing here waits for it to elapse: the refill answers first,
/// which is the outcome under test.
const PEND_TICKS: u64 = 200;

/// Put a component in a place.
///
/// Everything it is made of comes out of the account, a page at a time, through
/// the same derive a component's own growth uses — so a supervisor that has run
/// out cannot spawn and the refusal is `RESOURCE/QUOTA_EXHAUSTED`.
///
/// `offered` is what the supervisor supplied: one handle per need in the
/// manifest's order, and the frames its account was charged for while building
/// them. It is *checked* here and never trusted — see [`check_needs`] — and the
/// check runs before the first frame of the instance itself is charged, so a
/// refused spawn has spent nothing of the instance's.
///
/// Answers the epoch it went in at, how many needs were satisfied, and how many
/// frames the account paid for altogether.
///
/// # Safety
///
/// As [`demonstrate`].
#[expect(
    clippy::too_many_arguments,
    reason = "every one is a thing a spawn is not allowed to invent: the machine's memory, \
              its address space, what it agreed to interpret, the place, the account it \
              spends, the supervisor's table it checks a supply against, the reservations \
              it tests the demand against, and the supply itself. Bundling them would be a \
              type that exists so a lint passes, which `runtime::demonstrate` already \
              declined for this reason"
)]
unsafe fn spawn(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    place: &mut Place,
    account: &Account,
    supervisor: &mut Table,
    reservations: &Reservations,
    offered: Supply,
) -> Result<(u32, usize, usize), Failure> {
    if place.occupant.is_some() || place.retired {
        return Err(Failure::WrongPlace);
    }
    let record = Record::read(place.module).map_err(Failure::Manifest)?;
    // A different manifest is a different place. The hash is over the record
    // and the image together, so this refuses a component whose *code* changed
    // as firmly as one whose declaration did.
    if ContentId::of(place.module) != place.manifest {
        return Err(Failure::WrongPlace);
    }
    let image = record.image(place.module).map_err(Failure::Manifest)?;
    let pages = text_pages(image)?;
    admit(record, supervisor, account, pages, reservations, place.reservation)?;
    // Every refusal R04 asks of a spawn, decided here and before anything of
    // the instance's own is charged. The frame checks the supply against the
    // supervisor's table rather than against what the supervisor said about it,
    // which is the difference between validating an argument and believing a
    // caller.
    check_needs(record, supervisor, &offered)?;

    // Frames out of the account: one per page of text, then a stack, then a
    // control ring. Retyped through `derive`, which advances the account's
    // watermark — so this is the supervisor spending, not the frame allocating,
    // and a supervisor that has run out is refused rather than served from
    // anything the frame keeps back.
    //
    // A page at a time and not one block, and the reason is the watermark: an
    // account is spent in frames and refunded from the top of one, so a
    // multi-page text that was one retype would be a refund that could only
    // come back whole. What that costs is that the pages are not contiguous by
    // construction, which is why the image is copied page by page below rather
    // than in one call.
    //
    // The needs were charged before the offer was made, so their frames are
    // already in `offered.charged` and this appends to that list rather than
    // starting one: everything an instance cost has to be in one place for the
    // teardown to give all of it back.
    let mut charged = offered.charged;
    let mut charges = offered.charges;

    // SAFETY: the caller's guarantee that the kernel's space is live and frames
    // are addressable through its direct map.
    let mut space = unsafe { paging::user_space(frames, kernel) }.map_err(Failure::Space)?;

    // One page at a time, charged and copied and mapped before the next is
    // asked for, and nothing about the text kept afterwards. **No array**, and
    // that is a fix rather than a style: a list of every address this maps,
    // sized by the bound the *handles* are sized by, put a kilobyte on the boot
    // processor's kernel stack per place and overflowed it into its guard page
    // — a double fault with nothing in it that says where it came from.
    //
    // A page at a time and not one block, for the account's sake: an account is
    // spent in frames and refunded from the top of a watermark, so a multi-page
    // text that was one retype would be a refund that could only come back
    // whole. What that costs is that the pages are not contiguous, which is why
    // the image is copied a page at a time here rather than in one call.
    for page in 0..pages {
        let (handle, text) = charge(supervisor, account, frames)?;
        let Some(slot) = charged.get_mut(charges) else { return Err(Failure::Account) };
        *slot = handle;
        charges += 1;

        let from = page * FRAME_SIZE as usize;
        let Some(chunk) = image.get(from..) else { return Err(Failure::Account) };
        let bytes = chunk.len().min(FRAME_SIZE as usize);
        let into = frames.virt(Frame::from_addr(text));
        // SAFETY: `text` is a frame this function just retyped out of the
        // account and handed to nobody else; it is one frame, addressable
        // through the direct map, and `bytes` is at most one frame — taken as a
        // minimum rather than assumed, because the last page of an image is
        // almost never full.
        unsafe { core::ptr::copy_nonoverlapping(chunk.as_ptr(), into, bytes) };
        // SAFETY: as `user_space`, and `space` is not in `CR3` — it has never
        // been.
        unsafe {
            paging::map_user(
                frames,
                &mut space,
                crate::process::TEXT + page as u64 * FRAME_SIZE,
                text,
                UserPage::Text,
                features,
            )
        }
        .map_err(Failure::Space)?;
    }

    // The two fixed parts, after the text and in that order, because the order
    // is what a teardown gives them back in.
    let mut fixed = [0u64; FIXED_PARTS];
    for slot in &mut fixed {
        let (handle, object) = charge(supervisor, account, frames)?;
        let Some(at) = charged.get_mut(charges) else { return Err(Failure::Account) };
        *at = handle;
        charges += 1;
        *slot = object;
    }
    let (Some(&stack), Some(&control)) = (fixed.first(), fixed.get(1)) else {
        return Err(Failure::Account);
    };

    for (virt, phys, kind) in [
        (crate::process::SPAWN_STACK, stack, UserPage::Data),
        (crate::process::SPAWN_CONTROL, control, UserPage::Data),
    ] {
        // SAFETY: as `user_space`, and `space` is not in `CR3` — it has never
        // been.
        unsafe { paging::map_user(frames, &mut space, virt, phys, kind, features) }
            .map_err(Failure::Space)?;
    }

    // The control ring, written where the frame can reach it and mapped where
    // the component can. The frame is the only producer on it, which is what
    // `CONTROL_ENTRIES` sizes and what RFC 0008 makes non-negotiable: a
    // supervisor never speaks to its child directly, it asks the frame.
    let at = frames.virt(Frame::from_addr(control));
    // SAFETY: the frame was zeroed above, is frame-aligned — stronger than the
    // cache-line alignment the layout needs — and is `FRAME_SIZE` bytes.
    // Nothing outside this function holds a pointer into it.
    let ring = unsafe {
        Mapping::describe(
            at,
            FRAME_SIZE as u32,
            CONTROL_ENTRIES,
            place.epoch,
            f_abi::feature::CONTROL_EVENTS,
            f_abi::feature::CONTROL_EVENTS,
        )
    }
    .map_err(Failure::Ring)?;
    // A control ring is the one channel on which `CONTROL_EVENTS` is *required*
    // rather than offered: a control ring whose peer cannot speak notices is
    // not a control ring, and the spawn does not proceed.
    if ring.negotiated().features & f_abi::feature::CONTROL_EVENTS == 0 {
        return Err(Failure::Ring(error::pack(error::PEER, error::peer::FEATURE_REQUIRED)));
    }

    // The table, and the notices it will owe. Filled before the component
    // exists to reach it, which is the same order `process::prepare` uses and
    // for the same reason.
    let mut table = Table::EMPTY;
    table.owes_notices();
    let mut satisfied = 0;
    let mut index = 0;
    for need in record.needs() {
        // An ask is not supplied at spawn. It arrives later, through the
        // powerbox, as a grant naming this component's endpoint.
        if need.route == route::POWERBOX {
            continue;
        }
        let handle = supplied_at(&offered, index);
        index += 1;
        // A need the manifest marks optional and the supervisor did not supply
        // is a slot the component does not get. `check_needs` has already
        // refused the case where that was not permitted.
        if handle == Handle::NULL {
            continue;
        }
        let found = supervisor.inspect(handle).map_err(Failure::Need)?;
        // The declared rights and nothing beside them. `OFFERED` is what the
        // supervisor added so that it could hand the capability on and give the
        // name up again afterwards, and neither of those is the child's
        // business — R06: the child receives exactly what was listed.
        table.grant(found.kind, need.rights, found.object, found.extent).map_err(|_| {
            Failure::Capability(error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED))
        })?;
        satisfied += 1;
    }

    let epoch = place.epoch;
    place.occupant = Some(Instance {
        epoch,
        space,
        charged,
        charges,
        supplied: offered.handles,
        supplies: offered.count,
        control: at as u64,
        ring,
        table,
    });
    place.epoch += 1;
    Ok((epoch, satisfied, charges))
}

/// The refusals R04 asks of a spawn, decided before anything is spent.
///
/// RFC 0008: *the frame checks each one: it is of the declared type, it carries
/// at least the declared rights, and it carries `GRANT`, because handing it on
/// is what the supervisor is doing. A need not supplied and not optional
/// refuses the spawn. A handle supplied for a need the manifest does not
/// declare refuses the spawn.*
///
/// A fifth refusal is here that RFC 0008 states as a property of the account
/// rather than as a check on a handle: a need declares a *quantity* — bytes for
/// an untyped region, pages for a frame — and a handle naming less than that is
/// a component that would start and then discover it cannot run. `ADMISSION`
/// and not `RESOURCE`, for the reason [`admit`] gives at length.
///
/// Each earns its own code, because a caller that cannot tell which of them
/// happened cannot handle it as ordinary control flow (R07), and every one of
/// them is provoked at boot by [`probe_refusals`].
///
/// # Errors
///
/// [`Failure::Need`], carrying the packed refusal.
fn check_needs(record: &Record, supervisor: &Table, offered: &Supply) -> Result<(), Failure> {
    let declared = record.needs().iter().filter(|need| need.route != route::POWERBOX).count();
    // Positional, so a handle past the last need is a field the record does not
    // describe. `ARGUMENT/RESERVED_NOT_ZERO` is that refusal, and it is the
    // same one a record with a non-zero reserved byte earns — which is the
    // point: the spawn entry and the record are one argument between them.
    if offered.count > declared {
        return Err(Failure::Need(error::pack(
            error::ARGUMENT,
            error::argument::RESERVED_NOT_ZERO,
        )));
    }
    let mut index = 0;
    for need in record.needs() {
        if need.route == route::POWERBOX {
            continue;
        }
        let handle = supplied_at(offered, index);
        index += 1;
        if handle == Handle::NULL {
            if need.optional != 0 {
                continue;
            }
            return Err(Failure::Need(error::pack(
                error::AUTHORITY,
                error::authority::NO_SUCH_CAP,
            )));
        }
        let found = supervisor.inspect(handle).map_err(Failure::Need)?;
        let Some(kind) = need.cap_type() else { return Err(Failure::Manifest(Refusal::Value)) };
        if found.kind != kind {
            return Err(Failure::Need(error::pack(error::AUTHORITY, error::authority::WRONG_TYPE)));
        }
        if !rights::holds(found.rights, need.rights | rights::GRANT) {
            return Err(Failure::Need(error::pack(
                error::AUTHORITY,
                error::authority::RIGHT_NOT_HELD,
            )));
        }
        if found.extent < least_extent(need) {
            return Err(Failure::Need(error::pack(error::ADMISSION, error::admission::MEMORY)));
        }
    }
    Ok(())
}

/// The handle the supervisor supplied for the `index`th need it supplies at
/// spawn, or [`Handle::NULL`] for one it did not supply.
fn supplied_at(offered: &Supply, index: usize) -> Handle {
    if index >= offered.count {
        return Handle::NULL;
    }
    offered.handles.get(index).copied().unwrap_or(Handle::NULL)
}

/// The least a handle satisfying this need may name.
///
/// Unit: bytes. Zero for the types that do not span a range, and that is the
/// absence of a check rather than a weaker one: an endpoint has no size, and
/// comparing an extent against zero says so rather than inventing a bound.
fn least_extent(need: &Need) -> u64 {
    match need.cap_type() {
        Some(CapType::Untyped) => need.bytes,
        Some(CapType::Frame) => u64::from(need.frames) * FRAME_SIZE,
        _ => 0,
    }
}

/// Build what a supervisor supplies a spawn with: one handle per need, out of
/// its own account and its own endpoint.
///
/// This is the *supervisor's* side of RFC 0008's spawn entry and not the
/// frame's, and it is in this file only because the supervisor is — E1-B08
/// moves both. What it may not do is skip anything: a need's declared quantity
/// is carved out of the account here, so `bytes` and `frames` are numbers the
/// account actually pays rather than fields a reader assumed somebody read.
///
/// # Errors
///
/// [`Failure::Account`] when the account cannot pay for what the manifest
/// declares, and [`Failure::Capability`] when the supervisor is refused an
/// operation on its own table.
fn offer(
    supervisor: &mut Table,
    account: &Account,
    place: &Place,
    record: &Record,
    frames: &FrameAllocator,
) -> Result<Supply, Failure> {
    let mut out = Supply::EMPTY;
    for need in record.needs() {
        if need.route == route::POWERBOX {
            continue;
        }
        let Some(kind) = need.cap_type() else { return Err(Failure::Manifest(Refusal::Value)) };
        let held = need.rights | OFFERED;
        let handle = match kind {
            CapType::Untyped | CapType::Frame => {
                let (at, bytes) = carve(supervisor, account, frames, least_extent(need), &mut out)?;
                grant_into(supervisor, frames, kind, held, at, bytes)?
            }
            // An interrupt names no memory, so there is nothing to carve and
            // nothing to derive from. Which vector a device raises is the
            // topology's to bind and the machine's to know — `virtio-blk`'s
            // manifest says exactly that and refuses to name one — and this
            // build has no route by which a device interrupt reaches a
            // component at all. So what a driver is supplied here is a
            // capability **of the declared type, carrying the declared rights,
            // naming no vector**: enough to be spawned against, and not enough
            // to wait on. [`Report::unbound`] counts them rather than leaving
            // the sentence to be found, because a need satisfied by an object
            // the machine does not have is precisely the shape of a check that
            // is green while the property is false.
            // *Reversal:* E1-B09, which is the first thing that gives this
            // object something to be.
            CapType::Irq => grant_into(supervisor, frames, kind, held, 0, 0)?,
            // An endpoint or a channel routed from a sibling is the topology's,
            // and the topology is not in a manifest — `docs/manifest.md` says
            // so. The demonstration has one place and no siblings, so what a
            // sibling need gets here is the place's own endpoint, which is the
            // honest answer for a topology of one.
            _ => supervisor
                .derive(place.endpoint, held, &mut backing(frames))
                .map_err(Failure::Capability)?,
        };
        let Some(slot) = out.handles.get_mut(out.count) else { return Err(Failure::Account) };
        *slot = handle;
        out.count += 1;
    }
    Ok(out)
}

/// Take a contiguous run of frames out of the account and answer where it
/// starts and how long it is.
///
/// Contiguous because an account is a watermark and nothing else charges
/// against it between these calls — which is a fact about one supervisor with
/// one occupant per place, and not a property of `Table::derive`. *Reversal:* a
/// supervisor that builds two offers at once has to carve before it
/// interleaves, or ask the frame for a retype that takes a count.
fn carve(
    supervisor: &mut Table,
    account: &Account,
    frames: &FrameAllocator,
    bytes: u64,
    out: &mut Supply,
) -> Result<(u64, u64), Failure> {
    if bytes == 0 || !bytes.is_multiple_of(FRAME_SIZE) {
        return Err(Failure::Manifest(Refusal::Value));
    }
    let mut first = 0;
    let mut taken = 0;
    while taken < bytes {
        let (handle, object) = charge(supervisor, account, frames)?;
        if taken == 0 {
            first = object;
        }
        let Some(slot) = out.charged.get_mut(out.charges) else { return Err(Failure::Account) };
        *slot = handle;
        out.charges += 1;
        taken += FRAME_SIZE;
    }
    Ok((first, bytes))
}

/// Every refusal a spawn owes R04, taken on purpose.
///
/// A check nobody has watched fail is indistinguishable from one that cannot
/// fail, and these are the checks a supervisor above the frame will be the
/// first to trip. Each supply below is wrong in exactly one way and each must
/// earn its own code: a suite in which two probes earn one refusal has tested
/// one of them.
///
/// Nothing is spent. [`check_needs`] runs before the first frame of an instance
/// is charged, and none of these supplies carves anything out of the account —
/// the handles that have to exist are granted straight into the supervisor's
/// table, named against no memory, and given up again here.
///
/// Answers how many refusals were taken, which is fewer than five for a
/// manifest whose first need cannot express one: a need already declaring
/// `GRANT` has no *missing `GRANT`* to provoke. Reported rather than assumed,
/// because a suite that quietly shrinks is a suite that keeps saying everything
/// holds.
///
/// # Safety
///
/// As [`demonstrate`], and the place must be empty.
#[expect(
    clippy::too_many_arguments,
    reason = "it is [`spawn`]'s list, because it calls [`spawn`] five times with five wrong \
              supplies. A struct holding them would be a struct with one construction site \
              and one use, named after the lint that asked for it"
)]
unsafe fn probe_refusals(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    place: &mut Place,
    account: &Account,
    supervisor: &mut Table,
    reservations: &Reservations,
    record: &Record,
) -> Result<u32, Failure> {
    let declared = record.needs().iter().filter(|need| need.route != route::POWERBOX).count();
    let Some(first) = record.needs().iter().find(|need| need.route != route::POWERBOX) else {
        return Ok(0);
    };
    let Some(kind) = first.cap_type() else { return Err(Failure::Manifest(Refusal::Value)) };
    let least = least_extent(first);
    let mut taken = 0;

    // 1. A need not supplied, and not optional.
    // SAFETY: the caller's guarantee, and the place is empty.
    let outcome = unsafe {
        spawn(frames, kernel, features, place, account, supervisor, reservations, Supply::EMPTY)
    };
    taken += refused(outcome, error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP))?;

    // 2. A handle supplied for a need the manifest does not declare. Nothing
    //    has to be in the slot for this: what is refused is the *count*, and a
    //    supply that claimed more than the record describes is refused before
    //    any of it is looked at.
    let mut over = Supply::EMPTY;
    over.count = declared + 1;
    // SAFETY: as above.
    let outcome =
        unsafe { spawn(frames, kernel, features, place, account, supervisor, reservations, over) };
    taken += refused(outcome, error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO))?;

    // 3. The wrong type. Whichever of the two types that carry an extent the
    //    first need declares, this is the other one, so the probe is a type
    //    error and never also a quantity error.
    let other = if kind == CapType::Frame { CapType::Untyped } else { CapType::Frame };
    let mut wrong = Supply::EMPTY;
    let held =
        grant_into(supervisor, frames, other, first.rights | OFFERED, 0, least.max(FRAME_SIZE))?;
    wrong.handles[0] = held;
    wrong.count = 1;
    // SAFETY: as above.
    let outcome =
        unsafe { spawn(frames, kernel, features, place, account, supervisor, reservations, wrong) };
    taken += refused(outcome, error::pack(error::AUTHORITY, error::authority::WRONG_TYPE))?;
    supervisor.relinquish(held).map_err(Failure::Capability)?;

    // 4. The declared type and the declared quantity, without `GRANT`. Only
    //    provokable when the need does not itself declare `GRANT`.
    if first.rights & rights::GRANT == 0 {
        let mut weak = Supply::EMPTY;
        let held = grant_into(
            supervisor,
            frames,
            kind,
            first.rights | rights::REVOKE,
            0,
            least.max(FRAME_SIZE),
        )?;
        weak.handles[0] = held;
        weak.count = 1;
        // SAFETY: as above.
        let outcome = unsafe {
            spawn(frames, kernel, features, place, account, supervisor, reservations, weak)
        };
        taken += refused(outcome, error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD))?;
        supervisor.relinquish(held).map_err(Failure::Capability)?;
    }

    // 5. The declared type and rights, naming less than the declared quantity.
    //    Only provokable for a need that declares one.
    if least > 0 {
        let mut small = Supply::EMPTY;
        let held = grant_into(supervisor, frames, kind, first.rights | OFFERED, 0, least - 1)?;
        small.handles[0] = held;
        small.count = 1;
        // SAFETY: as above.
        let outcome = unsafe {
            spawn(frames, kernel, features, place, account, supervisor, reservations, small)
        };
        taken += refused(outcome, error::pack(error::ADMISSION, error::admission::MEMORY))?;
        supervisor.relinquish(held).map_err(Failure::Capability)?;
    }

    Ok(taken)
}

/// Require a spawn to have been refused, and with exactly this code.
///
/// A probe that was *not* refused fails the boot, and it has to: a spawn that
/// went through here has put an occupant in a place on the strength of a supply
/// nobody accepted, which is the failure the probe exists to find.
fn refused(outcome: Result<(u32, usize, usize), Failure>, want: i32) -> Result<u32, Failure> {
    match outcome {
        Err(Failure::Need(code)) if code == want => Ok(1),
        Err(why) => Err(why),
        Ok(_) => Err(Failure::Need(want)),
    }
}

/// Refuse a spawn before anything is spent, or let it through.
///
/// # Why this is `ADMISSION` and not `RESOURCE`
///
/// R08's distinction, and it is the whole reason this function exists rather
/// than the account simply running out. `RESOURCE/QUOTA_EXHAUSTED` is an
/// account that ran out *while it was being spent*, which a component recovers
/// from by spending less. This is a demand refused before anything was spent,
/// because the manifest states what the component is made of and the supervisor
/// offered less than that. A component does not start and then discover it
/// cannot run, and spawn is the moment of refusal — which is what RFC 0008 says
/// and what `CONTRIBUTING.md`'s R02 row names this task as landing.
///
/// Three refusals, and the third is the one worth reading.
///
/// **The account.** Every byte a component is made of is retyped from the
/// supplied `Untyped`, so an account holding less than `memory_bytes` is a
/// spawn that will fail partway through. Refused with
/// [`error::admission::MEMORY`], whose detail is what the account actually
/// holds.
///
/// **The reservation.** RFC 0007's arithmetic, which E1-B07 landed in
/// `f_abi::reserve` and `crate::admit` fills the machine description for. What
/// used to be here was a blanket refusal of the hard class with
/// [`error::admission::NOT_SCHEDULABLE`], because there was no arithmetic to
/// run; there is one now, so the demand is *tested* — cores under the
/// whole-core rule and whatever exclusion the part's missing partitioning
/// costs, the frame's own tick against the period and the slack, a partition or
/// a bandwidth allocation where the part offers one, and the pre-faulted pool —
/// and the refusal names which of the four could not be delivered rather than
/// naming the class.
///
/// The soft class goes through the same call and is refused its memory and
/// nothing else, which is what `docs/manifest.md` says the class means. One
/// path, so there is one vocabulary: RFC 0005 rule 2's *a kind is delivered in
/// full or the spawn is refused `ADMISSION`* is the same `Table::admit` and the
/// same codes, because a `hostile` component's whole core is RFC 0007's core
/// mechanism used for confidentiality — one mechanism, two claims.
///
/// It **tests and does not keep**, and the difference is the lifetime of a
/// reservation rather than a shortcut. A reservation is granted to a *place* —
/// RFC 0041's place survives its occupant, and RFC 0007's pre-faulted pages are
/// never reclaimed for the life of the reservation, so a restart into the same
/// place keeps the same cores rather than asking for them again. So the keeping
/// happens once, where the place is built, and this is the check every spawn
/// runs against everything the table already holds. A spawn that kept would
/// spend the machine on a component's restarts, and a probe that was refused
/// later for a different reason would have taken a reservation on its way past.
///
/// The table it is handed lives as long as the supervisor does, which is what
/// makes the test a test: a per-call table would answer every demand as if it
/// were the first, and a second hard-class component would be admitted onto
/// cores the first already holds.
///
/// **And a place that already holds a grant is not tested against it.** That is
/// the other half of the same sentence and it was missing: a spawn is either a
/// place's first occupant, in which case the demand is new and the table has to
/// be asked, or a restart, in which case the place is already holding what is
/// being asked for and `Table::admit` would find those cores in `taken` and
/// refuse `ADMISSION/NO_CORE` — against the place's own reservation, on the
/// first fault, for a hard-class manifest on a part that can grant one. What
/// tells the two apart is `Grant::answers`, which compares what the record
/// declares against what the place kept rather than trusting the place to be
/// holding the right thing: a record whose demand has changed is tested again,
/// which is R04 and is the same refusal the manifest pin makes on other
/// grounds.
///
/// The soft path was wrong in the same way and harmless at today's sizes — a
/// restart was tested against a pool its own place had already spent — and it
/// is fixed at the root instead: `f_abi::reserve` no longer charges the soft
/// class against the hard class's pinned pool at all, because that pool is not
/// what a soft component's memory comes from.
///
/// **The list.** A fourth, added when accounts stopped being one size: every
/// frame an instance is made of has a name in [`Instance::charged`], because a
/// teardown gives them back last-charged-first and a watermark can only give
/// back its top. That list is [`CHARGED_MAX`] long, so a manifest declaring more
/// than fits is refused here — `ADMISSION/MEMORY`, the same domain as an account
/// too small, because it is the same statement about a different ledger — rather
/// than part-way through a spawn at an array bound with no domain at all.
///
/// **The domain.** RFC 0005: a kind is delivered in full or the spawn is
/// refused with `ADMISSION`, and a machine that cannot supply an idle sibling
/// does not host a `private` component "with a note". This build delivers every
/// kind, and the reason is a fact about the build rather than a mechanism:
/// **no two components are ever co-resident**, because a component runs when the
/// frame hands it a core and the frame hands out one at a time. Exclusion is
/// therefore total by construction, and a domain that is total by construction
/// is delivered in full.
///
/// The clause that used to carry that — *nothing schedules one, and there is no
/// scheduler until E1-B08* — stopped being the reason twice over: `runtime.rs`
/// schedules a component and RFC 0047 schedules a driver. What has not changed
/// is the *one at a time*, and that is now the whole of the argument rather
/// than a consequence of a larger one.
///
/// *Reversal, and it is nearer than it was:* the moment two components can
/// occupy one core's siblings, this stops being a fact and becomes a check —
/// `private` needs an idle sibling and `hostile` needs a core nobody else is
/// on, and a spawn that cannot get one is refused here. The condition is a
/// frame that gives out a second core while a component holds the first, which
/// is one line in a boot path rather than a scheduler somebody has to write.
///
/// # Errors
///
/// [`Failure::Admission`], carrying the packed refusal.
fn admit(
    record: &Record,
    supervisor: &Table,
    account: &Account,
    text_pages: usize,
    reservations: &Reservations,
    kept: Option<Grant>,
) -> Result<(), Failure> {
    // A place that already holds the grant this record's demand was admitted
    // for is not asking for a second one, and testing it would refuse it its
    // own cores. `Grant::answers` compares what the record declares rather than
    // trusting the field, so a record whose demand changed is tested again.
    if !kept.is_some_and(|grant| grant.answers(&Demand::of(record))) {
        reservations.admit(&Demand::of(record)).map_err(|why| Failure::Admission(why.code()))?;
    }
    let held = supervisor.inspect(account.handle).map_err(Failure::Capability)?.extent;
    if held < record.memory_bytes {
        return Err(Failure::Admission(error::pack(error::ADMISSION, error::admission::MEMORY)));
    }
    if charges_for(record, text_pages) > CHARGED_MAX {
        return Err(Failure::Admission(error::pack(error::ADMISSION, error::admission::MEMORY)));
    }
    Ok(())
}

/// How many frames one instance of this manifest charges its account.
///
/// [`parts`] for what every instance is made of, and then whatever each need
/// declares a quantity of — which is the same arithmetic [`carve`] performs, so
/// a manifest that passes here is one [`offer`] can pay for. Powerbox needs are
/// not counted because they are not supplied at spawn, which is the rule
/// [`check_needs`] and [`offer`] already share.
///
/// Unit: frames.
fn charges_for(record: &Record, text_pages: usize) -> usize {
    let mut charges = parts(text_pages);
    for need in record.needs() {
        if need.route == route::POWERBOX {
            continue;
        }
        charges = charges.saturating_add((least_extent(need) / FRAME_SIZE) as usize);
    }
    charges
}

/// Take one frame out of the account, zero it, and answer the handle it went
/// into and the address it names.
///
/// The zeroing is not tidiness. A frame the account hands over is memory a
/// component is being given, and `mem::alloc_zeroed` states as an obligation —
/// not an aspiration — that nothing a frame's last owner wrote may reach its
/// next one. An account is refunded and re-spent across a restart, so the last
/// owner here is the *previous instance of the same place*, which is precisely
/// the boundary a restart exists to make total.
fn charge(
    supervisor: &mut Table,
    account: &Account,
    frames: &FrameAllocator,
) -> Result<(Handle, u64), Failure> {
    // `REVOKE` as well as the two that describe the memory, because the frame
    // has to be able to give this name up again when the instance it was minted
    // for is torn down — see [`Table::relinquish`], which asks for exactly that
    // right and asks for it because giving up a capability gives up its
    // descendants too. Deliberately not `DERIVE` and not `GRANT`: a frame the
    // account handed over for one instance is not something to retype further
    // or hand on.
    let minted = supervisor
        .derive(account.handle, rights::READ | rights::WRITE | rights::REVOKE, &mut backing(frames))
        .map_err(|_| Failure::Account)?;
    let object = supervisor.inspect(minted).map_err(Failure::Capability)?.object;
    let at = frames.virt(Frame::from_addr(object));
    // SAFETY: `at` is the direct-map address of a frame inside the account's
    // region, which this module allocated and handed to nobody else, and the
    // count is exactly one frame. Nothing holds a reference into it: the
    // capability naming it was minted a line ago and given to no other table.
    unsafe { core::ptr::write_bytes(at, 0, FRAME_SIZE as usize) };
    Ok((minted, object))
}

/// Put a capability in the supervisor's own table, buying a page of slots when
/// there is nowhere to put it.
///
/// # The decision `cap.rs` left to this task, taken
///
/// [`Table::grant`] does not grow, its own documentation says why, and then it
/// names who has to choose: *RFC 0008 has the frame placing capabilities into a
/// running component's table — a powerbox grant, a spawn's needs — and every one
/// of those is a grant that may find the table full. The answer is either that
/// the placing component pays out of the `Untyped` it is already spending on the
/// spawn, or that a grant into a full table is refused and the supervisor grows
/// the child first. This task does not choose, because there is no second
/// component yet to choose for.*
///
/// There is a second component now. **The choice is the second one: a grant into
/// a full table is refused, and the holder buys the page itself, out of its own
/// account.** The first would have the frame spending somebody else's account on
/// the frame's own say-so, at a moment the component being granted to may not
/// exist yet — which is the kernel reserve RFC 0008 refuses to have, arrived at
/// from the other side. Keeping the payer and the grower the same party is what
/// makes a quota a number a supervisor can predict.
///
/// What it costs, stated rather than discovered: a holder with no account of its
/// own cannot be granted anything once its table is full. That is the honest
/// shape — authority arriving from outside does not pay for somewhere to land —
/// and it is why [`SUPERVISOR_ORDER`] exists and is granted first.
///
/// Only the *supervisor's* table is grown here, never a child's. A spawn's needs
/// are bounded by [`f_abi::manifest::CAPABILITIES_MAX`] and a fresh table is
/// [`crate::cap::TABLE_SLOTS`] wide, so a child that needed a page would be a
/// child whose manifest had outgrown the schema — which is `lint-manifests`'s
/// refusal and not a page purchase.
///
/// # Errors
///
/// [`Failure::Capability`], carrying whatever the table refused: the grow when
/// nothing could pay for a page, and the grant when it could and the slot was
/// still refused.
fn grant_into(
    supervisor: &mut Table,
    frames: &FrameAllocator,
    kind: CapType,
    rights: u8,
    object: u64,
    extent: u64,
) -> Result<Handle, Failure> {
    let full = error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED);
    match supervisor.grant(kind, rights, object, extent) {
        Err(refused) if refused == full => {
            supervisor.grow(&mut backing(frames)).map_err(Failure::Capability)?;
            supervisor.grant(kind, rights, object, extent).map_err(Failure::Capability)
        }
        other => other.map_err(Failure::Capability),
    }
}

/// How many of a manifest's needs this build satisfies with a capability that
/// names no object the machine has.
///
/// The `irq` needs, and nothing else — see the [`CapType::Irq`] arm of
/// [`offer`]. Counted into [`Report::unbound`] and printed, because *the needs
/// were checked* and *the needs were met* are two different claims and a spawn
/// that reported only the first would be the third false pass of this epoch
/// wearing a fourth hat.
///
/// Unit: capabilities.
fn unbound_needs(record: &Record) -> u32 {
    record
        .needs()
        .iter()
        .filter(|need| need.route != route::POWERBOX && need.cap_type() == Some(CapType::Irq))
        .count() as u32
}

/// What a connect answered.
///
/// Three variants and not an `Option`, because RFC 0008 gives a connect three
/// outcomes and a two-valued answer is how the third comes to be invented
/// separately by the builder and by the test.
enum Answer {
    /// A channel to the occupant at this epoch.
    Channel(u32),
    /// The place is empty and the connect is waiting.
    Pending,
    /// The place is retired. Carries the packed refusal.
    Refused(i32),
}

impl core::fmt::Display for Answer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Channel(epoch) => write!(f, "a channel opened, header epoch {epoch}"),
            Self::Pending => write!(f, "the place is empty, the connect pends"),
            Self::Refused(code) => write!(f, "refused {code:#x}: the place is retired"),
        }
    }
}

/// Ask for a channel to whoever occupies a place.
///
/// `probe` says who submitted: the client under test, or the frame taking an
/// outcome on purpose. It decides nothing about the answer and everything about
/// which counter the answer lands in — [`Report::lost`] is the number gate G1's
/// sentence is about, and a frame's own probe must never inflate it. The branch
/// is the same branch either way, which is what keeps that zero a claim about
/// the client rather than about unreachable code.
fn connect(
    frames: &mut FrameAllocator,
    place: &mut Place,
    endpoint: Handle,
    deadline: u64,
    pending: &mut Option<PendingConnect>,
    report: &mut Report,
    probe: bool,
) -> Result<Answer, Failure> {
    if place.retired {
        // Not a failure of the frame, and not a failure of the boot. A client
        // told `GONE` has been *answered*, and being answered is the whole of
        // what this mechanism promises; what it does not promise is that the
        // answer is a channel.
        if probe {
            report.probed += 1;
        } else {
            report.lost += 1;
        }
        return Ok(Answer::Refused(error::pack(error::PEER, error::peer::GONE)));
    }
    let Some(occupant) = place.occupant.as_ref() else {
        // Not a failure either. RFC 0008: a connect on an empty place pends,
        // and the three outcomes are a refill, a retirement and this connect's
        // own deadline passing.
        *pending = Some(PendingConnect { place: 0, endpoint, deadline });
        return Ok(Answer::Pending);
    };
    let epoch = open_channel(frames, occupant.epoch)?;
    Ok(Answer::Channel(epoch))
}

/// Answer a pending connect whose own deadline has passed.
///
/// The third of a pending connect's three outcomes, and the one RFC 0008 asks
/// E1-B05 to add beside `peer::GONE`. It is `PEER/EMPTY`, and it is deliberately
/// not `GONE`: the place may yet be refilled by a respawn, so a client that can
/// wait longer may submit again — where a client told `GONE` would be right to
/// give up.
///
/// `now` is a tick count and not an instant, for the reason [`PEND_TICKS`]
/// gives: a deadline a boot log carries has to be the same number on an emulator
/// and on a machine.
fn expire(place: &Place, pending: &mut Option<PendingConnect>, now: u64) -> Option<i32> {
    let waiting = pending.as_ref()?;
    if place.occupant.is_some() || now < waiting.deadline {
        return None;
    }
    *pending = None;
    // A retired place is gone and is not coming back, which is a different
    // answer to a different question: a client told `EMPTY` may submit again and
    // a client told `GONE` would be right to give up.
    let code = if place.retired { error::peer::GONE } else { error::peer::EMPTY };
    Some(error::pack(error::PEER, code))
}

/// Answer a pending connect with a channel to the place's new occupant.
fn resume(
    frames: &mut FrameAllocator,
    place: &mut Place,
    pending: &mut Option<PendingConnect>,
    report: &mut Report,
) -> Result<u32, Failure> {
    let Some(waiting) = pending.take() else { return Err(Failure::Connect(0)) };
    if waiting.place != 0 || waiting.endpoint == Handle::NULL {
        return Err(Failure::Connect(0));
    }
    let occupant = place.occupant.as_ref().ok_or(Failure::WrongPlace)?;
    let epoch = open_channel(frames, occupant.epoch)?;
    report.resumed += 1;
    Ok(epoch)
}

/// Build a channel region and read back the epoch its header carries.
///
/// The epoch is *the ordinal of the occupant this channel was opened to*, which
/// is `ChannelHeader::epoch`'s second reading and the one RFC 0008 gives it: the
/// region does not survive the peer, so the field's job is to tell a
/// reconnecting client, in the first cache line of its new channel, that this is
/// not the peer it had.
fn open_channel(frames: &mut FrameAllocator, epoch: u32) -> Result<u32, Failure> {
    let frame = frames.alloc_zeroed(Order::FRAME).ok_or(Failure::NoMemory)?;
    let at = frames.virt(frame);
    // SAFETY: the frame was just allocated zeroed, is frame-aligned and is
    // `FRAME_SIZE` bytes; nothing else holds a pointer into it.
    let described =
        unsafe { Mapping::describe(at, FRAME_SIZE as u32, CONTROL_ENTRIES, epoch, 0, 0) };
    let opened = match described {
        Ok(mapping) => mapping.epoch(),
        Err(why) => {
            // SAFETY: as above, and the mapping was refused so nothing holds a
            // reference into the region.
            unsafe { frames.free(frame) };
            return Err(Failure::Ring(why));
        }
    };
    // The client end adopts the same bytes and must reach the same conclusion —
    // the check a single-ended round trip cannot make.
    // SAFETY: as above; two ends over one region hand out only atomics and
    // `UnsafeCell`s, which is what makes that sound.
    let far = unsafe { Mapping::adopt(at, FRAME_SIZE as u32, 0, 0) }.map_err(Failure::Ring)?;
    let agreed = far.epoch();
    // SAFETY: allocated here, and both mappings are past their last use.
    unsafe { frames.free(frame) };
    if agreed != opened {
        return Err(Failure::Ring(0));
    }
    Ok(opened)
}

/// End the occupant of a place, whatever caused it, and give everything back.
///
/// Uniform for a fault, an exit and a stop whose deadline passed, in the order
/// RFC 0008 fixes — and the order is fixed because a seeded run has to reproduce
/// it. Answers what it withdrew: capabilities, mappings, frames refunded, and
/// peer-gone notices posted.
///
/// # Why there is no shootdown here
///
/// Because there is nothing to shoot down. An instance's address space is never
/// in `CR3` on any core in this build — no scheduler has run one — so no core
/// holds a translation to its pages and the shootdown is the empty case.
/// `process::withdraw` is where the non-empty case runs, on every `cap=unmap`
/// boot, and it is the same call this will make when a scheduler puts an
/// instance on a core.
fn tear_down(
    frames: &mut FrameAllocator,
    place: &mut Place,
    supervisor: &mut Table,
    account: &Account,
    why: u64,
    report: &mut Report,
) -> Result<(u32, u32, u32, u32), Failure> {
    let mut withdrawn = (0, 0, 0, 0);
    if let Some(mut occupant) = place.occupant.take() {
        // 1. Revoke the table. Every slot, in slot order, and the mappings a
        //    revoked capability authorised go with the names — which for this
        //    instance is every page it had, because its whole address space is
        //    about to stop existing.
        withdrawn.0 = occupant.table.used() as u32;
        withdrawn.1 = occupant.table.capacity() as u32;
        occupant.table.clear_all();

        // 2. Tear down its channels. There are none standing here — the client's
        //    channel region is freed at the moment it is read, because there is
        //    no component to hold the far end of it yet — and the far end's next
        //    submission earning `PEER/GONE` is what `connect` answers on a
        //    retired place.

        // 3. Give up the names the supervisor minted to make the offer with.
        //    Not refunded: the memory behind them is the same memory the
        //    charges below give back, and giving it back twice is the bug the
        //    two lists exist to not have.
        for index in (0..occupant.supplies).rev() {
            let Some(handle) = occupant.supplied.get(index).copied() else { continue };
            if handle == Handle::NULL {
                continue;
            }
            supervisor.relinquish(handle).map_err(Failure::Capability)?;
        }

        // 4. Return the memory to the `Untyped` it was retyped from. What an
        //    account paid for comes back to that account, not to a global free
        //    list, which is what makes a supervisor's quota a real number after
        //    its children have lived and died.
        //
        //    Last charged first, because a watermark can only give back its top
        //    — `Table::refund` states the bound and why a general answer would
        //    be a free list per account.
        for index in (0..occupant.charges).rev() {
            let Some(handle) = occupant.charged.get(index).copied() else { continue };
            // The name goes before the memory does. A supervisor still holding
            // a `Frame` capability naming a refunded page would be holding
            // authority over memory the next instance is about to be given,
            // which is the one thing a restart may not leave behind.
            supervisor.relinquish(handle).map_err(Failure::Capability)?;
            supervisor
                .refund(account.handle, FRAME_SIZE, account.floor)
                .map_err(Failure::Capability)?;
            withdrawn.2 += 1;
        }
        // The page tables under it are the allocator's — see the module comment
        // on what is not yet charged to the account — and are returned exactly,
        // by the same list `process::reap` gives back.
        for frame in occupant.space.tables().iter().copied() {
            // SAFETY: every one of these came from this allocator in `spawn`,
            // and the address space they describe has never been in `CR3` on
            // any core, so no translation reaches them.
            unsafe { frames.free(frame) };
        }
    }

    // 5. Post peer-gone to every holder of an endpoint to the place. The
    //    supervisor is one such holder, and this is how it learns — there is no
    //    separate wait-for-child. Outside the branch above on purpose: a place
    //    being *retired* is a death of the place rather than of an occupant,
    //    and its holders are owed the news whether or not anybody was in it.
    if supervisor.note_peer_gone(place.endpoint).is_ok() {
        withdrawn.3 += 1;
    }

    // 6. Record it. Which cause, counted per place, which is where E1-P06's
    //    blast-radius number comes from — read, under RFC 0013, never
    //    delivered.
    match cause::of(why) {
        cause::FAULT => place.faults += 1,
        cause::EXIT => place.exits += 1,
        cause::STOPPED => place.stops += 1,
        _ => {}
    }
    if cause::of(why) == cause::RETIRED {
        place.retired = true;
        report.retired += 1;
    }
    Ok(withdrawn)
}

/// Publish every notice both tables owe onto the control rings their holders
/// read, and drain what arrives the way a polling point does.
///
/// # Why this posts and then drains, in rounds
///
/// Because a control ring is [`CONTROL_ENTRIES`] deep and a table owes more
/// notices than that. RFC 0008's answer is that a notice is *pending state the
/// frame publishes when there is room*, so the ring's depth bounds how much is
/// visible and never how much is true: this posts what fits, drains it, and
/// goes round again. The round count reaches the report because one round would
/// have proved nothing about the case the design is built for.
///
/// The drain is the component's half, performed here on its behalf because
/// nothing is scheduled. That is enough to show a notice arriving as a flagged
/// completion entry and is not enough to show a component acting on one — which
/// is E1-B08's, and `user/store/src/lib.rs` says so at the crate that will do
/// it.
fn publish(
    place: &mut Place,
    supervisor: &mut Table,
    ledger: &Mapping,
    report: &mut Report,
) -> Result<(), Failure> {
    if let Some(occupant) = place.occupant.as_mut() {
        let pumped = pump(&mut occupant.table, &occupant.ring, Handle::NULL)?;
        report.notices += pumped.0;
        report.collected += pumped.1;
        report.rounds += pumped.2;
        report.kinds |= pumped.3;
    }
    let pumped = pump(supervisor, ledger, Handle::NULL)?;
    report.notices += pumped.0;
    report.collected += pumped.1;
    report.rounds += pumped.2;
    report.kinds |= pumped.3;
    Ok(())
}

/// Post what fits, drain what arrived, and go round until nothing is owed.
///
/// Answers what was posted, what came back off the ring, and how many rounds it
/// took. The two counts are kept apart because *published* and *delivered* are
/// the two halves R05 is about, and a single number could not tell a frame that
/// posted nothing from one whose peer read nothing.
fn pump(
    table: &mut Table,
    ring: &Mapping,
    control: Handle,
) -> Result<(u32, u32, u32, u8), Failure> {
    let poster = Poster::new(ring.completions()).ok_or_else(|| {
        Failure::Notice(error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER))
    })?;
    let collector = Collector::new(ring.completions()).ok_or_else(|| {
        Failure::Notice(error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER))
    })?;
    let (mut posted, mut taken, mut rounds) = (0, 0, 0);
    let mut kinds = 0u8;
    loop {
        let mut published = 0;
        while poster.free().map_err(ring_error)? > 0 {
            let Some(entry) = next_notice(table, control) else { break };
            // R04: a kind this build does not define is not published. A frame
            // that produced one would otherwise leave a component an entry it
            // is not permitted to skip and cannot name.
            if !f_abi::control::is_notice(&entry) || !notice::known(entry.result) {
                return Err(Failure::Notice(entry.result));
            }
            poster.post(entry).map_err(ring_error)?;
            published += 1;
        }
        // The polling point.
        while let Some(entry) = collector.take().map_err(ring_error)? {
            if !f_abi::control::is_notice(&entry) || !notice::known(entry.result) {
                return Err(Failure::Notice(entry.result));
            }
            // Which kind arrived, read off the entry a component would read it
            // off rather than off the state it was published from: what is
            // under test here is *delivery*, and a tally taken on the posting
            // side would be counting the frame's intentions.
            kinds |= 1u8 << (entry.result.clamp(1, 7) - 1);
            taken += 1;
        }
        if published == 0 {
            break;
        }
        posted += published;
        rounds += 1;
    }
    Ok((posted, taken, rounds, kinds))
}

/// The next notice a table owes, in the order `f_abi::control::ORDER` fixes.
///
/// Slots ascending, then the stop, then the two grades. No timestamp, because
/// the boot log is a fixture and a stamp in it would be a different number on
/// every run; `Cqe::timestamp` is what a component reads to know *when*, and a
/// component drains its own ring.
///
/// The third phase — reclaim, per core — is absent rather than skipped: it is
/// bounded by the cores in a component's allocation, and nothing here holds an
/// allocation because nothing here is scheduled. RFC 0008 puts that state beside
/// the allocation for exactly this reason, and E1-B08 is where an allocation
/// first exists. The three calls below are three rather than one so that the
/// reclaim phase splices in at the position `ORDER` fixes.
fn next_notice(table: &mut Table, control: Handle) -> Option<Cqe> {
    if let Some(entry) = table.next_slot_notice(0) {
        return Some(entry);
    }
    if let Some(entry) = table.next_stop_notice(control, 0) {
        return Some(entry);
    }
    table.next_grade_notice(0)
}

/// A ring's own refusal, as a packed error.
///
/// Three shapes, and each is a different thing having gone wrong, so they are
/// not collapsed into one: a full ring is a quota, a corrupt one is a header
/// nobody can believe, and a moved epoch is a peer that restarted underneath.
fn ring_error(why: RingError) -> Failure {
    Failure::Notice(match why {
        RingError::Full => error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
        RingError::Corrupt => error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
        RingError::EpochChanged => error::pack(error::PEER, error::peer::EPOCH_CHANGED),
    })
}

/// The direct map, as a table's backing.
///
/// # Safety obligations, discharged at every call site by construction
///
/// Every frame a table here charges for is inside the account's region, which
/// this module allocated and handed to nobody else, and `frames` is rebound onto
/// the address space that is live for the whole of the boot.
fn backing(frames: &FrameAllocator) -> Direct<'_> {
    // SAFETY: as the doc comment above. The account is one contiguous region
    // allocated by `demonstrate` and freed by it, no other table holds an
    // `Untyped` naming any of it, and the caller of `demonstrate` guarantees
    // `frames` is rebound onto the live direct map.
    unsafe { Direct::new(frames) }
}

/// A manifest name, for a log line.
///
/// A wrapper rather than a `str`, because a name is bytes from an untrusted
/// record and this kernel does not turn untrusted bytes into `str` to print
/// them. Every byte outside the alphabet the record already refuses would print
/// as `?`, which cannot happen and costs nothing to be sure of.
struct Name<'a>(&'a [u8]);

impl core::fmt::Display for Name<'_> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        for byte in self.0 {
            let shown = if byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-' {
                *byte as char
            } else {
                '?'
            };
            core::fmt::Write::write_char(f, shown)?;
        }
        Ok(())
    }
}
