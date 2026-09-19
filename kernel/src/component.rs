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
//! **All three of the things that were missing are here now, and the restart
//! policy has left this file.** What this paragraph used to say is that there is
//! no supervisor component, in three bullets; `user/supervisor` is that
//! component, and the boot walks all three:
//!
//! - `f_abi::control::op::SPAWN` is implemented *and submitted*. The frame holds
//!   one place open ([`held_open`]), schedules the supervisor on a worker core,
//!   and answers the entry the supervisor puts on its own control ring — so a
//!   boot contains a component that another component spawned. The `held` and
//!   `supervised` lines are where a reader meets it.
//! - **A supervisor is told its occupant died**, by RFC 0076: the frame posts
//!   `notice::PEER_GONE` into the slot of the endpoint the supervisor holds for
//!   that place, pumps it onto the supervisor's ring, and *then* hands over a
//!   core. Being told happens between runs rather than during one, which is what
//!   lets it happen at all without the boot processor writing a running core's
//!   capability table. [`consult`] is the whole sequence and carries the
//!   argument.
//! - A supervisor is a component, so it needs a place, an account and a
//!   manifest, and it has all three. What it does **not** yet have is an
//!   `Untyped` of its own standing in for [`PLACES_MAX`] and
//!   [`SUPERVISOR_ORDER`]; RFC 0044 names that as one deviation with the other
//!   two, and it is the one still outstanding.
//!
//! So the decision is `f_supervisor::policy`'s, above the frame, and this file
//! no longer contains it. What is left here is [`Budget`] — two numbers the
//! frame stores between consultations and never reads — and the mechanism a
//! verdict is performed by. RFC 0008 asked for exactly that split and
//! `cargo xtask lint-owed` held the sentence for three epochs; the day the call
//! left, it went red and said so.
//!
//! **Two of the three fates a place has now come back across the boundary and
//! one does not.** A restart does: the supervisor decides it and submits the
//! spawn. A retirement does not — there is no opcode for *end this place* as
//! opposed to *end its occupant* — so the boot's retirement is still the frame's
//! scripted act, with the verdict's arithmetic tested in `f_supervisor::policy`
//! rather than driven here. Closing that needs four deaths in a row and is the
//! increment after this one.
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
use f_ring::{Collector, Consumer, Mapping, Poster, RingError};

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

/// The manifest name of the one component the frame starts itself.
///
/// A name and not a position, because the loader's order is the loader's. RFC
/// 0073's bootstrap argument is what makes this one component rather than none
/// or all of them: somebody has to be first, and a frame that starts exactly one
/// and then answers that one's ring has a smaller privileged surface than a
/// frame that starts every component there is.
const SUPERVISOR: &[u8] = b"supervisor";

/// The manifest name of the one component the frame does **not** start.
///
/// Its place is built, admitted and left empty for the supervisor to fill, which
/// is RFC 0008's *restart is the supervisor's act* made into something a boot
/// does rather than something a document says. [`held_open`] argues why it is
/// this component.
const SUPERVISED: &[u8] = b"virtio-gpu";

/// The rate the core running an occupant arms its own timer at. Unit: hertz.
///
/// The same thousand the rest of this kernel uses, so that a component that
/// looked at its own timer would not find a different clock depending on which
/// path started it.
const OCCUPANT_HZ: u32 = 1000;

/// How many ticks that timer asks for. Unit: timer ticks.
///
/// A bound rather than a schedule, exactly as `RuntimePlan::target` is: the
/// occupant ends itself long before this, and what this stops is a component
/// that never does.
const OCCUPANT_TICKS: u64 = 64;

/// How long the boot processor waits for the core to report finished.
/// Unit: microseconds.
///
/// Generous on purpose. This is not a measurement and nothing is timed against
/// it — it is the bound past which *a core never answered* is a better
/// conclusion than *keep waiting*, and a boot that hit it would be a red boot
/// rather than a slow one.
const OCCUPANT_MICROS: u64 = 500_000;

/// How many places this build's supervisor can hold.
///
/// **Five, and the fifth is the supervisor itself.** It was four — two component
/// files with room for two more — and `user/supervisor` is the fifth file, so
/// the bound moved to fit it rather than the file being left out to fit the
/// bound. A build that carried more component files than places is the drift
/// this number exists to make impossible, and the direction of the fix is always
/// this one.
///
/// That the supervisor occupies one of its own places is the bootstrap circle,
/// and it is placed rather than removed: somebody is first, and what this build
/// chooses is that the frame performs exactly one spawn and every other spawn
/// moves above it. When a supervisor is a component *that runs*, this bound is
/// its `Untyped` rather than a constant — RFC 0044 — which is the direction
/// everything else in this tree has already gone and is what increment 7 of
/// `intent/0005-the-datapath/plan-e1-b05.md` pays.
///
/// It is also the bound on [`modules`], and the two being one number is the
/// point: the loader hands over component files, and every one of them gets a
/// place. A build that carried more component files than places would be a
/// build where the boot half of RFC 0035's pair silently covered less than the
/// workload half, which is exactly the drift `JOIN_GAP` was built to make
/// visible.
const PLACES_MAX: usize = 6;

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
    /// A component's state tree could not be published, or could not be read
    /// back once it had been. Carries the packed refusal.
    ///
    /// Always a bug in the frame and never in a manifest: `Record::read` has
    /// already judged the declaration and [`admit`] has already refused an
    /// empty one, so what is left is this build writing bytes it cannot read.
    /// It fails the boot for the reason every other variant here does — a
    /// component half-built is worse than no component, and one nobody can read
    /// is exactly what RFC 0013 says this mechanism exists to prevent.
    StateTree(i32),
    /// The client the frame ran against a serving occupant refused. Carries
    /// that client's own message, because the client is the one that knows what
    /// it was doing — this file has never submitted a block request and would be
    /// paraphrasing.
    ///
    /// Distinct from [`Self::NoAnswer`] on purpose: the occupant answered, and
    /// what went wrong is on the near side of the boundary. A boot that
    /// collapsed the two would report a wedged driver for a client that could
    /// not register a buffer.
    Datapath(&'static str),
    /// A core was handed an occupant to run and did not report finished inside
    /// the bound.
    ///
    /// Distinct from every variant above, all of which are the frame refusing
    /// something before a core was involved. This one means the frame committed
    /// — a job was published and a core was told — and then heard nothing, which
    /// is the one failure here that leaves a core in a state the boot processor
    /// cannot describe.
    NoAnswer,
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
            Self::StateTree(_) => "a component's state tree could not be published or read back",
            Self::Datapath(why) => why,
            Self::NoAnswer => "a core was given a place's occupant to run and never reported back",
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
    // SAFETY: the caller's guarantee, passed down.
    let (found, count, _) = unsafe { generations(boot) };
    (found, count)
}

/// Every component file the loader placed, split into **places and their
/// successors**.
///
/// # Why a second module with one name is not a second place
///
/// Because a place is a name and an account and an endpoint, and RFC 0041 says
/// it survives its occupant. Two builds of one component are two *occupants* a
/// place may have, one after the other — which is the whole of what a generation
/// swap is — and giving the second one a place of its own would produce two
/// drivers for one device, each with its own clients, neither of them the other's
/// successor.
///
/// So the first module carrying a given `Record::name` is that component's
/// place, and any later module carrying the same name is a *successor*: a
/// generation the frame has been handed and has not installed. `E2-B06` is what
/// installs one.
///
/// Grouped by the manifest's declared name and not by its content address,
/// deliberately. Two generations of one component differ in content by
/// construction — that is what makes them two — so a content address is exactly
/// the wrong key. The name is the thing they share, and `cargo xtask
/// lint-manifests` already refuses two manifests claiming one device, which is
/// the confusion this could otherwise create.
///
/// # Safety
///
/// As [`modules`].
unsafe fn generations(
    boot: &BootInfo,
) -> ([&'static [u8]; PLACES_MAX], usize, [&'static [u8]; PLACES_MAX]) {
    let mut found: [&'static [u8]; PLACES_MAX] = [&[]; PLACES_MAX];
    let mut next: [&'static [u8]; PLACES_MAX] = [&[]; PLACES_MAX];
    let mut count = 0;
    for module in boot.modules() {
        // **Not `break` on a full list**, and the difference cost a boot. A
        // successor arrives *after* the places it succeeds — it is the last
        // module on the list — so a loop that stopped as soon as it had
        // `PLACES_MAX` places would stop exactly one module before the only
        // module a swap needs. What is full is the *place* list, and that is
        // tested where a place would be added.
        // SAFETY: the caller's guarantee, and every module is in the reserved
        // list — see `main::reserved_ranges` — so nothing else owns these bytes.
        let bytes = unsafe { module.bytes() };
        let Ok(record) = Record::read(bytes) else { continue };
        let label = record.label();
        // A name already taken is a successor for that place, and is kept
        // beside it rather than after it: the index is the place's, so a
        // demonstration holding a place index has the module that would
        // replace its occupant without searching for it.
        let seen = (0..count).find(|index| {
            found
                .get(*index)
                .and_then(|module| Record::read(module).ok())
                .is_some_and(|held| held.label() == label)
        });
        if let Some(index) = seen {
            if let Some(slot) = next.get_mut(index) {
                *slot = bytes;
            }
            continue;
        }
        if count == PLACES_MAX {
            continue;
        }
        if let Some(slot) = found.get_mut(count) {
            *slot = bytes;
            count += 1;
        }
    }
    (found, count, next)
}

/// The two numbers a supervisor's restart policy counts with, stored by the
/// frame and interpreted by nobody here.
///
/// # Why the frame holds these and does not read them
///
/// RFC 0008 moved the decision above the frame and RFC 0076 is what finally
/// made that possible: a supervisor is *run* when the frame has something to
/// tell it, so nothing in a supervisor's image survives between two of its
/// runs — there are no writable statics in a component, and the core clears an
/// occupant's table on the way out. Its tally therefore has to live somewhere
/// that outlives an instance, and the only such place that already exists is
/// the [`Place`].
///
/// So these are **bytes the frame copies**. They go out to the supervisor on
/// its board, come back changed, and are stored again. Nothing in `kernel/`
/// compares them, adds to them, or branches on them; `user/supervisor`'s
/// `policy::Budget` is the type that means something, and this is its
/// resting place.
///
/// RFC 0076 records this as the seam where somebody will one day argue the
/// frame still holds the policy. The answer is this doc comment and
/// `cargo xtask lint-owed`'s absence of a row: the day the frame reads one of
/// these, the objection lands.
#[derive(Clone, Copy, Debug, Default)]
struct Budget {
    /// Restarts inside the window, as the supervisor last left it.
    /// Unit: restarts.
    used: u32,
    /// When that window opened, in the logical ticks the frame stamps a
    /// consultation with. Unit: timer ticks.
    opened: u64,
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
    /// Where its published state tree lives, as a kernel address. Unit: bytes.
    tree: u64,
    /// The same frame's physical address, which is what a mount word carries
    /// and what a grant would need. Unit: bytes, physical.
    tree_physical: u64,
    /// How many nodes it publishes, out of its manifest's declaration.
    /// Unit: nodes. Never zero — a spawn that would have made it zero was
    /// refused `ADMISSION/NO_STATE_TREE` before this instance existed.
    tree_nodes: u32,
    /// Where its heap is, as a kernel address, or zero for a component that
    /// declared no `heap` need.
    ///
    /// Kept so the frame can read the region's own prologue after the component
    /// has run — which is how a boot says an allocation happened, rather than
    /// taking the component's word for it. Unit: bytes.
    heap: u64,
    /// Where its board is, as a kernel address, or zero for a component that
    /// declared no `board` need.
    ///
    /// Written by the frame before the first instruction and read by it after
    /// the last one, which is the two halves `f_supervisor::routing` describes.
    /// A kernel address and not the component's, because the frame reaches it
    /// through the direct map and never through the component's page tables.
    /// Unit: bytes.
    board: u64,
    /// Where its data ring is, as a kernel address, or zero for a component
    /// that declared no `data` need.
    ///
    /// The frame's end over these bytes is **not** held here, and that is the
    /// difference between this field and `ring` above. A control ring has the
    /// frame as its only producer for the life of the instance, so the frame
    /// keeps its end. A data ring's client is whoever the boot gives this
    /// component, which today is the frame and tomorrow is not — so the address
    /// is kept and the end is adopted by the client at the moment there is one.
    /// Unit: bytes.
    data: u64,
    /// The snapshot the frame read back off that tree the moment it published
    /// it, through `f_abi::state::Reader` and not from what it had just
    /// written.
    ///
    /// Kept rather than recomputed, because it is the *first* reading and the
    /// only one taken before the component could have touched a word — so a
    /// later reading that differs is the component having done something, which
    /// is what a reader wants to know, and a later reading that agrees on a
    /// component which has run is a tree nothing is writing into.
    /// Unit: none — a hash.
    tree_snapshot: u64,
    /// The first capability in the occupant's **own** table.
    ///
    /// The word a component is entered with carries a selector and this handle
    /// — `f_abi::door::Entry` — so that its first act can be a capability call
    /// rather than a guess about which slot it was given. It is the manifest's
    /// first non-powerbox need, which for every component in this tree is the
    /// account it was made out of.
    ///
    /// `Handle::NULL` for a component whose every need was optional and
    /// unsupplied, which is a component with nothing to be told. Recorded rather
    /// than recomputed because the table is the child's and the frame does not
    /// hold a second copy of it.
    first: Handle,
}

/// What every instance is made of besides its text: a stack, a control ring and
/// a state tree.
///
/// It was three, with text as the third, and text stopped being one frame the
/// day a component's image stopped fitting in one page — RFC 0047. So the fixed
/// part is the variable part's complement, and the two are added by [`parts`]
/// rather than by each caller, because the number appears in three places: what
/// [`charges_for`] predicts, what [`admit`] refuses on, and what [`spawn`]
/// actually takes. Three copies of one sum is how a manifest gets admitted for
/// a footprint it then overruns.
///
/// **Three since RFC 0065**, and the third is the state tree. It is charged to
/// the account like everything else an instance is made of, which is the point
/// rather than an implementation detail: a tree the frame paid for out of
/// something it kept back would be observability the supervisor's quota does
/// not bound, and that is the shape of every metrics subsystem that ends up
/// switched off. Every manifest in this tree already names it — *address space,
/// page tables, text, stack, control ring, state tree, capability table* — so
/// what changed is that the sentence is now a frame.
/// The stack is [`crate::process::SPAWN_STACK_PAGES`] of these and the other
/// two are the control ring and the state tree, so this moves when the stack
/// does. It was the literal 3 while the stack was one page, which is the form
/// that goes quietly wrong: a shape that grew a page and left the count alone
/// would charge for less than it mapped, and the frame would hand back fewer
/// frames than it took at every teardown.
const FIXED_PARTS: usize = crate::process::SPAWN_STACK_PAGES + 2;

/// The need a component declares when it wants a heap.
///
/// The frame maps this one rather than only granting it, which is why the name
/// is load-bearing rather than a label — see the mapping in `spawn`. A component
/// that declares no such need gets no heap and no mapping, and its image is the
/// same bytes it was.
const NEED_HEAP: &[u8] = b"heap";

/// The need a supervisor declares when it wants to be told what it may do.
///
/// The frame maps this one too, and for the heap's reason one step earlier: the
/// first thing a supervisor does is read it, so there is no moment at which it
/// could have asked. `f_supervisor::routing` is the layout; this is the name the
/// manifest calls it by.
///
/// One page, refused at any other size where it is mapped. A board is a fixed
/// layout and a component that asked for two pages of one would be a component
/// disagreeing with the crate that reads it.
const NEED_BOARD: &[u8] = b"board";

/// The need a component declares when it serves a client on a ring of its own.
///
/// Mapped rather than only granted, for the board's reason one step earlier and
/// at the next step of the same story: the frame writes the header and the
/// component adopts the server end over it, and a component that forbids
/// `unsafe` cannot map a page for itself. `f_ring::adopt`, RFC 0037.
///
/// One page, refused at any other size. It is the ring's *memory*; how deep the
/// ring is and what it speaks are the manifest's `[[ring]]` table, which the
/// frame does not read here.
const NEED_DATA: &[u8] = b"data";

/// Does this need carry `wanted` as its whole name?
///
/// The field is NUL-padded to a fixed width, so a prefix match on its own would
/// make `heap` and `heaps` the same need. What follows the prefix has to be the
/// padding, which is the half a `starts_with` would have got wrong quietly.
fn named(need: &f_abi::manifest::Need, wanted: &[u8]) -> bool {
    need.name.len() >= wanted.len()
        && &need.name[..wanted.len()] == wanted
        && match need.name.get(wanted.len()) {
            Some(byte) => *byte == 0,
            None => true,
        }
}

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
    /// Which of the frame's mount nodes this place publishes its occupant's
    /// tree into. Unit: none — a slot ordinal, below [`PLACES_MAX`].
    ///
    /// On the place and not on the occupant, and it is the same lifetime
    /// argument [`Place::reservation`] makes below: a mount is a slot in the
    /// frame's own tree, a place survives its occupant, and a reader watching
    /// one node across a restart is watching *this place* refill rather than
    /// two unrelated components that happened to be given one address.
    slot: usize,
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
    budget: Budget,
    /// Deaths by cause, for the boot log and for the state tree E1-P06 reads
    /// its blast-radius number out of.
    faults: u32,
    /// Deaths by exit.
    exits: u32,
    /// Deaths by a stop whose deadline passed.
    stops: u32,
    /// Restarts performed.
    restarts: u32,
    /// Which generation of its occupant this place is delivering to, or
    /// [`f_abi::swap::PAUSED`] while a swap is in flight.
    ///
    /// **RFC 0016's fifth cross-core word**, and `abi/src/swap.rs` carries the
    /// argument rather than restating it here: one `Release` store when a
    /// generation is committed, one `Acquire` load at every delivery, and a
    /// paused word *pends* a submission rather than refusing it — which is the
    /// same mechanism RFC 0008's pending connect already rests on.
    ///
    /// On the place and not on the occupant, for [`Place::reservation`]'s
    /// reason: a client that submitted across a swap is submitting to the
    /// *place*, and the word is what makes the gap between two occupants a wait
    /// rather than a refusal.
    ///
    /// Its ordering is unobservable on x86-64 by construction, which is why
    /// `ring/tests/litmus.rs` and the AArch64 job are where it means anything.
    routing: f_abi::swap::Routing,
    /// Swaps this place has completed. Unit: swaps.
    swaps: u32,
    /// Swaps this place began and abandoned. **Never summed with
    /// [`Place::restarts`]**, which RFC 0012 requires: an abandoned swap leaves
    /// the occupant it already had, and charging one to a restart budget would
    /// let failed updates retire a healthy component. Unit: swaps.
    abandoned: u32,
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
    /// Generation swaps that reached `commit`. **Never summed with restarts**,
    /// which RFC 0012 requires: a restart gives a client a component that has
    /// never heard of it, and a swap hands the successor the history its
    /// predecessor lived. Unit: swaps.
    pub swaps: u32,
    /// Needs satisfied with a capability of the declared type that names no
    /// object this machine has. Unit: capabilities.
    ///
    /// **The gap this change leaves, as a number rather than a paragraph.**
    /// Today it is the `irq` need each driver manifest declares: nothing in this
    /// build routes a device interrupt to a component, so what the spawn
    /// supplies is a capability of the right type, carrying the right rights,
    /// naming no vector. It is enough for the lifecycle — the spawn is real, the
    /// account is real, the table is real — and it is not enough for the
    /// component to wait on the device. [`unbound_needs`] is the arithmetic and
    /// [`offer`] is the reason.
    ///
    /// **Who closes it is an open question, and saying so is better than the
    /// answer this comment used to give.** It named `E1-B09`, which is the
    /// *user-interrupt doorbell between two ends of a ring* and says nothing
    /// about device interrupts. That matters more than a misfiled pointer:
    /// `E1-B09` is owed to silicon under RFC 0093 — QEMU implements no part of
    /// UINTR — so attributing this to it makes a thing that may well be
    /// buildable here read as blocked on a purchase. `DATAPATH_GAP` carried the
    /// same class of error about three of `E1-P10`'s four numbers and it cost
    /// two corrections to unwind.
    ///
    /// What is certain is the sentence above: no vector reaches a component. A
    /// kernel-side delivery — the frame taking the interrupt and posting a
    /// notice on the control ring the component already drives — needs no
    /// hardware this machine lacks, and is the shape `E0-B15`'s `Path::KernelIpi`
    /// already uses for doorbells. Whether that is the right answer, and which
    /// task owns it, is for whoever next reads RFC 0024 against this field
    /// rather than for this comment to assert.
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
    /// completion entries. Counted apart from [`Report::notices`] because
    /// *published* and *delivered* are the two halves R05 is about.
    /// Unit: notices.
    pub collected: u32,
    /// Notices posted onto a ring whose holder drains it **itself**, and
    /// therefore not collected here.
    ///
    /// # Why this is a second bucket and not a weaker check
    ///
    /// The invariant used to be `collected == notices`: every notice the frame
    /// published, it drained back, because nothing was scheduled and the frame
    /// performed the component's half on its behalf. `user/supervisor` performs
    /// its own half now — that is what being told means (RFC 0076) — so the
    /// frame must post to its ring and leave it alone.
    ///
    /// Relaxing the check to `collected <= notices` would have made a boot that
    /// published nothing and a boot that lost everything look the same. Instead
    /// the two are still required to add up: `collected + handed == notices`,
    /// and what changed is that a notice now has two honest destinations rather
    /// than one. Unit: notices.
    pub handed: u32,
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
    /// State trees mounted under the frame's root and read back through it.
    /// Unit: trees.
    ///
    /// **Equal to [`Report::spawns`] on any boot that finished**, and that is
    /// the number E1-B15's first clause is: every component the supervisor
    /// started published a tree. A spawn that had produced no tree would be a
    /// spawn `admit` should have refused, so a gap between these two is a hole
    /// in the refusal rather than a component that was merely quiet.
    pub mounted: u32,
    /// Nodes across every tree mounted, summed as they were mounted.
    /// Unit: nodes.
    ///
    /// Beside the count for the reason `BLK_BYTES` sits beside `BLK_COPIES`:
    /// *four trees were mounted* is a claim about a mechanism and says nothing
    /// about whether any of them had anything in it.
    pub nodes: u32,
    /// The most any component had allocated at once, read out of its heap's own
    /// prologue after it had run. Unit: bytes.
    ///
    /// A **peak** and not a live figure, because a component that allocated and
    /// freed has a live figure of zero and is exactly the case this is evidence
    /// for. Zero here means no component allocated anything, which on a tree
    /// where one declares a `heap` need is a finding rather than a default.
    ///
    /// **It stopped being zero when an occupant was first handed a core**, and
    /// the two things that had to be true for that are worth keeping together.
    /// Until `E1-B05` a component spawned into a place was never scheduled, so
    /// no component with a `heap` need had ever executed (RFC 0075). And once
    /// one did, it died on its first allocation: `user/init/link.ld` pointed
    /// `alloc`'s shim marker at the image's first byte, which this toolchain
    /// *calls* rather than reads, so the allocator re-entered `component::start`
    /// until the stack met its guard page. Both components that declare a heap
    /// carried that, and neither could have shown it before one of them ran.
    ///
    /// `HEAP_GAP` in `xtask` was the constant that declared the zero and the
    /// check that refused the change. It is gone, paid; the check is inverted
    /// and a zero is now the failure.
    pub heap_peak: u32,
    /// Whether any component was ever refused an allocation for want of room.
    ///
    /// Read from the same prologue, and never cleared once set: what a reader
    /// wants is *did this ever fail*, and a component that recovered from one
    /// refusal and failed later was not fine.
    pub heap_starved: bool,
    /// How large the largest heap the frame described was. Unit: bytes.
    pub heap_bytes: u32,
    /// Spawns refused because the manifest declared no state tree.
    /// Unit: refusals.
    ///
    /// Non-zero on every boot, on purpose. RFC 0065's refusal is what makes the
    /// clause above stay true, and a refusal nobody has watched happen is
    /// indistinguishable from one that cannot — the same argument
    /// `state::node::MEMORY_FORCED` makes about a counter.
    pub mute: u32,
    /// How many times a place's occupant was handed a core. Unit: handings.
    ///
    /// **Handings and not components**, which the unit used to get wrong: the
    /// supervisor is consulted again on every restart and each consultation is
    /// another core given to the same occupant, so this has counted more than
    /// one component's worth since the restart path existed.
    ///
    /// A count rather than a flag because the number this should be is a
    /// decision — RFC 0073's bootstrap argument says the frame *starts* exactly
    /// one component — and a flag would record that it happened without
    /// recording how often. The set of components in it is the supervisor on
    /// every boot with a second core, and on the one boot that supplies a place
    /// with a device window, that place's occupant as well.
    pub scheduled: u32,
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
/// whose header carries a real epoch.
///
/// **The sentence that used to stand here said nothing here is scheduled,
/// because there is no scheduler until `E1-B08`.** Both halves have since
/// stopped being true. `E1-B08` landed, and RFC 0075 joined the two halves this
/// tree had been carrying separately, so one occupant — the supervisor's — is
/// handed a core below. What is still not here is a *load*: one component
/// running once is not several running under contention, and that is the
/// sentence still separating this from gate G1.
///
/// The log it prints is a fixture: every number in it is a count rather than a
/// duration, so two runs of one commit produce the same bytes on machines two
/// orders of magnitude apart in speed. That is why the backoff below is stated
/// in ticks and never in milliseconds, and why nothing here reads a clock.
///
/// `worker` is the core an occupant may be run on and that core's TSC rate, or
/// `None` on a machine that started no second core. `None` is a boot that spawns
/// every place and schedules none, which is what every boot before RFC 0075 was
/// — so the path is exercised where there is a core for it and the boot is not
/// failed where there is not.
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
#[expect(
    clippy::too_many_arguments,
    reason = "`fill` one function down makes this argument at length and this is that list \
              plus the five the demonstration itself needs: the core an occupant may be \
              given, what this boot found that a place cannot carve for itself, what it \
              found that only the frame can tell that occupant, and the client that occupant \
              is to be given something to answer, and the generation this machine was asked to be. \
              Bundling them \
              would be a type that exists so a lint passes, which `runtime::demonstrate` \
              already declined for this reason"
)]
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    boot: &BootInfo,
    now: u64,
    tree: &crate::state::Tree,
    worker: Option<(usize, u64)>,
    supplied: &[Supplied],
    routing: &[(u32, u64)],
    datapath: Option<&mut dyn Datapath>,
    generation: Option<Generation>,
) -> Result<Report, Failure> {
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let (modules, count, successors) = unsafe { generations(boot) };
    let module = *modules.first().filter(|_| count > 0).ok_or(Failure::NoComponent)?;
    let record = Record::read(module).map_err(Failure::Manifest)?;

    let before = frames.free_count();
    // What a *client* keeps, as opposed to what the demonstration spends, and it
    // is subtracted rather than tolerated. A remapping unit holds a bus's
    // context table for the life of the machine — `Unit::detach` says why it is
    // not freed and is right — so a free-count comparison alone would report the
    // first attach on a bus as a leak, every time, and be ignored within a week.
    // Two numbers rather than a tolerance: a check with slack in it is a check
    // that stops noticing the first frame. `blk::demonstrate` makes the same
    // subtraction one file over, for the same reason and in the same words.
    let mut retained = 0u64;
    let mut report = Report { components: count, places: count, ..Report::default() };
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
        // The scripted place, and slot zero of the frame's mount nodes.
        slot: 0,
        manifest: ContentId::of(module),
        module,
        endpoint,
        epoch: 0,
        occupant: None,
        retired: false,
        reservation: None,
        budget: Budget::default(),
        routing: f_abi::swap::Routing::new(f_abi::swap::PAUSED),
        swaps: 0,
        abandoned: 0,
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
    // Through the ring server, which is RFC 0073's subject. The entry is built
    // here rather than submitted by a component because there is no supervisor
    // component yet; what this buys is that the route exists and the boot walks
    // it before anything depends on it.
    let spawned = {
        // Read before the server borrows the place, because the completion is
        // checked against it after.
        let expected_endpoint = u64::from(place.endpoint.bits());
        // The frame is its own submitter here, so the two tables are one table —
        // and this is the line that says so out loud rather than by sharing a
        // field. A copy and not a borrow, because the server needs the other
        // half exclusively; it is the same bytes, taken one statement earlier.
        let submitted_from = supervisor;
        let mut serving = Serving {
            place: &mut place,
            submitter: &submitted_from,
            supervisor: &mut supervisor,
            frames,
            account: &account,
            kernel,
            features,
            reservations: &reservations,
            spawned: (0, 0, 0),
            answered: 0,
        };
        let entry = f_abi::Sqe {
            opcode: f_abi::control::op::SPAWN,
            cap: account.handle.bits(),
            ext: [ContentId::of(module).bits(), 0],
            ..f_abi::Sqe::ZERO
        };
        // An `ext` naming a manifest this place does not hold is refused, and it
        // is checked here rather than trusted to `spawn`'s own check for the
        // reason the arm's comment gives: the two stop being the same question
        // the moment a generation swap puts a newer image behind a place.
        if serving.execute(&f_abi::Sqe { ext: [entry.ext[0] ^ 1, 0], ..entry }).result == 0 {
            return Err(Failure::WrongPlace);
        }
        let answer = serving.execute(&entry);
        if answer.result != 0 {
            return Err(Failure::Admission(answer.result));
        }
        if answer.ext != expected_endpoint {
            // `abi/src/control.rs` says a spawn completes with an endpoint to
            // the place. A completion that carried something else would be a
            // submitter holding a handle to the wrong thing.
            return Err(Failure::WrongPlace);
        }
        serving.spawned
    };
    report.spawns += 1;
    report.unbound += unbound_needs(record);
    spawned_line(record, &place, spawned);
    mounted_line(record, &place, mount(tree, &place, &mut report)?);

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
    //
    // **Which place is held open is decided before the loop and not inside it**,
    // and that is not tidiness. The decision needs to know that a supervisor
    // exists at all, and the loader's order is not this build's to choose:
    // `docs/second-boot-outside-qemu.md` records the boot where places 2 and 3
    // swapped on hardware because `tools/f-on-metal.sh` sorts what `xtask` hand
    // orders. A loop that held a place open the moment it met one and hoped the
    // supervisor came later would be a boot that depends on that sort.
    let held = held_open(&modules, count, worker.is_some());
    // The supervisor, its core, and where it goes back — held out of `extras`
    // from the join below until the scripted place has finished dying, because
    // it is consulted about every one of those deaths. `None` on a build with no
    // supervisor or no second core, where the frame decides nothing and
    // `restart_line` says so rather than a verdict appearing from nowhere.
    let mut consulting: Option<(usize, usize, u64, Extra)> = None;
    for index in 1..count {
        let Some(module) = modules.get(index).copied() else { break };
        let occupant =
            if held == Some(index) { Occupant::ByTheSupervisor } else { Occupant::ByTheFrame };
        // SAFETY: the caller's guarantee, passed down; `module` came out of
        // `modules` above and is one of the boot modules the direct map covers.
        let mut extra = unsafe {
            fill(
                frames,
                kernel,
                features,
                &mut supervisor,
                &mut reservations,
                module,
                &mut report,
                tree,
                index,
                occupant,
                // Handed the whole list rather than this component's share of
                // it. Each entry names the component it is for, so the filter
                // lives where the need is matched and there is no second place
                // for the two to disagree about which window belongs to whom.
                supplied,
            )
        }?;
        // This place's own polling point, not the first place's: what a spawn
        // owes is owed to *its* occupant, and pumping the wrong ring would leave
        // a component holding capabilities it was never told about while the
        // counters said every notice had been delivered.
        publish(&mut extra.place, &mut supervisor, &ledger_ring, &mut report)?;

        let Some(slot) = extras.get_mut(index) else { return Err(Failure::WrongPlace) };
        *slot = Some(extra);
    }

    // ---------------------------------------------------------------- the join
    //
    // `kernel/src/runtime.rs` has described this since RFC 0033: *there, a
    // component is spawned into a place and never scheduled; here, a component
    // is scheduled and never spawned into one.* RFC 0075 ended that half; this
    // is the other one, and it is the act E1-B05 is named after — **the
    // supervisor spawns, and the frame answers.**
    //
    // After the loop rather than inside it, so that every place the supervisor
    // may fill exists before it is given a core. Inside the loop this depended
    // on the loader having placed the supervisor's module after the one it
    // fills, which no part of this tree guarantees and one recorded hardware
    // boot contradicts.
    //
    // **One component and not all of them**, and the rule is the bootstrap
    // argument `user/supervisor/manifest.toml` makes: somebody is first, the
    // frame is what starts that one, and every other start moves above it. A
    // frame that started all five would be a frame that had kept the job this
    // task exists to move.
    //
    // RFC 0075 is why no `Prepared` appears here. The occupant's pages were
    // derived from its account and are owed back to it by `tear_down`; handing
    // them to `process::reap` would return them to the frame allocator instead,
    // which is a double free the type system would not name.
    //
    // Guarded on `held` and not only on the supervisor existing. A supervisor
    // with nothing to fill would be a boot that scheduled a component and proved
    // nothing about spawning — worse than not scheduling it, because the log
    // would carry a `scheduled` line either way. The two conditions come from
    // one function so they cannot drift: `held_open` answers `None` unless there
    // is a supervisor *and* a worker *and* a place for it.
    if let Some((cpu, tsc_khz)) = worker
        && let Some(index) = held
        && let Some(at) = supervising(&modules, count)
    {
        // Taken out of the array for the length of the run, because the ring
        // server borrows the held-open place mutably while the supervisor's own
        // occupant is being scheduled out of the same array. Two `&mut` into one
        // array is what this avoids, and taking it out is the honest version:
        // for as long as the supervisor is running, that place is the server's.
        let Some(mut open) = extras.get_mut(index).and_then(Option::take) else {
            return Err(Failure::WrongPlace);
        };
        let Some(mut extra) = extras.get_mut(at).and_then(Option::take) else {
            return Err(Failure::WrongPlace);
        };

        let consulted = {
            let occupant = extra.place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
            // Nothing has died here — this place has never been filled — so the
            // watch buys no notice on this consultation. It is taken anyway,
            // because the supervisor matches a death against the endpoint on its
            // board and a row with no endpoint would be a row no future death
            // could ever be attributed to.
            let watches = watch(occupant, frames)?;
            let mut asking = Consulting {
                generation,
                frames,
                kernel,
                features,
                supervisor: &mut supervisor,
                reservations: &reservations,
                on: (cpu, tsc_khz),
            };
            // SAFETY: `cpu` is the core `main` vouched is started and idle, and
            // `occupant` is the instance this frame spawned a few lines above
            // with its address space live.
            unsafe { consult(&mut asking, occupant, &mut open.place, &open.account, watches, now) }?
        };
        open.place.budget = consulted.budget;
        report.scheduled += 1;
        scheduled_line(
            Name(SUPERVISOR),
            cpu,
            SUPERVISOR_LIFE,
            consulted.announced,
            consulted.death,
        );
        supervised_line(&consulted, u32::from(open.place.occupant.is_some()));
        // What it made of the generation, on the one boot in this file that
        // shows it one. Read after the core came back, off the far half of the
        // same page the frame wrote the root onto.
        if let Some(occupant) = extra.place.occupant.as_ref() {
            // SAFETY: the core reported finished — `consult` waits for it — and
            // the page is still mapped; an address space is torn down by
            // `tear_down` and not here.
            if let Some((digest, counts)) = unsafe { read_assembled(occupant) } {
                assembled_line(digest, counts);
            }
        }

        // The spawn the *supervisor* made, recorded on this side. Everything
        // below is what `fill` does after its own spawn and for the same
        // reasons — it is deliberately not inside `Serving::spawn`, because a
        // ring server that mounted state trees would be a server that does the
        // frame's bookkeeping on a submitter's schedule.
        if open.place.occupant.is_some() {
            let record = Record::read(open.place.module).map_err(Failure::Manifest)?;
            report.spawns += 1;
            report.unbound += unbound_needs(record);
            spawned_line(record, &open.place, consulted.spawned);
            mounted_line(record, &open.place, mount(tree, &open.place, &mut report)?);
            open.place.reservation = Some(
                reservations
                    .grant(&Demand::of(record))
                    .map_err(|why| Failure::Admission(why.code()))?,
            );
            publish(&mut open.place, &mut supervisor, &ledger_ring, &mut report)?;
        }

        // The held-open place goes back; the supervisor's does **not**, yet. It
        // is kept out until the scripted place below has been through its
        // lifecycle, because every death there is a death this supervisor is
        // consulted about — RFC 0008's *restart is the supervisor's act*, and
        // RFC 0076's *it is told when it is started*. It is put back before the
        // teardown loop, which is the one place every place has to be present.
        let Some(back) = extras.get_mut(index) else { return Err(Failure::WrongPlace) };
        *back = Some(open);
        consulting = Some((at, cpu, tsc_khz, extra));
    }

    // ------------------------------------------ the supplied place's occupant
    //
    // `E1-B05`'s second act, on the one boot that asks for it. The place was
    // filled above with a device window the frame *supplied* rather than carved;
    // this is that place's occupant being handed a core, which is the second and
    // third of the three things `CHAOS_GAP` says the work needs.
    //
    // **Two shapes, and which one a boot takes is decided by whether it brought
    // a client.** Without one, [`run_ring3`] waits: the occupant runs, reads its
    // own device and ends before the next line is printed, which is the whole of
    // what `cargo xtask blk place` asserts and is unchanged. With one,
    // [`serve_ring3`] starts the core and does *not* wait, the client submits
    // from here while the occupant answers over there, and the frame serves the
    // occupant's control ring for the whole of the join. That second shape is
    // `E1-B05`'s third act and the word `CHAOS_GAP` spent two epochs calling
    // *concurrently*.
    //
    // Guarded on the supply rather than on a parameter, so that every boot with
    // no supplied place does exactly what it did before: `supplied` is empty on
    // all of them but two.
    if let Some((cpu, tsc_khz)) = worker
        && let Some(index) = supplied_place(&extras, supplied)
    {
        let record = {
            let Some(extra) = extras.get(index).and_then(Option::as_ref) else {
                return Err(Failure::WrongPlace);
            };
            Record::read(extra.place.module).map_err(Failure::Manifest)?
        };
        match datapath {
            None => {
                let Some(extra) = extras.get_mut(index).and_then(Option::as_mut) else {
                    return Err(Failure::WrongPlace);
                };
                let occupant = extra.place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
                // Told where its device is before it is told to look, which is
                // the ordering every board in this tree keeps: the page is
                // complete, magic last, before any core can read it.
                //
                // SAFETY: `occupant.board` is a frame `fill` allocated for this
                // instance and mapped into its address space and nobody else's;
                // the direct map covers it and no core is inside this instance.
                unsafe { write_routing(occupant, routing) }?;
                let life = f_virtio_blk::routing::life::IDENTIFY;
                // SAFETY: `cpu` is the core `main` vouched is started and idle —
                // the consultation above ran to completion on it, which is what
                // `run_on` returning `Ok` means, so it is idle again — and
                // `occupant` is the instance `fill` spawned a loop ago with its
                // address space live.
                let ran = unsafe { run_ring3(occupant, kernel, features, (cpu, tsc_khz), life) };
                let (announced, death) = ran?;
                report.scheduled += 1;
                scheduled_line(Name(record.label()), cpu, life, announced, death);
                // SAFETY: the core reported finished, and the page is still
                // mapped — an address space is torn down by `tear_down` and not
                // here.
                let identified = unsafe { read_identified(occupant) };
                identified_line(Name(record.label()), identified);
                // The component's own account of the supply, required rather
                // than printed. A boot in which this occupant ran and reported
                // nothing is one where the window was mapped somewhere it could
                // not reach, which is precisely the failure the frame's own
                // supply line cannot see.
                let Some((outcome, _)) = identified else { return Err(Failure::WrongPlace) };
                if outcome != f_virtio_blk::routing::stopped::IDENTIFIED {
                    return Err(Failure::WrongPlace);
                }
            }
            Some(client) => {
                // **One pass per generation of the occupant**, and the loop is
                // `E1-P06`. A client that answers `Drove::Kill` has work
                // outstanding and wants the driver taken away from under it; the
                // frame does that inside `serve_ring3`, tears the dead occupant
                // down, refills the place *from the same supply*, and runs the
                // client again against the new one. What the client holds across
                // that boundary is its own — a restart means a new table, so
                // every registration it had is gone and every entry that was in
                // flight is unanswered.
                //
                // Bounded rather than open, because a demonstration whose exit
                // condition is the client's word for it is a demonstration that
                // hangs when the client is wrong.
                let mut generation = 0u32;
                // The window, allocated once and freed at the end of the loop
                // whether a swap happened or not. A page nobody used costs a
                // page; a page allocated inside the branch that needs it would
                // be a page the outgoing occupant is told about after it has
                // already been asked to write into it.
                let window = frames.alloc_zeroed(Order::FRAME).ok_or(Failure::NoMemory)?;
                let window_at = crate::process::SWAP_WINDOW;
                let window_bytes = crate::process::SWAP_WINDOW_MAX;
                // The half of the protocol that outlives one pass: a swap is
                // planned and staged against the occupant on its way out, and
                // acknowledged against the one that replaced it.
                let mut swapping: Option<f_abi::swap::Swap> = None;
                let mut replaying = 0u32;
                loop {
                    let Some(extra) = extras.get_mut(index).and_then(Option::as_mut) else {
                        return Err(Failure::WrongPlace);
                    };
                    let occupant = extra.place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
                    // Told where its device is before it is told to look, and
                    // told again on every generation: a refilled place has a new
                    // board, zeroed, because the account it was carved from was
                    // handed back and taken again.
                    //
                    // SAFETY: `occupant.board` is a frame `fill` allocated for
                    // this instance and mapped into its address space and nobody
                    // else's; the direct map covers it and no core is inside
                    // this instance.
                    unsafe { write_routing(occupant, routing) }?;
                    // The transfer window, in this occupant's space and at the
                    // address both generations know it by. Mapped for every
                    // occupant rather than only for the ones a swap touches: a
                    // window mapped when the swap is decided would be a mapping
                    // made while the component holds a core.
                    // SAFETY: the instance `fill` or `spawn` built, with its
                    // address space live and no core inside it yet.
                    unsafe { show_window(occupant, frames, features, window) }?;
                    // SAFETY: as above.
                    unsafe { write_swap(occupant, window_at, window_bytes, replaying) }?;
                    replaying = 0;
                    // The data ring's header, written by the *grantor* and
                    // adopted by the occupant, which is `f_ring::adopt` and RFC
                    // 0037: the component believes nothing it is handed and would
                    // do exactly this if its peer were hostile. Written here
                    // rather than in `spawn` because the entry count is the
                    // client's business and `spawn` has no client.
                    if occupant.data == 0 {
                        return Err(Failure::WrongPlace);
                    }
                    let bytes = u32::try_from(FRAME_SIZE).map_err(|_| Failure::WrongPlace)?;
                    // SAFETY: `occupant.data` is a frame `fill` allocated zeroed
                    // for this instance out of its own account, mapped into its
                    // address space and nobody else's, frame-aligned and
                    // `FRAME_SIZE` long, with no reference into it held anywhere.
                    // The core is idle until `serve_ring3` starts it.
                    let described = unsafe {
                        Mapping::describe(occupant.data as *mut u8, bytes, CONTROL_ENTRIES, 0, 0, 0)
                    };
                    described.map_err(Failure::Ring)?;

                    let life = f_virtio_blk::routing::life::SERVE;
                    // SAFETY: as the `None` arm's `run_ring3` — the core is
                    // started and idle and this instance's address space is live
                    // — and `client` serves only the occupant's control ring,
                    // whose two ends are single-producer and single-consumer.
                    let ran = unsafe {
                        serve_ring3(occupant, frames, features, (cpu, tsc_khz), life, client)
                    };
                    let (announced, death, driven) = ran?;
                    report.scheduled += 1;
                    scheduled_line(Name(record.label()), cpu, life, announced, death);
                    retained = retained.saturating_add(client.retained(frames));
                    // The client's news, after the join and never instead of it.
                    let drove = driven.map_err(Failure::Datapath)?;

                    // --- the generation swap -----------------------------
                    //
                    // **`E2-B06`'s own sentence, at the boot.** That task is met
                    // in `sim/src/swap.rs` and was unpaid here, because no boot
                    // could put two generations of one component in front of the
                    // frame. `generations` finds the successor the loader placed,
                    // and this is what installs it.
                    //
                    // The order is `abi/src/swap.rs`'s and `sim/src/swap.rs`'s,
                    // step for step, because two implementors of one protocol
                    // that agree only in outline are two protocols.
                    if let Drove::Swap { in_flight } = drove {
                        let Some(&module) = successors.get(index).filter(|it| !it.is_empty())
                        else {
                            // Asked to swap on a boot that was handed no second
                            // generation. A refusal and not a restart: the client
                            // asked for something this machine was not given.
                            return Err(Failure::WrongPlace);
                        };
                        let next = Record::read(module).map_err(Failure::Manifest)?;
                        let held = Record::read(extra.place.module).map_err(Failure::Manifest)?;
                        // Planned from the two *declarations* and nothing else,
                        // which is the sentence `kernel/src/component.rs`'s
                        // refusal is being replaced by. `Incompatible` is not an
                        // abandonment — it is a pair that never should have been
                        // offered — so it fails the boot rather than restarting
                        // the place.
                        let mut swap = f_abi::swap::Swap::plan(&held.transfer, &next.transfer)
                            .map_err(|_| Failure::WrongPlace)?;

                        // What the occupant said on its way out. The frame asked
                        // it to hand over while it ran; this is the answer, and
                        // the order below is what turns two readings into a
                        // protocol rather than a pair of hopes.
                        // SAFETY: the core reported finished and the page is
                        // still mapped.
                        let (quiescent, records) = unsafe { read_handed(occupant) };
                        // SAFETY: as above.
                        let was = unsafe { read_generation(occupant) };

                        extra.place.routing.pause();
                        // The rings are empty because the client stopped
                        // submitting before it asked, which is what `drive`
                        // answering `Swap` means. RFC 0018's cursors are the
                        // other half and are the *frame's* to check; this is the
                        // frame checking it.
                        let staged = swap
                            .drained(true)
                            .and_then(|()| swap.asserted(quiescent))
                            .and_then(|()| swap.wrote(records));
                        if let Err(why) = staged {
                            abandoned_line(Name(record.label()), why);
                            extra.place.routing.commit(extra.place.epoch);
                            return Err(Failure::WrongPlace);
                        }

                        handed_line(Name(record.label()), was, in_flight, records);

                        // --- the place takes its successor -------------------
                        //
                        // `place.module` and `place.manifest` move **together**,
                        // and that is what keeps `spawn`'s refusal fail-closed
                        // rather than deleted: it compares the module it is given
                        // against the manifest the place holds, and both are now
                        // the successor's. A different manifest is still refused
                        // everywhere else, and is reachable here only through a
                        // plan made from two declarations.
                        let cause = cause::pack(cause::STOPPED, 0);
                        tear_down(
                            frames,
                            &mut extra.place,
                            &mut supervisor,
                            &extra.account,
                            cause,
                            &mut report,
                            tree,
                        )?;
                        extra.place.module = module;
                        extra.place.manifest = ContentId::of(module);
                        let offered = offer(
                            &mut supervisor,
                            &extra.account,
                            &extra.place,
                            next,
                            frames,
                            supplied,
                        )?;
                        // SAFETY: as the first spawn — the caller's guarantee,
                        // and the place is empty because the teardown emptied it.
                        unsafe {
                            spawn(
                                frames,
                                kernel,
                                features,
                                &mut extra.place,
                                &extra.account,
                                &mut supervisor,
                                &reservations,
                                offered,
                                supplied,
                            )
                        }?;
                        report.spawns += 1;
                        mounted_line(next, &extra.place, mount(tree, &extra.place, &mut report)?);
                        let epoch = extra.place.occupant.as_ref().map_or(0, |it| it.epoch);
                        swapped_line(Name(next.label()), epoch, records);
                        // The window, and where the records are, told to the new
                        // occupant before it is given a core.
                        let taking = extra.place.occupant.as_ref().ok_or(Failure::WrongPlace)?;
                        // SAFETY: the instance `spawn` just built, with its
                        // address space live and no core inside it.
                        unsafe { write_swap(taking, window_at, window_bytes, records) }?;
                        generation += 1;
                        if generation > KILLS_MAX {
                            return Err(Failure::WrongPlace);
                        }
                        swapping = Some(swap);
                        replaying = records;
                        continue;
                    }

                    let Drove::Kill { in_flight } = drove else {
                        // --- the generation that ended by agreement ----------
                        //
                        // SAFETY: the core reported finished, and the page is
                        // still mapped.
                        let served = unsafe { read_served(occupant) };
                        served_line(Name(record.label()), served);
                        // What makes this a demonstration rather than a log line:
                        // the count is the *occupant's* and the frame did not
                        // write it. A boot whose client submitted into a ring
                        // nobody was reading prints an identical `scheduled` line
                        // and a zero here.
                        let Some((outcome, entries, _)) = served else {
                            return Err(Failure::WrongPlace);
                        };
                        if outcome != f_virtio_blk::routing::stopped::TOLD || entries == 0 {
                            return Err(Failure::WrongPlace);
                        }
                        // --- the swap's far end ---------------------------
                        //
                        // Acknowledged and committed against the occupant that
                        // *replaced* the one that handed over, which is why the
                        // `Swap` outlives a pass of this loop. `Swap::commit`
                        // refuses from any phase but `Acknowledged`, and is the
                        // only guard on the ordering — the retire-after-commit
                        // half is guarded by nothing but this code.
                        if let Some(mut swap) = swapping.take() {
                            // SAFETY: the core reported finished and the page is
                            // still mapped.
                            let replayed = unsafe { read_replayed(occupant) };
                            let taken = swap.records();
                            let done = swap
                                .acknowledged(replayed == taken)
                                .and_then(|()| swap.commit(&extra.place.routing, generation));
                            if let Err(why) = done {
                                abandoned_line(Name(record.label()), why);
                                extra.place.abandoned += 1;
                                return Err(Failure::WrongPlace);
                            }
                            extra.place.swaps += 1;
                            report.swaps += 1;
                            // SAFETY: as above.
                            let now = unsafe { read_generation(occupant) };
                            committed_line(Name(record.label()), now, replayed, taken);
                        }
                        break;
                    };

                    // --- the generation that was killed -----------------------
                    //
                    // **No report is read here, and the absence is the point.**
                    // Writing that report is a component's last act: it fills in
                    // the far half of its board and then ends. An occupant taken
                    // away mid-serve never reaches it, so a frame that required
                    // one would be requiring the component to have finished —
                    // which is precisely what this generation did not do.
                    //
                    // What is believed about this generation instead is the
                    // client's, and it is the only thing that can be: the
                    // operations it had outstanding, counted on its own side.
                    //
                    // A kill with nothing outstanding is a restart, and a restart
                    // demonstrates nothing about a blast radius. Refused here
                    // rather than reported, because the whole claim rests on it.
                    if in_flight == 0 {
                        return Err(Failure::WrongPlace);
                    }
                    generation += 1;
                    if generation > KILLS_MAX {
                        return Err(Failure::WrongPlace);
                    }
                    killed_line(Name(record.label()), in_flight, death);
                    report.faults += 1;

                    // --- the refill, from the same supply --------------------
                    //
                    // The single-core respawn branch below has carried a comment
                    // for two epochs saying a supplied object would have to reach
                    // it before this could work, *and that this was `E1-P06`'s
                    // business*. It is, and the answer turned out not to need the
                    // place to carry anything: `demonstrate` was handed the
                    // supply as an argument and still holds it, so the refill
                    // offers the same window and the same queue memory the first
                    // spawn was given. A device window carved out of a fresh
                    // account would be a different device.
                    let cause = cause::pack(cause::FAULT, u64::from(error::argument::BAD_ADDRESS));
                    tear_down(
                        frames,
                        &mut extra.place,
                        &mut supervisor,
                        &extra.account,
                        cause,
                        &mut report,
                        tree,
                    )?;
                    let offered = offer(
                        &mut supervisor,
                        &extra.account,
                        &extra.place,
                        record,
                        frames,
                        supplied,
                    )?;
                    // SAFETY: as the first spawn — the caller's guarantee, and
                    // the place is empty because the teardown above emptied it.
                    unsafe {
                        spawn(
                            frames,
                            kernel,
                            features,
                            &mut extra.place,
                            &extra.account,
                            &mut supervisor,
                            &reservations,
                            offered,
                            supplied,
                        )
                    }?;
                    extra.place.restarts += 1;
                    report.restarts += 1;
                    report.spawns += 1;
                    // The new occupant's tree, into the same slot the dead one
                    // published through. `Place::slot` is on the place and not on
                    // the occupant for exactly this: a reader watching one node
                    // across a kill is watching *this place* refill, rather than
                    // two unrelated components that happened to be given one
                    // address. It is also what keeps this function's own closing
                    // equality — every spawn published a tree — true across a
                    // restart.
                    mounted_line(record, &extra.place, mount(tree, &extra.place, &mut report)?);
                    let epoch = extra.place.occupant.as_ref().map_or(0, |next| next.epoch);
                    refilled_line(Name(record.label()), epoch);
                }
                // SAFETY: allocated a few lines up, mapped only into address
                // spaces `tear_down` has taken down or is about to, and named by
                // nothing else.
                unsafe { frames.free(window) };
            }
        }
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
    // The endpoint, in the **supervisor's** table, granted before the death
    // rather than after it. RFC 0076: a place's `PEER_GONE` pends in the slot of
    // the endpoint that names it, so a supervisor granted one afterwards would
    // hold a handle to a place whose death it had already missed. This is the
    // powerbox grant that makes a supervisor tellable, and the ordering is the
    // whole of it.
    let watching = match consulting.as_mut() {
        Some((_, _, _, extra)) => {
            let occupant = extra.place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
            Some(watch(occupant, frames)?)
        }
        None => None,
    };

    let cause = cause::pack(cause::FAULT, u64::from(error::argument::MALFORMED_HEADER));
    let torn = tear_down(frames, &mut place, &mut supervisor, &account, cause, &mut report, tree)?;
    // And the supervisor is told, in the slot it holds for this place.
    // `tear_down` posts to the frame's own table because that is the holder it
    // knows; a second holder is told here, at the one site that has one. Walking
    // *every* holder is what a revocation walk does and this is not one — the
    // day there are two supervisors, this line becomes that walk.
    if let Some(handle) = watching
        && let Some((_, _, _, extra)) = consulting.as_mut()
        && let Some(occupant) = extra.place.occupant.as_mut()
    {
        let _ = occupant.table.note_peer_gone(handle);
    }
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
    // One of them is the state-tree refusal, and it is counted apart because it
    // is not about a *supply*: the other five are ways a supervisor can offer
    // the wrong thing, and this one is a component that would have nothing to
    // say about itself. RFC 0065.
    report.mute += 1;
    tree.add(crate::state::node::COMPONENTS_REFUSED, 1);
    crate::kprintln!(
        "  refusals      {} spawn(s) refused on purpose: {} way(s) a supply can be wrong — \
         missing, undeclared, wrong type, short rights, short quantity — and a manifest \
         declaring no state tree, refused ADMISSION/NO_STATE_TREE",
        refusals,
        refusals - 1,
    );

    // -------------------------------------------------------------- restart
    //
    // **The supervisor's act, and this is the line where that stopped being a
    // sentence in RFC 0008 and became something the boot does.** The frame does
    // not decide: it hands a core to the component that was told, on its ring,
    // that this place lost its occupant, and then performs whatever that
    // component asked for. `kernel/src/component.rs` held `policy::decide` for
    // three epochs and no longer contains it.
    //
    // `now` is a tick count read once through `Env` and advanced by the backoff
    // the *supervisor* returns, which is why nothing in this log moves between a
    // fast host and a slow one.
    if let Some((_, cpu, tsc_khz, extra)) = consulting.as_mut() {
        // The notice the teardown made pending, onto the supervisor's own ring,
        // *before* it is given a core. This is RFC 0076's whole mechanism in one
        // call: everything a supervisor is told is memory put there while
        // nothing was running. It happens before the occupant is borrowed,
        // because `publish` takes the place and the consultation takes what is
        // in it.
        publish_only(&mut extra.place, &mut supervisor, &ledger_ring, &mut report)?;
        let occupant = extra.place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
        let mut asking = Consulting {
            generation,
            frames,
            kernel,
            features,
            supervisor: &mut supervisor,
            reservations: &reservations,
            on: (*cpu, *tsc_khz),
        };
        let watches = watching.ok_or(Failure::WrongPlace)?;
        // SAFETY: `cpu` is the core `main` vouched is started and idle, and this
        // instance's address space is live — it is torn down in the loop at the
        // end of this function and not before.
        let consulted =
            unsafe { consult(&mut asking, occupant, &mut place, &account, watches, now) }?;
        place.budget = consulted.budget;
        report.scheduled += 1;
        supervised_line(&consulted, u32::from(place.occupant.is_some()));
        if place.occupant.is_none() {
            // A supervisor that was asked and did not refill is not a failure
            // here — `Leave` and `Retire` are two of its three verdicts — but it
            // is not this demonstration either, and a boot that carried on would
            // be reporting a restart that did not happen.
            return Err(Failure::WrongPlace);
        }
        place.restarts += 1;
        report.restarts += 1;
        restart_line(record, &place, consulted.verdict);
    } else {
        // --- no supervisor to ask, and the boot says so rather than pretending
        //
        // A machine with one core has nowhere to run a supervisor — `main` hands
        // `worker` as `None` and `held_open` answers `None` with it — so this
        // place is refilled by the frame, with no policy consulted at all.
        //
        // **That is a smaller demonstration and it is labelled as one.** The
        // alternative was to keep a copy of the decision here for this case,
        // which is exactly the thing RFC 0008 spent three epochs removing: a
        // second implementation of a rule, reachable on a configuration nobody
        // looks at, drifting from the one above it. A frame that respawns
        // without deciding is honest; a frame that decides is the reversal
        // coming back.
        //
        // `cargo xtask cores` is the boot that takes this path.
        // Nothing supplied: this is the respawn path on a single-core boot,
        // and it refills a place from the same account the first spawn used.
        // A supplied object would have to be carried on the place to be
        // available here, which is `E1-P06`'s business and not this branch's.
        let offered = offer(&mut supervisor, &account, &place, record, frames, &[])?;
        // SAFETY: as the first spawn — the caller's guarantee, and the place is
        // empty because the teardown above emptied it.
        unsafe {
            spawn(
                frames,
                kernel,
                features,
                &mut place,
                &account,
                &mut supervisor,
                &reservations,
                offered,
                &[],
            )
        }?;
        place.restarts += 1;
        report.restarts += 1;
        crate::kprintln!(
            "  restart       place {} under {} — refilled by the frame, with no supervisor \
             consulted: this machine has no core to run one on",
            Name(record.label()),
            restart::label(record.restart),
        );
    }
    let spawned = place.occupant.as_ref().map_or((0, 0, 0), |occupant| (occupant.epoch, 0, 0));
    report.spawns += 1;
    crate::kprintln!(
        "  spawn         place {} epoch {} — nothing carried over: new table, new memory, \
         new control ring, new state tree",
        Name(record.label()),
        spawned.0,
    );
    // And the place's mount, filled again. The *node* is the place's and
    // survives its occupant; the address in it is the occupant's and does not,
    // which is what a reader watching one mount across a restart sees and is
    // the whole reason the slot lives on the place. Nothing is carried over
    // here either: the frame this instance publishes into was zeroed by
    // `charge` on its way out of the account.
    mounted_line(record, &place, mount(tree, &place, &mut report)?);

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
    // Through the ring server rather than by reaching into the occupant, which
    // is RFC 0073's whole subject. These entries are built here rather than
    // submitted by a component, and that is now a statement about *this
    // demonstration* rather than about the tree: `user/supervisor` submits a
    // real `op::SPAWN` above. What the scripted stop keeps is a stop whose
    // deadline the boot chooses, which a component cannot be asked to produce
    // on cue.
    //
    // The frame is its own submitter here, as it was everywhere before there was
    // a supervisor; `submitted_from` is the same bytes, copied one statement
    // earlier so the server can hold the other half exclusively.
    let submitted_from = supervisor;
    let mut serving = Serving {
        place: &mut place,
        submitter: &submitted_from,
        supervisor: &mut supervisor,
        frames,
        account: &account,
        kernel,
        features,
        reservations: &reservations,
        spawned: (0, 0, 0),
        answered: 0,
    };
    let stop_entry = |deadline: u64| f_abi::Sqe {
        opcode: f_abi::control::op::STOP,
        cap: endpoint.bits(),
        deadline,
        ..f_abi::Sqe::ZERO
    };

    // The refusals first, and they are checked rather than printed: a boot line
    // per refusal would move the trace hash for a check, and a refusal nobody
    // asserts is a refusal that can quietly stop happening.
    //
    // A promise nothing can refuse is one the frame will not make (RFC 0008).
    if serving.execute(&stop_entry(f_abi::NO_DEADLINE)).result == 0 {
        return Err(Failure::Connect(0));
    }
    // And an opcode this build does not implement is refused and never ignored,
    // which is the arm every opcode that has not arrived yet lands in.
    if serving.execute(&f_abi::Sqe { opcode: 0x7F, ..f_abi::Sqe::ZERO }).result == 0 {
        return Err(Failure::Connect(0));
    }

    if serving.execute(&stop_entry(STOP_DEADLINE)).ext != STOP_DEADLINE {
        return Err(Failure::Connect(0));
    }
    // A later deadline may not move an earlier one, which is the one thing a
    // stop may never do — and the completion says which deadline is *kept*, so
    // a submitter that asked for the later one is told it did not get it.
    if serving.execute(&stop_entry(STOP_DEADLINE + 1)).ext != STOP_DEADLINE {
        return Err(Failure::Connect(0));
    }
    let occupant = place.occupant.as_mut().ok_or(Failure::WrongPlace)?;
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
    let torn = tear_down(frames, &mut place, &mut supervisor, &account, cause, &mut report, tree)?;
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
    //
    // **What is demonstrated here is the mechanism, and the verdict behind it is
    // no longer computed here.** This used to drive `policy::decide` in a loop
    // until it returned `Retire` and then act on that — a pure computation
    // wearing a boot's clothes, since no memory was spent and nothing was
    // spawned in any round of it. That arithmetic is
    // `f_supervisor::policy`'s now, and
    // `a_budget_spent_inside_its_window_retires_the_place` is the test. What
    // stays is what a test cannot do: tear the place down as retired and show
    // both ways a client meets one.
    //
    // *What that costs, stated rather than left to be noticed:* the retirement
    // in this boot is the frame's scripted act and not a verdict that came back
    // across the boundary, which the restart above now is. Driving it through
    // the supervisor means four deaths — `max_restarts` restarts and the one
    // that finds the budget spent — each with its own teardown, notice, core
    // schedule and refill. That is the next increment and it is a bigger boot,
    // not a harder one.
    let cause = cause::pack(cause::RETIRED, u64::from(place.epoch));
    let torn = tear_down(frames, &mut place, &mut supervisor, &account, cause, &mut report, tree)?;
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

    // The window the budget is counted over **is no longer exercised here**, and
    // the line it printed is gone rather than kept saying something the frame
    // did not do. It was four calls into `policy::decide` on a tally of its own
    // — a count exhausted inside the window retires, the same count once the
    // window has elapsed does not, which is the difference between a budget and
    // a lifetime cap that `docs/manifest.md` says is schema 2's.
    //
    // Those four calls are
    // `f_supervisor::policy`'s
    // `the_same_count_retires_inside_the_window_and_does_not_once_it_has_elapsed`
    // now. A boot cannot reach them at all any more: the window is a property of
    // arithmetic the frame does not contain, and a line here asserting it would
    // be the frame claiming to have checked something it cannot see.
    crate::kprintln!(
        "  budget        a window {} tick(s) wide and {} restart(s) in it, declared by the \
         manifest and counted above the frame — `f_supervisor::policy` is where it is decided \
         and tested",
        record.budget_window_ticks,
        record.max_restarts,
    );

    // ------------------------------------------------- the other places, back
    //
    // The supervisor's place first, back into the array it was held out of for
    // the whole of the scripted lifecycle above. It is put back *before* the
    // loop rather than skipped by it, because the loop is where every place is
    // torn down and a place that missed it is a place whose frames never come
    // home — which is exactly what the boot said, in the one sentence a leak
    // gets: *a component's frames did not all come back*.
    if let Some((at, _, _, extra)) = consulting.take() {
        let Some(slot) = extras.get_mut(at) else { return Err(Failure::WrongPlace) };
        *slot = Some(extra);
    }

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
            tree,
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
    if report.owed != 0 || report.collected + report.handed != report.notices {
        return Err(Failure::Leaked);
    }
    crate::kprintln!(
        "  notices       {} published in slot-then-stop-then-grade order over {} round(s), {} \
         drained back at a polling point as {} of 7 kind(s), {} handed to a component that \
         drains for itself, {} still owed",
        report.notices,
        report.rounds,
        report.collected,
        report.kinds.count_ones(),
        report.handed,
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

    if frames.free_count().saturating_add(retained) != before {
        return Err(Failure::Leaked);
    }

    // E1-B15's first clause, as an equality rather than as a log line: **every
    // component the supervisor started published a tree**. A spawn that had
    // produced no tree would be one `admit` should have refused, so a gap here
    // is a hole in the refusal and not a component that was merely quiet — and
    // it fails the boot for the reason every other check in this function does.
    if report.mounted != report.spawns {
        return Err(Failure::StateTree(error::pack(
            error::ADMISSION,
            error::admission::NO_STATE_TREE,
        )));
    }
    // And every mount is out again, which is the other half: a root still
    // naming a frame that has gone back to an account is a reader following one
    // place's mount into whatever was put there next.
    if tree.mounted() != 0 {
        return Err(Failure::StateTree(error::pack(
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        )));
    }

    report.faults = place.faults;
    Ok(report)
}

/// Which of the unscripted places the caller supplied a need for, if any.
///
/// Answers the *first* component named in `supplied` and finds the place whose
/// manifest carries that label. One component and not a set, because a supply is
/// what one boot found on the bus and this build has never had two at once; a
/// second would want this to answer a list, and the day it does is the day the
/// caller has two device windows to hand over.
///
/// Matched on the label rather than on the module pointer, because the supply is
/// written where the device was found and the modules are the loader's — a match
/// on an address would be this frame assuming an order
/// `docs/second-boot-outside-qemu.md` records hardware not keeping.
fn supplied_place(extras: &[Option<Extra>; PLACES_MAX], supplied: &[Supplied]) -> Option<usize> {
    let want = supplied.iter().map(|item| item.component).find(|name| !name.is_empty())?;
    (1..PLACES_MAX).find(|index| {
        extras
            .get(*index)
            .and_then(Option::as_ref)
            .and_then(|extra| Record::read(extra.place.module).ok())
            .is_some_and(|record| record.label() == want)
    })
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

/// Who puts the first occupant into a place [`fill`] has just built.
///
/// # Why this is a parameter and not two functions
///
/// Because the *place* is identical either way and that is the claim being
/// made. RFC 0041's place is a content hash, an endpoint and an account, and
/// nothing about it knows who spawns into it; a second `fill` for the held-open
/// case would be a second set of admissions, and `fill`'s own doc comment
/// already says what happens when two bodies build one thing — the boot log
/// says two places were filled and means two different things by it.
///
/// So the only difference is whether the spawn happens here, and it is one
/// branch at the one line where it could differ.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Occupant {
    /// The frame spawns, immediately, as it has since E1-B05 began. Every place
    /// but one.
    ByTheFrame,
    /// The place is built, admitted and left **empty**, for `user/supervisor` to
    /// fill from ring 3 over its control ring.
    ///
    /// This is the half of RFC 0008 the frame has been holding: *restart is the
    /// supervisor's act and the frame provides only the mechanism.* A place in
    /// this state is not a failure and is not a partial boot — it is a place
    /// with no occupant, which is exactly the state RFC 0041 says a place
    /// spends most of its life in and the state a connect is required to pend
    /// against.
    ByTheSupervisor,
}

/// One object the caller hands a spawn for a need the account cannot answer.
///
/// # Why a need can be unanswerable by an account
///
/// Because an account is memory the frame allocated and may take back, and a
/// device's registers are neither. `user/virtio-blk/manifest.toml` declares
/// `mmio` as four frames, and what it means by them is *the window the firmware
/// put this device's configuration in* — a fixed physical address this boot
/// found by walking the bus. [`carve`] would answer that need with four
/// arbitrary frames off the account's watermark, which is memory the driver
/// could read and write and which the device has never heard of.
///
/// So a supplied need is sourced by the caller and named by the manifest, and
/// the two are checked against each other: the extent the manifest declares must
/// be the extent the caller brings, or the spawn is refused. A need supplied at
/// the wrong size is the failure this check exists for, because the symptom
/// otherwise is a driver reading a device window that stops halfway.
///
/// # Why it is not charged to the account
///
/// A charged object is one `reap` hands back. Handing a device window back to
/// the frame allocator would put a bus address into the free list, and the next
/// component to be given that frame would be writing to a disk controller. That
/// is the whole reason this is a separate path rather than a flag on [`carve`].
#[derive(Clone, Copy)]
pub struct Supplied {
    /// Which component this is for, as its manifest labels it.
    ///
    /// **Carried here rather than filtered by the caller**, and the reason is a
    /// hazard rather than convenience: `virtio-blk`, `virtio-net` and
    /// `virtio-gpu` all declare needs called `mmio` and `queues`. A supply
    /// matched on the need's name alone would put the block device's register
    /// window into whichever of the three was filled first, and the symptom
    /// would be a driver talking fluently to the wrong device.
    pub component: &'static [u8],
    /// The need's name as the manifest spells it, matched exactly.
    pub name: &'static [u8],
    /// Where the object starts. Unit: bytes, physical.
    pub at: u64,
    /// How long it is. Must equal what the manifest declares for this need.
    /// Unit: bytes.
    pub bytes: u64,
    /// Whether the occupant's address space gets this mapped, and how.
    pub map: Placement,
}

/// What a supplied need becomes in the occupant's address space.
///
/// Named `Placement` rather than `Mapping` because `f_ring::Mapping` is a
/// channel over a region and is in scope here; two types called `Mapping` one
/// import apart is the kind of collision a reader resolves wrongly once.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Granted as a capability and not mapped. The component maps it itself, or
    /// holds it to hand on.
    Unmapped,
    /// Mapped writable at a fixed user address, with caching left on. Queue
    /// memory: ordinary RAM a device will read.
    Cached(u64),
    /// Mapped writable at a fixed user address with caching **off**. A device
    /// register window, where a stale read is a wrong answer about hardware
    /// state rather than a slow one.
    Uncached(u64),
    /// Mapped **read-only** at a fixed user address, and never zeroed.
    ///
    /// The frame *showing* a component something rather than giving it
    /// something, and the two halves of that sentence are the two ways this
    /// differs from [`Placement::Cached`].
    ///
    /// **Read-only**, because the one thing shown this way is the boot module,
    /// and a component that could write it could rewrite the generation it was
    /// measured from — at which point `kernel/src/measure.rs` is attesting to
    /// bytes that have since changed. `UserPage::ReadOnly` exists for exactly
    /// this: a rights bitmap the mapping cannot express is a rights bitmap that
    /// is not enforced.
    ///
    /// **Never zeroed**, because the bytes are the point. Every other supplied
    /// need is memory, and memory handed to a new occupant is zeroed so that
    /// *nothing carried over* stays true across a restart — which is what
    /// `E1-P06` found by being refused. A generation is not memory the occupant
    /// owns; it is a fact about the machine, and a fact blanked between
    /// occupants would be a supervisor told nothing about the topology it is
    /// part of. RFC 0094.
    Shown(u64),
}

/// The generation this machine was asked to be, for the component that
/// instantiates it.
///
/// Three facts and no bytes: where the boot module was supplied at, how long it
/// is, and the root it folds to. The frame holds all three already — it folded
/// the module to decide whether this machine is the generation `f.root=` named —
/// and until RFC 0094 it printed a line and let them go.
///
/// **The root is on the board and not recomputed by the reader**, which is the
/// whole of why it is carried. `f_assembler::Assembly::instantiate` refolds the
/// module and compares; a component that folded the module to get the root it
/// then compared against would be checking the bytes against themselves.
#[derive(Clone, Copy)]
pub struct Generation {
    /// How long the module is — its own length, not the extent it was mapped
    /// over.
    ///
    /// The two differ because a mapping is pages and a module is bytes. A reader
    /// given the mapped extent would read past the module into whatever the
    /// loader put after it, and `f_abi::boot::Module::read` would refuse — a
    /// refusal that is correct and says nothing about why.
    /// Unit: bytes.
    pub bytes: u64,
    /// What it folds to. Unit: none — a SHA-256.
    pub root: [u8; 32],
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
#[expect(
    clippy::too_many_arguments,
    reason = "the same argument `spawn` makes one function down, and this list is that one \
              plus the two a place is built from rather than spawned into: the frame's own \
              tree, which is where this place's occupant is mounted, and the mount slot it \
              takes. Bundling them would be a type that exists so a lint passes, which \
              `runtime::demonstrate` already declined for this reason"
)]
unsafe fn fill(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    supervisor: &mut Table,
    reservations: &mut Reservations,
    module: &'static [u8],
    report: &mut Report,
    tree: &crate::state::Tree,
    slot: usize,
    occupant: Occupant,
    supplied: &[Supplied],
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
        slot,
        manifest: ContentId::of(module),
        module,
        endpoint,
        epoch: 0,
        occupant: None,
        retired: false,
        reservation: None,
        budget: Budget::default(),
        routing: f_abi::swap::Routing::new(f_abi::swap::PAUSED),
        swaps: 0,
        abandoned: 0,
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

    if occupant == Occupant::ByTheFrame {
        let offered = offer(supervisor, &account, &place, record, frames, supplied)?;
        // SAFETY: the caller's guarantee, passed down. The place was built a few
        // lines ago and is empty.
        let spawned = unsafe {
            spawn(
                frames,
                kernel,
                features,
                &mut place,
                &account,
                supervisor,
                reservations,
                offered,
                // The same list `offer` was given a few lines up, and it has to
                // be: `offer` decides which needs the account does *not* pay
                // for and `spawn` decides which of them are mapped, so a caller
                // that handed one list to the first and an empty one to the
                // second produces a place holding a capability for a device
                // window that is in no address space.
                //
                // That is not hypothetical. It is what this line said until the
                // day an occupant of this place was first given a core, and the
                // symptom was a page fault at the third register page in a boot
                // whose every log line said the supply had worked.
                supplied,
            )
        }?;
        report.spawns += 1;
        report.unbound += unbound_needs(record);
        spawned_line(record, &place, spawned);
        mounted_line(record, &place, mount(tree, &place, report)?);
    } else {
        held_line(record, region.bytes());
    }

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

/// What a driver occupant said about its own device, read back off its board.
///
/// **The component's account and not the frame's**, and the pair is the point.
/// The `blk place` line a few above this one is the frame saying *I supplied
/// this window at this address*; this is the occupant saying *I read it*. A
/// window mapped into the wrong address space, or at the wrong address, or with
/// the wrong extent produces an identical frame line and nothing here — which is
/// why the boot refuses rather than printing a blank.
///
/// The capacity is in sectors because that is the unit the device publishes it
/// in, unconverted on purpose: `cargo xtask blk place` knows how large a disk it
/// made and multiplies, and a frame that did the arithmetic would be a frame
/// asserting a sector size nothing here reads.
fn identified_line(what: Name<'_>, identified: Option<(u64, u64)>) {
    match identified {
        Some((outcome, sectors)) => crate::kprintln!(
            "  identified    place {what} read its own device: {sectors} sector(s) of capacity, \
             outcome {outcome} — through the window its place was supplied with, from ring 3"
        ),
        None => crate::kprintln!(
            "  identified    place {what} left no report on its board, so nothing it was told \
             can be said to have arrived"
        ),
    }
}

/// How many times one boot may kill and refill a serving place.
///
/// One, today, and the number is a bound rather than a schedule: the client
/// decides when to ask, and this is what stops a client that keeps asking from
/// turning a demonstration into a loop with no exit. It rises when a half wants
/// more kills than one, and the thing that would have to rise with it is the
/// manifest's restart budget — eight in sixty thousand ticks — which is the
/// number that says how many a *supervisor* would allow.
/// Unit: kills.
const KILLS_MAX: u32 = 4;

/// A serving occupant taken away from its client on purpose.
///
/// **The one line in this boot that reports a component dying while it was
/// useful.** Everything else that ends a component here ended because the
/// component reached a point of its own choosing; this one did not, which is the
/// whole of what `E1-P06` is about and why the count of what was outstanding is
/// on the line rather than in a tally somewhere.
///
/// The death is printed because the *kind* of death is the evidence that the
/// kill worked: a `Killed` with vector 14 is a page fault taken at ring 3 on the
/// instruction after its text stopped being mapped, and an `Exited` here would
/// mean the component had gone before the frame got to it — which would make
/// every number below this line a measurement of something else.
fn killed_line(what: Name<'_>, in_flight: u32, death: crate::process::Death) {
    crate::kprintln!(
        "  killed        place {what} was taken away from its client with {in_flight} \
         operation(s) outstanding: {death:?} — a driver dying under load, which is the one \
         death in this boot the component did not choose"
    );
}

/// An occupant that handed its history over and stopped.
///
/// **The one line in this boot that reports a component ending while it was
/// working and being succeeded rather than replaced.** The generation is the
/// component's own answer — `user/virtio-blk` writes which build it is on every
/// report — so a swap that printed the frame's content address alone would be a
/// swap checked from one end.
fn handed_line(what: Name<'_>, was: u64, in_flight: u32, records: u32) {
    crate::kprintln!(
        "  handed over   place {what} generation {was} reached a quiescent point with \
         {in_flight} operation(s) outstanding and wrote {records} record(s) into the transfer \
         window — its own history, for its successor to replay"
    );
}

/// A place whose occupant is now the generation that succeeded the last one.
fn swapped_line(what: Name<'_>, epoch: u32, records: u32) {
    crate::kprintln!(
        "  swapped       place {what} epoch {epoch} — a second generation of the same \
         component in the same place, holding the same account, the same endpoint and the same \
         reservation, with {records} record(s) to replay"
    );
}

/// A swap that was planned and did not happen.
///
/// Reported and **not** charged to the restart budget. `abi/src/swap.rs` is
/// explicit: routing an abandonment through the restart path lets `max_restarts`
/// failed updates retire a healthy component's place, with the manifest's own
/// budget as the mechanism.
fn abandoned_line(what: Name<'_>, why: f_abi::swap::Abandoned) {
    crate::kprintln!(
        "  abandoned     place {what} kept the occupant it had: {} — a swap that did not \
         happen is not a restart, and this line is counted apart from one",
        why.label(),
    );
}

/// A swap that reached its far end: the successor replayed what it was handed.
///
/// The two counts are the whole verdict and they come from opposite sides. The
/// outgoing occupant said how many records it wrote; the incoming one says how
/// many it replayed; and `f_abi::swap::Swap::acknowledged` is refused unless they
/// agree. A single count, taken once, would be a swap reporting on itself.
fn committed_line(what: Name<'_>, now: u64, replayed: u32, taken: u32) {
    crate::kprintln!(
        "  committed     place {what} is generation {now} and replayed {replayed} of {taken} \n         record(s) its predecessor wrote — the routing word is open again, and a client \n         that submitted across the gap waited rather than being refused"
    );
}

/// A place refilled after a kill, and which occupant is in it now.
fn refilled_line(what: Name<'_>, epoch: u32) {
    crate::kprintln!(
        "  refilled      place {what} epoch {epoch} — the same device window and the same queue \
         memory, supplied again rather than carved: a new occupant of the same place, which is \
         what a client that held an endpoint across the kill comes back to"
    );
}

/// What a serving occupant said it did for its client, read back off its board.
///
/// **The third act's evidence, and it is the component's own account.** The
/// `scheduled` line above says the frame started a core; this says the occupant
/// on it took entries off a ring and answered them. A boot in which the client
/// submitted into a ring nobody was reading prints an identical `scheduled` line
/// and a zero here, which is precisely the failure the frame's own lines cannot
/// see — the same argument [`identified_line`] makes about a window mapped
/// nowhere, one act later.
///
/// The entries are counted and the bytes are reported beside them, never alone:
/// *nothing was refused* is a claim about a driver, and a driver that answered
/// nothing refused nothing either. `user/virtio-blk/manifest.toml` says the same
/// thing about the two state nodes these come from.
fn served_line(what: Name<'_>, served: Option<(u64, u64, u64)>) {
    match served {
        Some((outcome, entries, bytes)) => crate::kprintln!(
            "  served        place {what} answered {entries} entr(ies) and moved {bytes} B for a \
             client of the frame's, outcome {outcome} — from ring 3, on its own core, while the \
             client submitted"
        ),
        None => crate::kprintln!(
            "  served        place {what} left no report on its board, so nothing it was given \
             can be said to have been answered"
        ),
    }
}

/// What the supervisor made of the generation it was shown, read back off its
/// board.
///
/// **The component's account and not the frame's**, which is the same pairing
/// every other line in this file makes. The frame wrote the module's extent and
/// its root onto that page; this is what came back — a digest over the topology
/// the component assembled, and five counts of what it did with it.
///
/// `None` for a board with no digest on it, which is every boot that selected no
/// generation and every boot whose supervisor was refused before it got that far.
/// Zero is a real digest and would be indistinguishable from *there was none*,
/// so the test is the counts rather than the hash: a run that produced nothing
/// started nothing, skipped nothing and refused nothing.
unsafe fn read_assembled(occupant: &Instance) -> Option<(u64, [u64; 6])> {
    if occupant.board == 0 {
        return None;
    }
    use f_supervisor::routing::at;
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let get = |offset: u32| -> u64 {
        let start = offset as usize;
        page.get(start..start + 8)
            .and_then(|slot| slot.try_into().ok())
            .map_or(0, u64::from_le_bytes)
    };
    let counts = [
        get(at::STARTED),
        get(at::FAILED),
        get(at::ABSENT),
        get(at::UNSTARTED),
        get(at::SKIPPED),
        get(at::REFUSAL),
    ];
    if counts.iter().all(|count| *count == 0) {
        return None;
    }
    Some((get(at::DIGEST), counts))
}

/// A place's occupant handed a core.
///
/// A line of its own rather than a field on `supervisor ok`, because this is the
/// sentence `kernel/src/runtime.rs` has carried since RFC 0033 becoming false,
/// and a reader comparing two boots across the change should meet it where it
/// happened rather than in a summary at the end.
///
/// # Why the life is on the line, and how to read `never announced`
///
/// Because announcing is what *one* life does. `f_abi::door::ANNOUNCE` is the
/// first thing a component entered at selector zero says, and for a long time
/// selector zero was the only selector the frame ever asked for — so *it never
/// announced itself* meant *it did not get that far*. It does not mean that any
/// more: a driver asked to read its device and stop announces nothing, and the
/// line that carries its evidence is [`identified_line`] below.
///
/// So the life is printed, and the two readings are: a zero life that never
/// announced is a component that did not reach its first door, and any other
/// life that never announced is a component doing what it was asked.
///
/// # Why it names the place, having said `supervisor`
///
/// Because there can be two of these in one boot now. The supervisor's is the
/// first and is on every boot that has a second core; the other is the occupant
/// of the place `blk=place` supplies, and it is on that boot alone. A line that
/// named neither would make a reader count the `scheduled` lines to find out
/// which occupant reached ring 3 — and not counting is what the name is for.
/// # Why no tick count, having had one
///
/// Because it is time-derived and this line is in the log `cargo xtask trace`
/// hashes. `kernel/src/state.rs` wrote the rule down at E0-B14 — *nothing
/// time-derived is published, deliberately: a tick count would make two runs of
/// one commit disagree for a reason that has nothing to do with the kernel* —
/// and this line broke it from the day it was written.
///
/// It got away with it for exactly one increment. While the occupant announced
/// itself and ended, the count was zero or one and two boots agreed by luck;
/// the moment the occupant read a board, adopted a ring and submitted a spawn,
/// it became a measurement of how busy the host was, and `trace` went red with
/// two hashes that differed. That is the check working. The number it caught was
/// never evidence of anything — *did it reach ring 3* is what this line is for,
/// and `announced` answers that.
fn scheduled_line(
    what: Name<'_>,
    cpu: usize,
    life: u32,
    announced: bool,
    death: crate::process::Death,
) {
    crate::kprintln!(
        "  scheduled     place {what} on core {cpu} for life {life} — a place's occupant given a \
         core; it {} itself from ring 3",
        if announced { "announced" } else { "never announced" },
    );
    // How it ended, named rather than coded. The first time an occupant was
    // scheduled this said *killed*, and a status number alone would have left a
    // reader guessing which of the ways a component can die it had been — which
    // is the difference between a boot that reports and one that hints.
    match death {
        crate::process::Death::Exited(status) => {
            crate::kprintln!("  occupant      ended itself with status {status}");
        }
        crate::process::Death::Killed { vector, error, address, rip } => {
            crate::kprintln!(
                "  occupant      killed: exception {vector} at {address:#018x}, error {error:#x}, \
                 rip {rip:#018x}"
            );
        }
        crate::process::Death::Running => {
            crate::kprintln!("  occupant      never started, which the frame cannot explain");
        }
    }
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

/// The layout of a supervisor's board is `f_supervisor::routing`'s, and this is
/// where the two definitions are required to be one.
///
/// A comment would be a claim; this is a check, and the kernel is the artefact
/// that links both definitions. The same arrangement `kernel/src/blk.rs` has for
/// the three drivers and `ring/src/heap.rs` for the heap.
const _: () = assert!(crate::process::BOARD == f_supervisor::routing::AT);
const _: () = assert!(f_supervisor::routing::BYTES as u64 == FRAME_SIZE);

/// Give a supervisor the endpoint of a place, so that it can be told when that
/// place loses its occupant.
///
/// **Called before the death, never after**, and [`write_board`] argues why at
/// the parameter that carries the result: a `PEER_GONE` pends in the slot of the
/// endpoint it concerns, so a supervisor granted one afterwards holds a handle
/// to a place whose death it has already missed.
///
/// `REVOKE` beside `READ` and `WRITE`, because this is also the handle
/// `op::STOP` is checked against — one grant answers both halves of a lifecycle
/// rather than two grants answering one each. Deliberately **not** `GRANT`: a
/// supervisor that could hand a place's endpoint on could introduce a client to
/// a place without the frame knowing, which is the powerbox's job.
///
/// # Errors
///
/// As [`grant_into`].
fn watch(occupant: &mut Instance, frames: &FrameAllocator) -> Result<Handle, Failure> {
    // **Re-armed, because a supervisor runs more than once.** `Table::clear_all`
    // turns notice-owing off on its way out — *nothing is owed to a component
    // that no longer exists*, which is right for a teardown and wrong for an
    // occupant that has merely finished a run and will be given another core.
    // Without this the grant below succeeds, the death is posted, and
    // `note_peer_gone` records nothing because the table says nobody is
    // listening: the boot reported `told of 0 death(s)` for a place that had
    // demonstrably died, twice, before this line existed.
    occupant.table.owes_notices();
    grant_into(
        &mut occupant.table,
        frames,
        CapType::Endpoint,
        rights::READ | rights::WRITE | rights::REVOKE,
        0,
        0,
    )
}

/// Fill in the page the supervisor reads before it does anything else.
///
/// Written through the direct map, into a frame this instance's account paid for
/// and this frame mapped at [`crate::process::BOARD`], **before the core is told
/// to run**. That ordering is the whole reason the component may believe it, and
/// it is why the magic goes in last: a supervisor that read a half-written board
/// would be a supervisor spawning out of somebody else's arithmetic.
///
/// # Errors
///
/// [`Failure::WrongPlace`] for an occupant with no board — a component that
/// declared no `board` need is not a supervisor, and filling in a page it never
/// asked for would be a write into the direct map at offset zero.
///
/// # Safety
///
/// `occupant.board` must be the direct-map address of a frame this instance owns
/// and nothing else references, and the core that will read it must not have
/// been started yet.
#[expect(
    clippy::too_many_arguments,
    reason = "seven are what a consultation is made of — the allocator, the occupant, the place \
              it is about, the account it may spend, the table the grants land in, the endpoint \
              it watches and the tick it is stamped with — and the eighth is the generation this \
              machine was asked to be. A struct would be a type that exists so that a lint passes, \
              unpacked at the only call site there is"
)]
unsafe fn write_board(
    frames: &FrameAllocator,
    occupant: &mut Instance,
    target: &Place,
    account: &Account,
    supervisor: &Table,
    watches: Handle,
    now: u64,
    generation: Option<Generation>,
) -> Result<(), Failure> {
    use f_supervisor::routing::at;

    if occupant.board == 0 {
        return Err(Failure::WrongPlace);
    }

    // --- the powerbox grant -------------------------------------------------
    //
    // The account the held-open place was staked with, granted **into the
    // supervisor's own table** so that the spawn it submits names a capability
    // it holds. RFC 0008 calls this the powerbox grant and it is the whole
    // difference between a supervisor and a component that types an index: the
    // frame resolves `entry.cap` in the submitter's table, so a supervisor that
    // had not been handed this would be refused `AUTHORITY/RIGHT_NOT_HELD` —
    // which is exactly what the first boot of this path was, and it was the
    // check working rather than the check failing.
    //
    // `GRANT` beside the four the account already carries, because spending an
    // `Untyped` on somebody else's behalf is handing authority on, and that is
    // the right which names doing so. `Serving::spawn` requires it by name.
    //
    // Granted from what the *frame's* table says the object is, rather than from
    // `Account::floor` and a length computed here: one reading, from the place
    // that owns it.
    let found = supervisor.inspect(account.handle).map_err(Failure::Need)?;
    let spends = grant_into(
        &mut occupant.table,
        frames,
        CapType::Untyped,
        rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE | rights::GRANT,
        found.object,
        found.extent,
    )?;

    // --- and the endpoint, which is what makes a supervisor tellable ---------
    //
    // **Handed in rather than granted here**, and that is the whole of RFC
    // 0076's ordering made into a parameter. A place's `PEER_GONE` pends in the
    // capability slot of the endpoint that names it, so the supervisor has to
    // hold that endpoint *before* the death — a handle minted at consultation
    // time would name a place whose death had already been posted somewhere
    // else, and the notice and the board would disagree about which row a death
    // belongs to. That is not a hypothesis: the first boot of this path reported
    // `told of 0 death(s)` for a place that had demonstrably died, because these
    // were two grants.
    //
    // [`watch`] is where it is minted, at the caller, before whatever kills the
    // occupant.
    // SAFETY: the caller's guarantee. One frame, mapped by the direct map, owned
    // by this instance, and not yet reachable from any other core.
    let page =
        unsafe { core::slice::from_raw_parts_mut(occupant.board as *mut u8, FRAME_SIZE as usize) };
    let mut put = |offset: u32, value: u64| {
        let start = offset as usize;
        if let Some(slot) = page.get_mut(start..start + 8) {
            slot.copy_from_slice(&value.to_le_bytes());
        }
    };
    put(at::CONTROL_AT, crate::process::SPAWN_CONTROL);
    put(at::CONTROL_LEN, FRAME_SIZE);
    // The supervisor's own handle for the account it may spend — the one granted
    // a few lines above, in *its* table. Not the frame's name for the same
    // object: the two tables number their slots separately, so a handle copied
    // across would name whatever happened to be in that slot on the other side.
    // And not `Instance::first`, which is this component's handle for its *own*
    // account — the memory it is made of, not the memory it may spend.
    put(at::ACCOUNT, u64::from(spends.bits()));
    put(at::PLACES, 1);
    put(at::NOW, now);
    put(at::ROW + at::ROW_MANIFEST, target.manifest.bits());
    put(at::ROW + at::ROW_ENDPOINT, u64::from(watches.bits()));
    // The tally, handed back out. The frame stored these and did not look at
    // them; RFC 0076 is the argument and `Budget`'s doc comment is where it
    // says so beside the field.
    put(at::ROW + at::ROW_USED, u64::from(target.budget.used));
    put(at::ROW + at::ROW_OPENED, target.budget.opened);
    // The generation, where this boot selected one. RFC 0094: the frame folded
    // these bytes to decide whether this machine is the generation `f.root=`
    // named, and the component that instantiates the topology needs the same
    // three facts — where, how long, and what it folds to.
    //
    // Written whether or not the occupant declared a `module` need. A supervisor
    // with no need has nothing mapped at `at`, and reads a length beside an
    // address it cannot reach; what stops it acting on that is its own
    // `module_len == 0` test, not the frame withholding the number. The frame
    // saying less than it knows is how a component comes to guess.
    if let Some(generation) = generation {
        // The *component's* address, which is a constant, and not
        // `generation.at`, which is the frame's. The two name the same bytes
        // through two page tables and a board carrying the wrong one would be a
        // component told to read the direct map.
        put(at::MODULE_AT, crate::process::SPAWN_MODULE);
        put(at::MODULE_LEN, generation.bytes);
        for (index, chunk) in generation.root.as_chunks::<8>().0.iter().enumerate() {
            put(at::ROOT + index as u32 * 8, u64::from_le_bytes(*chunk));
        }
    }
    // Last, so that a component reading a page this function did not finish
    // finds a zero rather than a plausible layout. Nothing here races — the core
    // is idle until `run_on` — and the order is kept anyway, because the day
    // something does race is the day nobody remembers this was safe. The same
    // sentence `kernel/src/blk.rs` writes over its own board.
    put(at::MAGIC, f_supervisor::routing::MAGIC);
    Ok(())
}

/// What the supervisor said it did, read out of the far half of its board.
///
/// RFC 0013's *read, never delivered*: the frame watching a component through
/// memory it granted, costing the component nothing and telling it nothing.
///
/// Answers zeroes for an occupant with no board, which is every component but
/// one. That is not a silent failure — the caller prints what this returns
/// beside what the *frame* counted, and the two disagreeing is the finding.
///
/// # Safety
///
/// As [`write_board`], except that the core must have finished rather than not
/// started.
unsafe fn read_board(occupant: &Instance) -> (u64, u64, u64, u64, Budget) {
    if occupant.board == 0 {
        return (0, 0, 0, 0, Budget::default());
    }
    use f_supervisor::routing::at;
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let get = |offset: u32| -> u64 {
        let start = offset as usize;
        page.get(start..start + 8)
            .and_then(|slot| slot.try_into().ok())
            .map_or(0, u64::from_le_bytes)
    };
    (
        get(at::TOLD),
        get(at::SUBMITTED),
        get(at::REFUSED),
        get(at::SAID + at::SAID_VERDICT),
        // Copied, not interpreted. The frame stores these two numbers between
        // consultations and never compares them to anything; RFC 0076 names this
        // as the seam and `Budget`'s doc comment is the standing answer.
        Budget {
            used: u32::try_from(get(at::SAID + at::SAID_USED)).unwrap_or(0),
            opened: get(at::SAID + at::SAID_OPENED),
        },
    )
}

/// What one consultation produced.
///
/// Two halves from two sides of the privilege boundary, kept apart in the type
/// because that is the only form in which either is evidence: `answered` and
/// `spawned` are what *this core* did, and `told`, `submitted` and `verdict` are
/// the supervisor's own account of itself out of its board.
struct Consulted {
    /// Did the occupant announce itself from ring 3?
    announced: bool,
    /// How it ended.
    death: crate::process::Death,
    /// Entries this core answered on its ring. Unit: entries.
    answered: u64,
    /// The last refusal among them, or zero. Unit: none — a packed error.
    refusal: i32,
    /// What the last spawn produced: epoch, frames charged, needs supplied.
    spawned: (u32, usize, usize),
    /// Deaths the supervisor says it was told about. Unit: notices.
    told: u64,
    /// Spawns it says it submitted. Unit: entries.
    submitted: u64,
    /// Spawns it could not submit, for want of room. Unit: entries.
    unsent: u64,
    /// What it decided about the place it was consulted over, as
    /// `f_supervisor::policy::Verdict::to_wire`.
    verdict: u64,
    /// The tally it left behind, to be stored on the place.
    budget: Budget,
}

/// Ask the supervisor what to do about one place, and let it act.
///
/// # What this is, in one sentence
///
/// RFC 0008's *restart is the supervisor's act and the frame provides only the
/// mechanism*, as a function: the frame writes down what it knows, hands a core
/// to the component that decides, and then performs whatever that component
/// asked for.
///
/// # The order, which is the whole of RFC 0076
///
/// The caller has already torn down whatever died and called [`publish`], so the
/// `notice::PEER_GONE` is **on the supervisor's ring before this is called**.
/// That is what makes a supervisor tellable without the frame ever writing a
/// running core's capability table: everything it needs to know is memory it can
/// read at its own polling point, put there while nothing was running.
///
/// Then, and only then, the ring is served — against the table the core handed
/// back. `serve`'s doc and `user/supervisor/src/component.rs` carry the argument
/// for why that cannot happen while the component runs.
///
/// # Errors
///
/// [`Failure::NoAnswer`] for a core that did not report finished, and whatever
/// the board, the grants and the ring refuse.
///
/// # Safety
///
/// `cpu` must be a core the caller has vouched is started and idle, and
/// `occupant` must be an instance this frame built whose address space is live.
/// The frame's side of a consultation: everything a spawn the supervisor asks
/// for would be built out of, and the core it is asked on.
///
/// A struct for `Serving`'s reason rather than for clippy's. These five borrows
/// and one core travel together through every consultation and are meaningless
/// apart — a `consult` taking them loose had twelve parameters, and the two that
/// are about *this* consultation (which place, which tally) were lost among the
/// ten that are about the machine.
struct Consulting<'a> {
    /// The generation this machine is, for the board. `None` on a boot that
    /// selected none, which is every boot with no `f.root=`.
    generation: Option<Generation>,
    /// Where the frames a spawn charges come from.
    frames: &'a mut FrameAllocator,
    /// The kernel's address space, which a new instance's tables are built
    /// against.
    kernel: &'a paging::AddressSpace,
    /// What the processor offers those tables.
    features: Features,
    /// Where handles are minted and what a teardown will walk. The frame's,
    /// until RFC 0029's cross-table parent link lands — `Serving::supervisor`
    /// carries the argument.
    supervisor: &'a mut Table,
    /// What a spawn's admission is tested against.
    reservations: &'a Reservations,
    /// The core the supervisor runs on, and its timestamp rate.
    on: (usize, u64),
}

/// The life a consultation asks a supervisor for.
///
/// Zero, which `f_abi::door::Entry` calls the life every component has. Named
/// rather than written as a literal now that the frame asks for more than one,
/// so that the two call sites cannot be read as the same number by accident.
const SUPERVISOR_LIFE: u32 = 0;

/// Write what this boot found into a driver occupant's routing page.
///
/// # Why the caller hands over a list and not a struct
///
/// Because what goes on this page is what *this boot* discovered about a device
/// on a bus, and the frame's component module has never walked a bus. The list
/// is built where the device was found — `main.rs` — and written here, where the
/// page lives. A struct would put the layout in a third place, and the layout is
/// already agreed in exactly two: `f_virtio_blk::routing` and the const
/// assertion in `kernel/src/blk.rs` that ties its address to
/// `process::BOARD`.
///
/// **The slots are written in the order given**, and the caller's last one must
/// be the magic. A component reads the magic first and believes nothing without
/// it, so a page whose writer stopped half way must not carry one — which is the
/// same ordering `kernel/src/blk.rs` keeps for the same reason, and the reason
/// this does not sort or reorder anything.
///
/// # Errors
///
/// [`Failure::WrongPlace`] for an occupant with no board. A component that
/// declares no `board` need has no page to be told anything on, and filling in
/// one it never asked for would be a write into the direct map at an address
/// this instance does not own.
///
/// # Safety
///
/// `occupant.board` must be the direct-map address of a frame this instance
/// owns, and no core may be inside this instance.
unsafe fn write_routing(occupant: &Instance, slots: &[(u32, u64)]) -> Result<(), Failure> {
    if occupant.board == 0 {
        return Err(Failure::WrongPlace);
    }
    // SAFETY: the caller's guarantee. One frame, mapped by the direct map, owned
    // by this instance, and not reachable from any other core.
    let page =
        unsafe { core::slice::from_raw_parts_mut(occupant.board as *mut u8, FRAME_SIZE as usize) };
    for (offset, value) in slots {
        let start = *offset as usize;
        let Some(slot) = page.get_mut(start..start + 8) else { return Err(Failure::WrongPlace) };
        slot.copy_from_slice(&value.to_le_bytes());
    }
    Ok(())
}

/// What a driver occupant wrote back into the far half of its routing page.
///
/// Answers the outcome it named and the capacity it read, or `None` for a page
/// with no report magic on it — which is a component that did not reach the end
/// of what it was asked, and is a different thing from one that reached it and
/// found nothing. A zero capacity under a present magic is a real reading of an
/// empty device; a zero capacity under no magic is no reading at all, and a
/// frame that could not tell them apart would report an empty disk for a
/// component that never ran.
///
/// # Safety
///
/// As [`write_routing`], and the core must have finished rather than not
/// started.
unsafe fn read_identified(occupant: &Instance) -> Option<(u64, u64)> {
    if occupant.board == 0 {
        return None;
    }
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let get = |offset: u32| -> Option<u64> {
        let start = offset as usize;
        Some(u64::from_le_bytes(page.get(start..start + 8)?.try_into().ok()?))
    };
    if get(f_virtio_blk::routing::reported::MAGIC)? != f_virtio_blk::routing::MAGIC {
        return None;
    }
    Some((
        get(f_virtio_blk::routing::reported::OUTCOME)?,
        get(f_virtio_blk::routing::reported::CAPACITY)?,
    ))
}

/// What a serving occupant tallied, read back off the far half of its routing
/// page.
///
/// The outcome it named, the entries it answered and the bytes it moved for its
/// client, or `None` for a page with no report magic on it — [`read_identified`]
/// argues that distinction one function up and it is the same one here: a zero
/// under a present magic is a driver that served nothing, and a zero under no
/// magic is no reading at all.
///
/// **These are the occupant's numbers and not the frame's**, which is the whole
/// of why the boot requires them. The frame can say it described two rings and
/// started a core; only the component can say it took entries off one of them.
///
/// # Safety
///
/// As [`write_routing`], and the core must have finished rather than not
/// started.
unsafe fn read_served(occupant: &Instance) -> Option<(u64, u64, u64)> {
    if occupant.board == 0 {
        return None;
    }
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let get = |offset: u32| -> Option<u64> {
        let start = offset as usize;
        Some(u64::from_le_bytes(page.get(start..start + 8)?.try_into().ok()?))
    };
    if get(f_virtio_blk::routing::reported::MAGIC)? != f_virtio_blk::routing::MAGIC {
        return None;
    }
    Some((
        get(f_virtio_blk::routing::reported::OUTCOME)?,
        get(f_virtio_blk::routing::reported::SERVED)?,
        get(f_virtio_blk::routing::reported::BYTES)?,
    ))
}

/// Hand a place's occupant a core, let it run to completion, and take the core
/// back.
///
/// # Why this is a function and not two copies of eleven lines
///
/// Because two callers hand a place's occupant a core now and only one of them
/// is a consultation. [`consult`] writes a board before and serves a ring after;
/// the supplied place in [`demonstrate`] does neither — it declares no `board`
/// need and submits nothing. What the two share is exactly the swap, and the
/// swap is the part where a mistake is silent rather than loud: a capability
/// table left behind on the core outlives this run and resolves the *next*
/// occupant's handles at the wrong generation. `process::schedule_occupant`'s
/// own comment records that happening and how far the symptom sat from the
/// cause, which is why the pair belongs in one place with one argument for it.
///
/// # To completion, which is a limit and is not a design
///
/// [`crate::smp::run_on`] waits for the core to report finished, so an occupant
/// started here has ended before this returns. That is enough for a component
/// whose whole life is to announce itself, and it is not enough for one that
/// serves: a driver answers a client's entries *while* the client submits them,
/// which is `smp::start_on` and not this. `CHAOS_GAP` in `xtask` carries the
/// difference and names what closing it costs.
///
/// # Errors
///
/// [`Failure::NoAnswer`] for a core that was given the occupant and never
/// reported back. The table is deliberately **not** taken off that core: a core
/// that did not answer may still be inside the component, and reading its shard
/// would be the frame guessing about a race it just lost.
///
/// # Safety
///
/// `cpu` must be a core the caller has vouched is started and idle, and
/// `occupant` must be an instance this frame built whose address space is live.
unsafe fn run_ring3(
    occupant: &mut Instance,
    kernel: &paging::AddressSpace,
    features: Features,
    on: (usize, u64),
    selector: u32,
) -> Result<(bool, crate::process::Death), Failure> {
    let (cpu, tsc_khz) = on;
    // Which of this component's lives the frame is asking for, and `first` is
    // the occupant's own handle for its account, so its first act can be a
    // capability call rather than a guess about which slot it holds.
    //
    // Zero is the life every component has and is what a consultation asks for.
    // A component that does not name the selector it was given falls through to
    // that one, which is why the frame may ask for another without knowing
    // whether this particular image understands it.
    // SAFETY: the caller's guarantee, passed down unchanged — this function is
    // the two halves below in sequence and adds nothing to what either asks.
    let previous = unsafe { post_ring3(occupant, features, cpu, selector) };
    // SAFETY: the boot processor, with the kernel's space in `CR3` and the job
    // published into `cpu`'s own shard above. `run_on` makes the `Release` store
    // that publishes it.
    let ran = unsafe { crate::smp::run_on(cpu, kernel.root(), tsc_khz, OCCUPANT_MICROS) };
    ran.map_err(|_| Failure::NoAnswer)?;

    // SAFETY: the core reported finished, which is what `Ok` above means.
    Ok(unsafe { collect_ring3(occupant, cpu, previous) })
}

/// Put an occupant's job in a core's shards and answer the table that was
/// there.
///
/// The half of [`run_ring3`] that happens *before* a core is told anything, and
/// it is split out because the two callers differ only in what they do between
/// this and [`collect_ring3`]: one waits, and one has a client to run.
///
/// # Safety
///
/// As [`run_ring3`], and the answer must be handed to [`collect_ring3`] for
/// this same core before this instance is touched again — a table left on a
/// core is the next occupant's handles resolving at the wrong generation.
unsafe fn post_ring3(
    occupant: &mut Instance,
    features: Features,
    cpu: usize,
    selector: u32,
) -> crate::cap::Table {
    // Which of this component's lives the frame is asking for, and `first` is
    // the occupant's own handle for its account, so its first act can be a
    // capability call rather than a guess about which slot it holds.
    //
    // Zero is the life every component has and is what a consultation asks for.
    // A component that does not name the selector it was given falls through to
    // that one, which is why the frame may ask for another without knowing
    // whether this particular image understands it.
    let argument = f_abi::door::Entry::new(selector, occupant.first).bits();
    let root = occupant.space.root();
    // SAFETY: `root` is the address space this frame built for this occupant and
    // nothing has torn it down; its text is mapped executable at `process::TEXT`
    // and its stack is writable below `process::SPAWN_STACK_TOP`, both by
    // `spawn`. `cpu` is the caller's, and the table handed over is taken back by
    // `collect_ring3` before this instance is touched again.
    unsafe {
        crate::process::schedule_occupant(
            cpu,
            root,
            features,
            occupant.table,
            argument,
            OCCUPANT_HZ,
            OCCUPANT_TICKS,
        )
    }
}

/// The frame's side of [`Killer`], for one occupant on one core.
///
/// Holds the address space root rather than the instance, because the instance
/// is borrowed by the caller for the whole of the run and this has to be handed
/// to the client at the same time. A root is what `fault_occupant` needs and is
/// all it needs.
struct Killing {
    /// The occupant's address space. Unit: bytes, physical.
    root: u64,
    /// The core it is on. Unit: none — a core index.
    cpu: usize,
    /// Unit: kilohertz.
    tsc_khz: u64,
    /// Whether the core has been joined, so the caller does not join it twice.
    joined: bool,
}

impl Killer for Killing {
    fn kill(&mut self, frames: &mut FrameAllocator) -> Option<f_ring::PeerGone> {
        if self.joined {
            return None;
        }
        // SAFETY: `root` is the address space `post_ring3` scheduled, this core
        // holds the kernel's space, and `frames` is on the direct map.
        unsafe { fault_occupant(self.root, frames) }.ok()?;
        // Served by nothing while it dies, and that is deliberate rather than a
        // shortcut. The one thing this core answers for a driver is a
        // translation, and a driver whose text has just been taken away is not
        // going to ask for one: it faults on the instruction after, whatever it
        // was doing. A `serve` here would be the frame answering a component
        // that no longer exists.
        //
        // SAFETY: `start_on` was called for this core by `serve_ring3` and
        // nothing else has joined it.
        let joined = unsafe {
            crate::smp::join_serviced(self.cpu, self.tsc_khz, OCCUPANT_MICROS, &mut || {})
        };
        joined.ok()?;
        self.joined = true;
        // The core has reported finished, so nothing of this component is
        // running anywhere. That is what the notice below says, and it is true
        // before it is said rather than because it was.
        f_ring::PeerGone::told(f_abi::control::notice::PEER_GONE)
    }
}

/// Read what the core recorded and take this instance's table back off it.
///
/// # Safety
///
/// The core must have **reported finished** rather than merely been asked to
/// stop: `smp::run_on` or `smp::join_serviced` returning `Ok` is what that
/// means. A core that did not answer may still be inside the component, and
/// both reads below would be the frame guessing about a race it just lost —
/// which is why neither caller reaches this on the path where it did not.
unsafe fn collect_ring3(
    occupant: &mut Instance,
    cpu: usize,
    previous: crate::cap::Table,
) -> (bool, crate::process::Death) {
    // What the core recorded, read before the shards are reused.
    // SAFETY: the caller's guarantee that the core reported finished.
    let (announced, death, _ticks) = unsafe { crate::process::occupant_outcome(cpu) };
    // The other half of the swap, taken back *before* anything else looks at
    // this instance. What comes back is not what went in: a component may have
    // derived, and a teardown has to revoke what it ended holding.
    // SAFETY: as above — the core is finished, so nothing over there is holding
    // either table.
    occupant.table = unsafe { crate::process::reclaim_occupant_table(cpu, previous) };
    (announced, death)
}

/// What a client wants done with the occupant it has just been run against.
///
/// # Why the client asks and the frame acts
///
/// Because only the client knows when there is work outstanding, and only the
/// frame may take a component's world away. `E1-P06` kills a driver **under
/// sustained load**, and the word that costs something is *under*: a kill
/// delivered when nothing is in flight is a restart, and a restart demonstrates
/// nothing about a blast radius. So the client says *now, and I have this many
/// outstanding*, and [`fault_occupant`] is what makes it true.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Drove {
    /// The client has nothing more to submit and the occupant may end.
    Finished,
    /// The client has work outstanding and wants the occupant **swapped** for
    /// its successor rather than killed.
    ///
    /// The difference between this and [`Self::Kill`] is the whole of RFC 0012's
    /// distinction, and it is what the two are never summed for: a kill takes a
    /// component away and a restart gives the clients a new one that has never
    /// heard of them, while a swap hands the successor the history its
    /// predecessor lived. A client observes a restart as *every registration I
    /// hold is gone*; it observes a swap as a pause.
    Swap {
        /// How many operations the client had submitted and not been answered.
        /// Unit: operations.
        in_flight: u32,
    },
    /// The client has operations the occupant has not answered, and wants it
    /// killed with them outstanding. Carries how many, which the boot requires
    /// to be at least one — a run that killed an idle driver would pass every
    /// other check in this demonstration and assert nothing.
    /// Unit: operations.
    Kill {
        /// How many operations the client had submitted and not been answered.
        /// Unit: operations.
        in_flight: u32,
    },
}

/// What a client calls to have the occupant it is talking to taken away.
///
/// # Why the client holds the trigger and the frame holds the authority
///
/// Because of a constraint the ring's own types impose, and it is worth stating
/// rather than working around. A buffer the device holds is an
/// `f_ring::InFlight`, and dropping one is a panic: a client that walked away
/// from a buffer a device was writing into would be a client whose page the
/// device scribbles on after it has been handed to somebody else. The only ways
/// out are the completion, which is not coming, and `InFlight::reclaim`, which
/// demands an `f_ring::PeerGone` — *evidence* the peer's outstanding tokens are
/// void.
///
/// So the reclaim has to happen while the client still holds its buffers, which
/// is inside one `drive` call, which means the kill has to happen there too. The
/// client says when; this says what it costs. [`Self::kill`] does not return
/// until the occupant's core has reported finished, so a client that gets a
/// `PeerGone` back is holding evidence rather than an assumption.
pub trait Killer {
    /// End the occupant now and wait for its core.
    ///
    /// `None` when the occupant could not be ended, or when its core did not
    /// report back inside the bound — in which case the client must **not**
    /// reclaim anything, because a core that said nothing may still be inside
    /// the component.
    fn kill(&mut self, frames: &mut FrameAllocator) -> Option<f_ring::PeerGone>;
}

/// A client the frame runs against a place's occupant while that occupant is
/// on a core.
///
/// # Why this is a trait and not a function in this file
///
/// Because this file has no business knowing what a block request is. What it
/// knows is the sequence — post the job, start the core, be the client, join
/// the core while serving it — and that sequence is the same whatever the
/// occupant serves. What the client *is* lives where its protocol does:
/// `kernel/src/blk.rs` for a disk, and the day a second driver serves from a
/// place, beside that one.
///
/// It is dependency injection rather than a callback, the distinction
/// `cargo xtask lint-callbacks` draws for `f_env::Env`: the implementor is
/// chosen by `main` at boot and called by this file, and nothing a peer sends
/// registers anything.
///
/// # Why every method is handed the allocator
///
/// Because [`demonstrate`] holds the only `&mut FrameAllocator` there is, and an
/// implementor that held its own would be a second borrow of it for the whole of
/// a demonstration that is still building and tearing down places around it.
/// Passing it per call is what keeps the frame's one allocator one.
pub trait Datapath {
    /// Be the client, once, while the occupant runs.
    ///
    /// Returns when the client has nothing more to submit. The occupant is
    /// still on its core at that moment and is joined afterwards, so a `drive`
    /// that returns without telling the occupant to stop is a core the boot
    /// never gets back — telling it is this method's job, for the same reason
    /// the entries were.
    ///
    /// Called once per *generation* of the occupant. A client that answers
    /// [`Drove::Kill`] is called again against the next one, with that
    /// generation's own ring addresses, and everything it still holds is its own
    /// to resubmit — which is the whole of what a restart costs a client, and
    /// the whole of what `E1-P06` measures.
    ///
    /// # Errors
    ///
    /// A message for the boot log. It is reported *after* the join rather than
    /// instead of it: a client that gave up does not entitle the frame to
    /// abandon a core that is still inside a component.
    fn drive(
        &mut self,
        frames: &mut FrameAllocator,
        wired: Wired,
        killer: &mut dyn Killer,
    ) -> Result<Drove, &'static str>;

    /// Answer whatever the occupant has asked the frame for, and nothing else.
    ///
    /// Called between mailbox polls for the whole of the join. The one thing a
    /// driver cannot do for itself is turn a client's capability into an address
    /// its device may use — it does not own the remapping unit — so it asks, and
    /// this is what answers. RFC 0047.
    fn serve(&mut self, frames: &mut FrameAllocator);

    /// What this client took while it ran and is still holding, in frames.
    ///
    /// Taken out of [`demonstrate`]'s leak check rather than added to it, and
    /// **reported rather than given back**. A translation the driver asked for is
    /// a walk that allocates page tables in the device's domain, and those
    /// frames are the domain's until the domain is released — which happens
    /// after this function returns, outside the window this check covers,
    /// because the domain was also built outside it. A client that freed them
    /// here would leave the demonstration with *more* frames than it started
    /// with, which the same equality catches and which is the harder direction
    /// to read.
    ///
    /// Two numbers rather than a tolerance, which is this file's habit and
    /// `blk::demonstrate`'s: a check with slack in it is a check that stops
    /// noticing the first frame.
    ///
    /// Unit: frames.
    fn retained(&mut self, frames: &mut FrameAllocator) -> u64;
}

/// Where a serving occupant's two rings and its board are, as kernel addresses.
///
/// Handed to [`Datapath::drive`] rather than found by it, because these are
/// `spawn`'s answers: the pages were mapped out of the occupant's own account
/// and the frame reaches them through the direct map. A client that went looking
/// would be a second thing that knows where a component's ring is.
#[derive(Clone, Copy)]
pub struct Wired {
    /// The occupant's control ring, which the frame produces on and the
    /// occupant consumes. Unit: bytes, a kernel address.
    pub control: u64,
    /// The occupant's data ring, whose server end is the occupant's and whose
    /// client end is this client's. Unit: bytes, a kernel address.
    pub data: u64,
    /// The occupant's routing page. Unit: bytes, a kernel address.
    pub board: u64,
    /// The core the occupant is running on. Unit: none — a core index.
    pub cpu: usize,
    /// This machine's timestamp-counter rate, for bounding a wait.
    /// Unit: kilohertz.
    pub tsc_khz: u64,
}

/// Map the transfer window into an occupant's address space.
///
/// Called for **both** generations, which is the whole of what a window is: one
/// run of memory at one address in two address spaces, for the length of phase A
/// and no longer.
///
/// # Where the memory comes from, and where RFC 0063 says it should
///
/// The frame allocates it here and frees it after the swap. RFC 0063 says it
/// should be bought out of the **incoming** instance's account — *the account
/// that pays is the account that survives* — and this build does not do that,
/// for an ordering reason rather than a principled one: the outgoing instance
/// writes the window before the incoming instance exists, so an account that has
/// not been staked yet cannot have paid for it.
///
/// A place keeps its account across generations, so the honest fix is to carve
/// the window from *that* — which needs the charge to reach the incoming
/// occupant's refund list rather than the frame's, and that is a change to
/// `spawn`'s charging rather than to this function. Stated here rather than left
/// for a reader to notice: **this is a deviation with an owner**, and the day it
/// is paid this comment goes with it.
///
/// # Errors
///
/// [`Failure::Space`] for a mapping that could not be made.
///
/// # Safety
///
/// `occupant` must be an instance this frame built whose address space is not in
/// any core's `CR3`, and `window` must be a frame the caller allocated and holds.
unsafe fn show_window(
    occupant: &mut Instance,
    frames: &mut FrameAllocator,
    features: Features,
    window: Frame,
) -> Result<(), Failure> {
    // SAFETY: the caller's guarantee. One frame, into a space this frame built
    // and nothing is executing in.
    unsafe {
        paging::map_user(
            frames,
            &mut occupant.space,
            crate::process::SWAP_WINDOW,
            window.addr(),
            UserPage::Data,
            features,
        )
    }
    .map_err(Failure::Space)
}

/// Ask a running occupant to hand over at its next quiescent point.
///
/// **One word, written while the component holds a core**, and that is not a
/// race: the page is shared memory, the component polls it at a point of its own
/// choosing, and nothing here waits for an answer. R05 is why it has to work this
/// way — nothing is *delivered* to a component while it runs, so the only thing
/// a frame can do is leave something where the component will look.
///
/// `user/virtio-blk/manifest.toml`'s `[transfer]` table named the point it looks
/// at, in advance: the top of the serve loop, where an empty `Pending` is the
/// whole of what the component has accepted and not answered.
///
/// # Errors
///
/// [`Failure::WrongPlace`] for an occupant with no board, which is a component
/// nobody can ask anything.
///
/// # Safety
///
/// As [`write_routing`], except that a core **may** be inside this instance —
/// which is the point. The word written is one the component only reads.
unsafe fn ask_hand_over(occupant: &Instance) -> Result<(), Failure> {
    if occupant.board == 0 {
        return Err(Failure::WrongPlace);
    }
    let at = f_virtio_blk::routing::at::HAND_OVER as usize;
    // SAFETY: the caller's guarantee. One eight-byte word inside a frame this
    // instance owns and the direct map covers, written volatile because the
    // reader is another core.
    // SAFETY: `board` is a frame this instance owns and the direct map covers;
    // `at` is inside it and eight-byte aligned, so the offset stays in the page.
    let slot = unsafe { (occupant.board as *mut u8).add(at).cast::<u64>() };
    // SAFETY: as above. Volatile because the reader is another core, and one
    // word because a component reading a half-written flag is a component told
    // something nobody said.
    unsafe { slot.write_volatile(1) };
    Ok(())
}

/// Tell an occupant where its transfer window is and how much to replay out of
/// it.
///
/// Written after [`write_routing`] and before the core is started, which is
/// after the magic rather than before it. That is safe for the one reason the
/// magic exists: it guards against a component reading a page the frame *did not
/// finish*, and this page is finished — nothing has run yet, and the component
/// that will read it has not been given a core.
///
/// # Errors
///
/// As [`ask_hand_over`].
///
/// # Safety
///
/// As [`write_routing`].
unsafe fn write_swap(occupant: &Instance, at: u64, bytes: u64, replay: u32) -> Result<(), Failure> {
    use f_virtio_blk::routing::at as slot;
    // SAFETY: the caller's guarantee, and every offset below is inside the page.
    unsafe {
        write_routing(
            occupant,
            &[(slot::WINDOW_AT, at), (slot::WINDOW_LEN, bytes), (slot::REPLAY, u64::from(replay))],
        )
    }
}

/// What a component said about a hand-over it was asked for.
///
/// `(it held nothing, records it wrote)`. Read off the far half of its board
/// after its core has come back.
///
/// # Safety
///
/// As [`read_served`].
unsafe fn read_handed(occupant: &Instance) -> (bool, u32) {
    if occupant.board == 0 {
        return (false, 0);
    }
    use f_virtio_blk::routing::reported;
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let get = |offset: u32| -> u64 {
        let start = offset as usize;
        page.get(start..start + 8)
            .and_then(|slot| slot.try_into().ok())
            .map_or(0, u64::from_le_bytes)
    };
    (get(reported::QUIESCENT) != 0, u32::try_from(get(reported::RECORDS)).unwrap_or(0))
}

/// How many records the incoming occupant replayed into its own table.
///
/// **Counted on the far side of the boundary** and compared against what the
/// outgoing occupant said it wrote. Two tallies of one number, neither derived
/// from the other, which is `claims/0012`'s discipline: a swap that compared a
/// count against itself would report *nothing was lost* on a build where nothing
/// was transferred either.
///
/// # Safety
///
/// As [`read_served`].
unsafe fn read_replayed(occupant: &Instance) -> u32 {
    if occupant.board == 0 {
        return 0;
    }
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let start = f_virtio_blk::routing::reported::REPLAYED as usize;
    let word = page
        .get(start..start + 8)
        .and_then(|slot| slot.try_into().ok())
        .map_or(0, u64::from_le_bytes);
    u32::try_from(word).unwrap_or(0)
}

/// Which build of a component wrote a board, as the component itself says.
///
/// # Safety
///
/// As [`read_served`].
unsafe fn read_generation(occupant: &Instance) -> u64 {
    if occupant.board == 0 {
        return 0;
    }
    // SAFETY: the caller's guarantee.
    let page =
        unsafe { core::slice::from_raw_parts(occupant.board as *const u8, FRAME_SIZE as usize) };
    let start = f_virtio_blk::routing::reported::GENERATION as usize;
    page.get(start..start + 8).and_then(|slot| slot.try_into().ok()).map_or(0, u64::from_le_bytes)
}

/// Take an occupant's text away from it while it is running, so that its next
/// instruction faults.
///
/// **This is the kill `E1-P06` is about, and it is asynchronous by
/// construction.** Everything else in this tree that ends a component is
/// something the component did: a fault it took, an exit it chose, a stop it was
/// asked for and agreed to. None of those is a driver dying under a client's
/// load, because all of them are cooperative in the one way that matters — the
/// component reached a point of its own choosing. A driver that crashes does
/// not.
///
/// So the frame withdraws the mapping and tells every other core to forget it.
/// The running core's next instruction fetch faults, `process::kill` records a
/// `Death::Killed` in that core's own shard, and the core reports finished —
/// which is what `smp::join_serviced` is already waiting for. Nothing new
/// happens on the far side at all: this is the ordinary ring-3 fault path,
/// reached on purpose.
///
/// # Why the text and not the rings
///
/// Because the point is to kill the *component*, not to corrupt its work. Taking
/// a ring away would fault it too, and would also destroy the evidence — the
/// client's outstanding entries are on those pages, and a demonstration that
/// unmapped them could not then say whether the entries were lost by the kill or
/// by the killer. Text is the one thing a running component touches that holds
/// none of its state.
///
/// All sixteen pages, and not the first: `process::TEXT_PAGES` is the whole
/// mapping and a serving loop is not obliged to be in the first page of it. One
/// page would make this a kill that usually works, which is the worst kind.
///
/// # Errors
///
/// [`Failure::Space`] where the mapping was not there to withdraw, and
/// [`Failure::NoAnswer`] where a core did not acknowledge the shootdown — which
/// a caller cannot recover from, because an unacknowledged shootdown is a core
/// still executing through a translation that has been taken away.
///
/// # Safety
///
/// `occupant` must be an instance this frame built, `frames` must be rebound
/// onto the direct map, and this core must hold the kernel's address space —
/// the space being edited is the occupant's and is not in this core's `CR3`.
unsafe fn fault_occupant(root: u64, frames: &FrameAllocator) -> Result<(), Failure> {
    // **Every page that is there, and a component's text is shorter than the
    // reservation.** `process::TEXT_PAGES` is how much address space the frame
    // sets aside for text; `spawn` maps as many pages as the image actually
    // occupies, which for every component in this tree is fewer. So a page that
    // was never mapped is not a failure here — it is the end of the image — and
    // a version of this function that refused on the first one killed nothing at
    // all and reported that the core had wedged.
    let mut withdrawn = 0;
    for page in 0..crate::process::TEXT_PAGES as u64 {
        let at = crate::process::TEXT + page * FRAME_SIZE;
        // SAFETY: the caller's guarantee. `root` is a top-level table this
        // module built for this occupant, `frames` is on the direct map, and
        // this core holds the kernel's space.
        if unsafe { paging::unmap_user_live(frames, root, at) }.is_err() {
            continue;
        }
        withdrawn += 1;
        // Told after each page rather than once at the end, because a core that
        // faulted on page three while page four was still cached would be a kill
        // that landed for a reason this function cannot name.
        // SAFETY: the boot processor, with interrupts enabled — which is what
        // lets the core being told answer.
        unsafe { crate::smp::shootdown(at) }.map_err(|_| Failure::NoAnswer)?;
    }
    // Nothing withdrawn is nothing killed, and it must not be reported as a
    // component that wedged: an occupant whose text was already gone is a frame
    // bug one step earlier.
    if withdrawn == 0 {
        return Err(Failure::WrongPlace);
    }
    Ok(())
}

/// Hand a place's occupant a core, run a client against it, and take the core
/// back.
///
/// The other shape of [`run_ring3`], and the difference is the whole of
/// `E1-B05`'s third act: `run_on` waits, so an occupant scheduled through that
/// function has ended before its caller's next line. A driver is not
/// self-contained and cannot be — it answers entries *while* its client submits
/// them — so this posts the job, hands control back to the caller to be the
/// client, and then joins the core while **serving** it, which is
/// `smp::join_serviced` and the reason that function takes a closure.
///
/// # Why the client is a closure and not a second core
///
/// Because the frame is the client here, and the frame is this core. `drive` is
/// called with the occupant already running on `cpu`, so the two make progress
/// against each other over the two rings the routing page named — which is
/// exactly what `kernel/src/blk.rs` does for the instance it stands up outside
/// any place, and is the sequence this function exists to make available to an
/// instance that is *in* one.
///
/// `serve` is then called between mailbox polls for the whole of the join. The
/// one thing a driver cannot do for itself is turn a client's capability into
/// an address its device may use — it does not own the remapping unit — so it
/// asks on its control ring, and that ask is answered here, on the core where
/// the unit and the client's table already are. RFC 0047.
///
/// # Errors
///
/// [`Failure::NoAnswer`] for a core that never reported finished, whether
/// because the occupant wedged or because the bound was shorter than the work.
/// As [`run_ring3`], the table is deliberately **not** taken back on that path.
/// A `drive` that refused is answered *beside* the outcome rather than instead
/// of it: a client that gave up does not entitle the frame to abandon a core
/// that is still inside a component, so the join happens either way and the
/// caller decides which news matters.
///
/// # Safety
///
/// As [`run_ring3`], and [`Datapath::serve`] must touch nothing the running core
/// is writing — which for the one implementor means the occupant's control ring,
/// whose two ends are single-producer and single-consumer by construction.
unsafe fn serve_ring3(
    occupant: &mut Instance,
    frames: &mut FrameAllocator,
    features: Features,
    on: (usize, u64),
    selector: u32,
    client: &mut dyn Datapath,
) -> Result<(bool, crate::process::Death, Result<Drove, &'static str>), Failure> {
    let (cpu, tsc_khz) = on;
    let wired = Wired {
        control: occupant.control,
        data: occupant.data,
        board: occupant.board,
        cpu,
        tsc_khz,
    };
    // SAFETY: the caller's guarantee.
    let previous = unsafe { post_ring3(occupant, features, cpu, selector) };
    // SAFETY: `cpu` is a core the caller vouched is started and idle, and the
    // job it is about to take was published into its own shard above. The core
    // is not this one — `start_on` refuses that case rather than deadlocking.
    let started = unsafe { crate::smp::start_on(cpu) };
    if started.is_err() {
        // Nothing was taken, so nothing is holding the table: put it back before
        // answering, or this instance's handles stay on a core that never ran
        // and the teardown revokes the wrong set.
        // SAFETY: the core never took the job, so it is not inside this
        // component and its shards are the frame's to read.
        let _ = unsafe { collect_ring3(occupant, cpu, previous) };
        return Err(Failure::NoAnswer);
    }

    // The client, on this core, while the occupant runs on that one — and the
    // trigger it may pull, which is the only thing in this file that ends a
    // component while it is still useful.
    let mut killer = Killing { root: occupant.space.root(), cpu, tsc_khz, joined: false };
    let driven = client.drive(frames, wired, &mut killer);
    // And the hand-over, if that is what it asked for. Asked and not taken
    // away: the occupant ends itself, at a point it chose, having written what
    // its successor needs — which is the whole difference between a swap and
    // the kill `Killing` performs.
    //
    // SAFETY: `occupant` is the instance the caller vouched for. A core is
    // inside it, which is exactly the case `ask_hand_over` is written for.
    if matches!(driven, Ok(Drove::Swap { .. })) {
        // SAFETY: `occupant` is the instance the caller vouched for, and a core
        // being inside it is exactly the case this function is written for.
        unsafe { ask_hand_over(occupant) }?;
    }

    // Joined here only if the client did not have it joined for it. A second
    // `join_serviced` on a core already put back to `READY` would read the
    // mailbox as a refusal and turn a successful kill into `NoAnswer`.
    if !killer.joined {
        // SAFETY: `start_on` was called for this core and nothing else has
        // joined it; `serve` touches only the occupant's control ring, whose two
        // ends are single-producer and single-consumer by construction.
        let joined = unsafe {
            crate::smp::join_serviced(cpu, tsc_khz, OCCUPANT_MICROS, &mut || client.serve(frames))
        };
        joined.map_err(|_| Failure::NoAnswer)?;
    }

    // SAFETY: the core reported finished, which is what `Ok` above means.
    let (announced, death) = unsafe { collect_ring3(occupant, cpu, previous) };
    Ok((announced, death, driven))
}

unsafe fn consult(
    asking: &mut Consulting,
    occupant: &mut Instance,
    target: &mut Place,
    account: &Account,
    watches: Handle,
    now: u64,
) -> Result<Consulted, Failure> {
    let Consulting {
        generation,
        frames,
        kernel,
        features,
        supervisor,
        reservations,
        on: (cpu, tsc_khz),
    } = asking;
    let generation = *generation;
    let (features, cpu, tsc_khz) = (*features, *cpu, *tsc_khz);
    // Everything the component needs to know that is not a constant: its ring,
    // the account it may spend, the place it may act on, and the tally the frame
    // is holding for it. Written *before* the core is told to run, which is the
    // whole of why it may be believed.
    // SAFETY: `occupant.board` is a frame this frame allocated for this instance
    // and mapped into its address space and nobody else's; the direct map covers
    // it and the core that will read it is idle.
    unsafe {
        write_board(frames, occupant, target, account, supervisor, watches, now, generation)
    }?;

    // What the supervisor holds at the moment it is started, kept for the server
    // below to resolve its entries against. Taken here — after the grants above
    // and before the core is told to run — because that is precisely the state
    // the question *may the submitter spend this?* is about. The table the core
    // hands back is not it: an occupant's table is cleared when it ends.
    let submitted_from = occupant.table;

    // SAFETY: the caller's guarantee, passed down — `cpu` is started and idle
    // and this instance's address space is live.
    let (announced, death) =
        unsafe { run_ring3(occupant, kernel, features, (cpu, tsc_khz), SUPERVISOR_LIFE) }?;
    // SAFETY: the core is finished, and the page is still mapped — an address space is torn
    // down by `tear_down` and not here.
    let (told, submitted, unsent, verdict, budget) = unsafe { read_board(occupant) };

    let asks = Consumer::new(occupant.ring.channel()).ok_or(Failure::WrongPlace)?;
    let answers = Poster::new(occupant.ring.completions()).ok_or(Failure::WrongPlace)?;
    let mut serving = Serving {
        place: target,
        // The supervisor's own table as it stood when it submitted.
        submitter: &submitted_from,
        supervisor,
        frames,
        account,
        kernel,
        features,
        reservations,
        spawned: (0, 0, 0),
        answered: 0,
    };
    let (_, refusal) = serve(&asks, &answers, &mut serving)?;
    let answered = serving.answered;
    let spawned = serving.spawned;
    Ok(Consulted {
        announced,
        death,
        answered,
        refusal,
        spawned,
        told,
        submitted,
        unsent,
        verdict,
        budget,
    })
}

/// Answer everything the supervisor has asked for, and nothing else.
///
/// **The frame's polling point.** R05: nothing is delivered asynchronously, and
/// what happens here is this core looking at a ring in its own loop while
/// another core is inside a component. `kernel/src/supervisor.rs`'s
/// `Supervising::serve` is the same function for a driver's ring, and the two
/// are deliberately not merged: what they share is a loop and what they differ
/// in is every refusal, so one function with a mode would be one function that
/// could answer a spawn out of a driver's authority.
///
/// # Errors
///
/// [`Failure::Ring`] for a ring that stopped validating, or one with no room for
/// an answer — an operation performed and then not answered is a supervisor
/// waiting forever for a reply that was dropped on the floor, so the room is
/// checked *before* the entry is acted on.
/// Answers how many entries were served and the last refusal among them, or
/// zero for a run in which nothing was refused. **The refusal is returned rather
/// than only posted**, because the submitter of these entries has already ended
/// and cannot read its own completions — so a refusal that went only onto the
/// ring would be a refusal nothing in the machine ever reports.
fn serve(asks: &Consumer, answers: &Poster, serving: &mut Serving) -> Result<(u32, i32), Failure> {
    let broken = || Failure::Ring(error::pack(error::PEER, error::peer::GONE));
    let mut answered = 0;
    let mut refusal = 0;
    loop {
        let Some(entry) = asks.pop().map_err(|_| broken())? else { return Ok((answered, refusal)) };
        if answers.free().map_err(|_| broken())? == 0 {
            return Err(broken());
        }
        let answer = serving.execute(&entry);
        if answer.result != 0 {
            refusal = answer.result;
        }
        answers.post(answer).map_err(|_| broken())?;
        answered += 1;
    }
}

/// What the supervisor asked for and what it says it did.
///
/// Two counts from two sides of the privilege boundary on one line, because
/// that is the only form in which either is evidence. The frame's `answered` is
/// what this core actually served; `submitted` and `filled` are the component's
/// own account of itself out of its board. A boot where they disagree has a
/// supervisor that cannot see its own ring or a frame that answered something
/// nobody asked — and a line carrying only one of them could not tell you which.
/// What the supervisor assembled, and what it did with it.
///
/// **`E2-B05`'s exit, as a line a boot prints.** That task's first clause is
/// *boot is a pure function of one hash — the same root produces a byte-identical
/// topology*, and it has been demonstrated since E2 over an assembly built on a
/// host. What it was not was a property of a *boot*: nothing on the machine
/// instantiated anything, and `user/assembler`'s own module head said so.
///
/// The digest is `f_assembler::render::digest` over the topology the component
/// assembled — its decisions, not its input. A digest over the module would be
/// the module's own hash arriving by a longer route, which is the comparison
/// `user/assembler/src/render.rs` spends a page refusing.
///
/// Skipped is printed beside started and never folded into it. The frame holds
/// one place open and fills the rest itself, so most of a six-member topology is
/// a member this component correctly did not try — and a count that summed the
/// two would make a boot that started nothing look like a boot that started
/// everything.
fn assembled_line(digest: u64, counts: [u64; 6]) {
    let [started, failed, absent, unstarted, skipped, refusal] = counts;
    crate::kprintln!(
        "  assembled     topology {digest:#018x} — {started} started, {failed} failed, \
         {absent} with no device, {unstarted} left unstarted behind one, {skipped} skipped \
         as already filled by the frame; refusal {refusal}"
    );
}

fn supervised_line(consulted: &Consulted, filled: u32) {
    crate::kprintln!(
        "  supervised    told of {} death(s); decided {}; submitted {} spawn(s) from ring 3 and \
         could not submit {} — the frame answered {}, filled {} place(s), last refusal {:#010x}",
        consulted.told,
        verdict_label(consulted.verdict),
        consulted.submitted,
        consulted.unsent,
        consulted.answered,
        filled,
        consulted.refusal as u32,
    );
}

/// A restart, named for what decided it.
///
/// The line used to carry the backoff the frame's own `policy::decide` had
/// returned. It carries the *verdict* now, and not the pause, because the pause
/// was only ever interesting as evidence that a policy had run — and a verdict
/// that came back across a privilege boundary is better evidence of that than a
/// number the frame computed for itself. The backoff is the supervisor's, and it
/// is in the tally on its board.
fn restart_line(record: &Record, place: &Place, verdict: u64) {
    crate::kprintln!(
        "  restart       place {} under {} — the supervisor said {}; restart {} of {}",
        Name(record.label()),
        restart::label(record.restart),
        verdict_label(verdict),
        place.budget.used,
        record.max_restarts,
    );
}

/// A verdict ordinal as the word a reader wants.
///
/// The mapping is `f_supervisor::policy::Verdict::to_wire`'s, and it is written
/// out here rather than imported because what crosses the board is a number: a
/// frame that could pattern-match the supervisor's enum would be a frame that
/// links the supervisor's policy, which is the thing RFC 0008 spent three epochs
/// getting rid of. An ordinal this build does not name is printed as such
/// (R04) — a supervisor speaking a vocabulary the frame does not have is
/// something a boot log should say rather than round down to *leave*.
const fn verdict_label(wire: u64) -> &'static str {
    match wire {
        0 => "leave",
        1 => "restart",
        2 => "retire",
        _ => "a verdict this build does not name",
    }
}

/// Which module is the supervisor, by its manifest's label.
///
/// A label and not an index, for the reason `held_open` gives about the loader's
/// order, and a label and not a content hash because the hash is a property of
/// this build and the label is a property of the component — a supervisor
/// recompiled is still the supervisor.
fn supervising(modules: &[&'static [u8]; PLACES_MAX], count: usize) -> Option<usize> {
    (1..count).find(|index| {
        modules
            .get(*index)
            .copied()
            .is_some_and(|module| Record::read(module).is_ok_and(|r| r.label() == SUPERVISOR))
    })
}

/// Which place the frame builds and leaves for the supervisor to fill.
///
/// # Why this is one place and why it is that one
///
/// One, because the bootstrap argument only buys what it costs: a frame that
/// held every place open would be a frame betting the whole boot on a component
/// that has been executing for two commits. The interesting claim is that a
/// spawn can come from above the frame at all, and one place demonstrates it as
/// completely as four.
///
/// [`SUPERVISED`] and not *the next one along*, and three things pick it.
///
/// **It cannot be the first place.** That one is built before this loop and is
/// the subject of every scripted demonstration below — the connect that pends,
/// the fault, the restart, the retirement. Holding it open would not be an
/// experiment about who spawns; it would be deleting the rest of the boot.
///
/// **It cannot be a place this boot then schedules.** The occupant would be
/// running while the frame was still answering the supervisor's ring for it.
///
/// **Of the three that remain it is the one nothing else stands up.** All three
/// drivers are spawned into places here and then, on the datapath boots, stood
/// up *outside* those places by `prepare_driver` — which is `CHAOS_GAP`, still
/// declared and still true. `virtio-gpu` is the one whose datapath boot this
/// change cannot perturb, and holding its place open points at where that gap
/// closes rather than away from it.
///
/// The earlier version of this comment named `user/store` and argued that a
/// driver would entangle the evidence. It was wrong twice over: `store` is the
/// first place, so it was never available, and the entanglement runs the other
/// way — a driver's place is exactly the one a supervisor should end up filling.
///
/// Answers `None` when there is no supervisor to fill it, no such module, or no
/// worker core — in each of which cases the frame spawns everything, exactly as
/// it did before this change, and the boot is the boot it always was.
fn held_open(modules: &[&'static [u8]; PLACES_MAX], count: usize, worker: bool) -> Option<usize> {
    if !worker || supervising(modules, count).is_none() {
        return None;
    }
    (1..count).find(|index| {
        modules
            .get(*index)
            .copied()
            .is_some_and(|module| Record::read(module).is_ok_and(|r| r.label() == SUPERVISED))
    })
}

/// A place built, admitted, and deliberately left empty.
///
/// Its own line rather than a silence, because a place with no occupant and a
/// place that failed to get one look identical in a log that only prints
/// successes — and this boot now contains one of each on purpose. A reader who
/// cannot tell them apart cannot read the evidence for RFC 0008 either way.
///
/// It carries no content hash, which is what keeps `xtask`'s `spawned_from`
/// honest: that check counts the manifests a boot *spawned*, and this place has
/// not been spawned into yet. The line that follows it when the supervisor
/// succeeds is an ordinary [`spawned_line`], carrying the hash, printed from the
/// ring server — so the count is right without the check knowing which side
/// submitted.
fn held_line(record: &Record, staked: u64) {
    crate::kprintln!(
        "  held          {} — place built and admitted against a {} B account, occupant left to \
         the supervisor (RFC 0008)",
        core::str::from_utf8(record.label()).unwrap_or("?"),
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

/// What the frame found when it followed its own root into a component's tree.
///
/// A fourth line per spawn, after [`spawned_line`], and it is the evidence
/// E1-B15's first clause is about: the mount, the node count and the snapshot
/// the frame read back *through* the root rather than out of what it had
/// written. A boot that printed only *a tree was published* would be making the
/// claim; this prints the reading.
///
/// The snapshot of a tree whose component has not run is the hash of a block of
/// zeros under this schema, which is a constant for a manifest — so it is a
/// fixture number and the boot log stays reproducible. The day a component
/// writes into its own tree before this line, it stops being one, and that is
/// the day it says something a reader cares about.
fn mounted_line(record: &Record, place: &Place, found: (u64, [u8; 16], usize)) {
    let (snapshot, label, width) = found;
    let nodes = place.occupant.as_ref().map_or(0, |occupant| occupant.tree_nodes);
    crate::kprintln!(
        "  state mount   place {} -> frame root slot {}: root {}, {} node(s), snapshot \
         {snapshot:#018x}, read back through the root and not from what was written",
        Name(record.label()),
        place.slot,
        Name(label.get(..width).unwrap_or(&[])),
        nodes,
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
    supplied: &[Supplied],
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

    // The fixed parts, after the text and in that order, because the order is
    // what a teardown gives them back in: the stack's pages lowest-first, then
    // the control ring, then the state tree.
    let mut fixed = [0u64; FIXED_PARTS];
    for slot in &mut fixed {
        let (handle, object) = charge(supervisor, account, frames)?;
        let Some(at) = charged.get_mut(charges) else { return Err(Failure::Account) };
        *at = handle;
        charges += 1;
        *slot = object;
    }
    let stack_pages = crate::process::SPAWN_STACK_PAGES;
    let (Some(&control), Some(&published)) = (fixed.get(stack_pages), fixed.get(stack_pages + 1))
    else {
        return Err(Failure::Account);
    };

    // The stack, one mapping per page. The frames need not be contiguous in
    // physical memory and the *pages* must be contiguous in the component's,
    // because a stack with a hole in it faults in the middle of a call rather
    // than at its guard — and a fault in the middle of a call is the one this
    // shape spent eleven months not having a name for. Lowest address first, so
    // the order here is the order `fixed` was charged in and the order a
    // teardown walks.
    for page in 0..stack_pages {
        let Some(&phys) = fixed.get(page) else { return Err(Failure::Account) };
        // SAFETY: as `user_space`, and `space` is not in `CR3` — it has never
        // been.
        unsafe {
            paging::map_user(
                frames,
                &mut space,
                crate::process::SPAWN_STACK + page as u64 * FRAME_SIZE,
                phys,
                UserPage::Data,
                features,
            )
        }
        .map_err(Failure::Space)?;
    }

    for (virt, phys, kind) in [
        (crate::process::SPAWN_CONTROL, control, UserPage::Data),
        // Writable, and that is the one thing about this mapping worth arguing.
        // RFC 0013's *read, never delivered* is about the **reader**: a tree is
        // read and never pushed. The publisher writes it, and the publisher is
        // this component — so the page it publishes into is its own to write and
        // everybody else's to read. What would be a control plane is the other
        // direction, a mapping somebody *else* writes that this component acts
        // on, and there is none: nothing in the frame writes a word here after
        // the schema block, and `what-must-be-stated.html` section 19 is why.
        (crate::process::SPAWN_TREE, published, UserPage::Data),
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
    // Where this component's heap landed, for the instance below. Zero when it
    // declared no `heap` need, which is every component but one today.
    let mut heap_at = 0u64;
    // And its board, on the same terms. Zero for every component that declares
    // no `board` need, which is what makes `write_board` and `write_routing`
    // refusing on zero a real check rather than a formality: a frame that tried
    // to fill in a board for a component that asked for none would be writing
    // into the direct map at offset zero.
    let mut board_at = 0u64;
    // And its data ring, on the same terms again. Zero for the five components
    // that serve nobody from a place.
    let mut data_at = 0u64;
    // The first capability the child is given, for the word it is entered with.
    let mut first = Handle::NULL;
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
        let granted =
            table.grant(found.kind, need.rights, found.object, found.extent).map_err(|_| {
                Failure::Capability(error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED))
            })?;
        // The first one, kept for the word the occupant is entered with. Taken
        // here rather than derived afterwards because this is the only place the
        // child's own handle exists — `offered` holds the *supervisor's* names
        // for these objects, and the two tables number their slots separately.
        if first == Handle::NULL {
            first = granted;
        }
        satisfied += 1;

        // The heap, and it is the one need the frame *maps* rather than merely
        // granting. A `#[global_allocator]` answers its first allocation before
        // the component has run a line of its own, so there is no moment at
        // which the component could ask for this to be mapped — and asking would
        // be a capability call, which a component that forbids `unsafe` cannot
        // make. The capability is granted above as well, so the component holds
        // authority over the memory it is using; what the frame adds is that the
        // memory is already there when the allocator first looks.
        if named(need, NEED_HEAP) {
            if found.extent > crate::process::HEAP_MAX {
                return Err(Failure::Capability(error::pack(
                    error::RESOURCE,
                    error::resource::QUOTA_EXHAUSTED,
                )));
            }
            let pages = found.extent / FRAME_SIZE;
            for page in 0..pages {
                // SAFETY: as the fixed parts above — `space` is this component's
                // and is not in `CR3`, and `found.object` names a contiguous run
                // of `found.extent` bytes the supervisor carved out of this
                // component's own account.
                unsafe {
                    paging::map_user(
                        frames,
                        &mut space,
                        crate::process::SPAWN_HEAP + page * FRAME_SIZE,
                        found.object + page * FRAME_SIZE,
                        UserPage::Data,
                        features,
                    )
                }
                .map_err(Failure::Space)?;
            }
            // Described through the frame's own direct map, before the first
            // instruction, for `publish_tree`'s reason: the component writes the
            // numbers and never the shape, and a region nobody described is one
            // the allocator refuses rather than allocates out of.
            let at = frames.virt(Frame::from_addr(found.object)) as u64;
            heap_at = at;
            // SAFETY: `at` is the direct-map address of a run the supervisor just
            // carved and nobody else holds, frame-aligned and longer than the
            // prologue — `charge` zeroed it and the bound above kept it inside
            // `HEAP_MAX`.
            unsafe { f_ring::heap::describe(at, u32::try_from(found.extent).unwrap_or(0)) };
        }

        // A need the caller sourced rather than the account. `offer` has
        // already granted the capability and checked the extent against the
        // manifest; what is left is putting it in the occupant's address space,
        // which the manifest cannot say and the caller must.
        //
        // **Why the frame maps it rather than the component.** The same reason
        // the heap is mapped here: a driver's first instruction may touch its
        // registers, and asking for them would be a capability call, which a
        // component that forbids `unsafe` cannot make against a window it does
        // not yet hold. The capability is granted as well, so the component has
        // authority over what it is using; the mapping is what makes the
        // authority usable before it has run a line.
        if let Some(item) =
            supplied.iter().find(|item| item.component == record.label() && named(need, item.name))
        {
            let (at, kind) = match item.map {
                Placement::Unmapped => (0, UserPage::Data),
                // Not zeroed, and the arm is above `Cached` so that a reader
                // looking for the exception finds it before the rule.
                Placement::Shown(at) => (at, UserPage::ReadOnly),
                Placement::Cached(at) => {
                    // **Zeroed, because a new occupant inherits nothing.** That
                    // is what the restart line says in as many words — *nothing
                    // carried over: new table, new memory, new control ring, new
                    // state tree* — and everything the account pays for gets it
                    // for free, because `fill` allocates zeroed and `tear_down`
                    // hands the frames back. Supplied memory does not: the caller
                    // holds it across the life of the boot precisely so that the
                    // *same* run can be given to the next occupant, and the same
                    // run is the previous occupant's leavings.
                    //
                    // `E1-P06` found this by being refused. A virtqueue is a
                    // descriptor table and two rings at fixed offsets, and a
                    // driver that lays one out over memory already holding a dead
                    // driver's rings reads indices that have already gone by. The
                    // device answered the first read of the second occupant with
                    // `DEVICE`, and every log line up to it said the refill had
                    // worked.
                    //
                    // Only `Cached`. Writing zeroes over a device window is not
                    // clearing memory, it is a sequence of register writes, and
                    // `Uncached` is exactly how this file knows the difference.
                    //
                    // SAFETY: `item.at` names a run of `item.bytes` the caller
                    // allocated and holds, the direct map covers it, and no core
                    // is inside this instance — it does not exist yet.
                    unsafe {
                        core::ptr::write_bytes(
                            frames.virt(Frame::from_addr(item.at)),
                            0,
                            item.bytes as usize,
                        );
                    }
                    (at, UserPage::Data)
                }
                // Uncached, and this is the one that is invisible under an
                // emulator and fatal on a machine: a write-back mapping lets
                // the processor merge, reorder and delay stores to registers
                // whose whole meaning is when they were written. `UserPage`'s
                // own doc makes the argument at length.
                Placement::Uncached(at) => (at, UserPage::Device),
            };
            if at != 0 {
                for page in 0..item.bytes / FRAME_SIZE {
                    // SAFETY: as the fixed parts above — `space` is this
                    // component's and is not in `CR3`. `item.at` names a run of
                    // `item.bytes` the caller found and holds: for `mmio` a
                    // device window this boot walked the bus for, for queue
                    // memory a run the caller allocated. Neither is account
                    // memory, which is why `reap` never hands it back.
                    unsafe {
                        paging::map_user(
                            frames,
                            &mut space,
                            at + page * FRAME_SIZE,
                            item.at + page * FRAME_SIZE,
                            kind,
                            features,
                        )
                    }
                    .map_err(Failure::Space)?;
                }
            }
            // `satisfied` was already incremented above, with the grant: a
            // supplied need is satisfied by the same capability every other
            // need is, and differs only in where the object came from. The
            // `continue` is for the arms below, none of which is this need.
            continue;
        }

        // The board, and it is the second need the frame maps rather than merely
        // granting — for the first one's reason, one step earlier. A component
        // that has to read this page to know what it holds has no instruction it
        // could have executed before the page was there: a supervisor reads it
        // to know which ring to adopt, and a driver reads it to know where its
        // device landed.
        //
        // It is a *need* and not a fixed part, which is the whole of why this
        // costs no arithmetic anywhere else: the account pays for it because the
        // manifest declares it, `admit` already sizes declared needs, and
        // `cargo xtask lint-manifests` already refuses a manifest that stops
        // adding up. A component that declares no `board` gets no board and no
        // mapping — which is three of the six in this tree.
        if named(need, NEED_BOARD) {
            // Exactly one page. A board is a layout both sides hold and
            // `f_supervisor::routing::BYTES` is one frame; a need that declared
            // two would be a second page nothing addresses, and a need that
            // declared none would be a mapping of nothing at all.
            if found.extent != FRAME_SIZE {
                return Err(Failure::Capability(error::pack(
                    error::ARGUMENT,
                    error::argument::BAD_ADDRESS,
                )));
            }
            // SAFETY: as the heap above — `space` is this component's and is not
            // in `CR3`, and `found.object` names a frame the supervisor carved
            // out of this component's own account.
            unsafe {
                paging::map_user(
                    frames,
                    &mut space,
                    crate::process::BOARD,
                    found.object,
                    UserPage::Data,
                    features,
                )
            }
            .map_err(Failure::Space)?;
            board_at = frames.virt(Frame::from_addr(found.object)) as u64;
        }

        // The data ring, and it is the third the frame maps rather than merely
        // granting. The argument is the board's, one step on: a component that
        // serves a client adopts the server end over a page whose header the
        // *grantor* wrote, and it cannot be the one to map that page, because
        // mapping is a capability call and the first entry may be waiting
        // before it could make one.
        //
        // Nothing is written into it here. The header is the grantor's and the
        // grantor is whoever holds the client's end — for a boot that is
        // `component::demonstrate`, which describes it in the same breath as it
        // fills in the routing page and before any core can read either.
        if named(need, NEED_DATA) {
            // Exactly one page, and refused at any other size for the board's
            // reason: `f_ring::Mapping` lays a ring out over a region both ends
            // agree the size of, and a need that declared two pages would be a
            // second page the header does not describe.
            if found.extent != FRAME_SIZE {
                return Err(Failure::Capability(error::pack(
                    error::ARGUMENT,
                    error::argument::BAD_ADDRESS,
                )));
            }
            // SAFETY: as the board above — `space` is this component's and is
            // not in `CR3`, and `found.object` names a frame the supervisor
            // carved out of this component's own account.
            unsafe {
                paging::map_user(
                    frames,
                    &mut space,
                    crate::process::BLK_DATA,
                    found.object,
                    UserPage::Data,
                    features,
                )
            }
            .map_err(Failure::Space)?;
            data_at = frames.virt(Frame::from_addr(found.object)) as u64;
        }
    }

    // The state tree, written before the component's first instruction. RFC
    // 0065: the schema block comes out of the manifest's declaration, so a
    // component that is spawned and never scheduled still has a readable tree
    // with zeros in it — which is the difference between *this component has
    // done nothing* and *this component cannot be read*, and a supervisor that
    // could not tell those apart is the one this task was written against.
    //
    // The words are the component's and the frame writes none of them. What it
    // writes is the description, and it writes it once: `generation` is zero
    // and stays zero, because a component's schema changes only when its
    // manifest does and a different manifest is a different place.
    let tree_at = frames.virt(Frame::from_addr(published));
    let tree_nodes = publish_tree(tree_at, record)?;
    // Read back through the ordinary reader rather than trusted from what was
    // just written, for the reason `state::Tree::publish` reads its own header
    // back: the value that matters is the one in the bytes, and a round trip is
    // what catches a schema whose Rust type and whose wire image disagree. It
    // is also the *first* reading of this tree and the only one taken before
    // the component could have touched a word.
    let reader =
        f_abi::state::Reader::at(tree_at as u64, FRAME_SIZE as u32).map_err(Failure::StateTree)?;
    if reader.nodes() != tree_nodes {
        return Err(Failure::StateTree(error::pack(
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        )));
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
        tree: tree_at as u64,
        tree_physical: published,
        heap: heap_at,
        board: board_at,
        data: data_at,
        tree_nodes,
        tree_snapshot: reader.snapshot(),
        first,
    });
    place.epoch += 1;
    Ok((epoch, satisfied, charges))
}

/// Put a place's occupant's published tree under the frame's root, and read it
/// back from there.
///
/// **The reading is the point.** Writing an address into a mount node and
/// calling the tree reachable would be a claim; what makes it a check is that
/// this function then does what any other reader would do — takes the address
/// out of the node it just wrote, binds `f_abi::state::Reader` to it, and
/// requires the header, the schema and every offset in them to validate. A root
/// that named a region nobody had opened would be found by the first reader to
/// follow it, and that reader is this one.
///
/// # Errors
///
/// [`Failure::StateTree`] for a mount this build has no node for, or a child
/// the frame cannot read back through its own root.
fn mount(
    tree: &crate::state::Tree,
    place: &Place,
    report: &mut Report,
) -> Result<(u64, [u8; 16], usize), Failure> {
    let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
    let occupant = place.occupant.as_ref().ok_or(Failure::WrongPlace)?;
    if !tree.mount(place.slot, occupant.tree_physical) {
        return Err(Failure::StateTree(malformed));
    }
    // Back out through the root, which is what a reader that only has the frame
    // does. The kernel address is what this build can bind to — the mount word
    // is physical and the frame reaches physical memory through its direct map
    // — and the two naming one frame is `FrameAllocator::virt`'s whole
    // contract. *Reversal:* a reader outside the frame, which needs the mount
    // to be followed by a grant rather than by a direct-map lookup, and that is
    // a capability handed over and not a wider mount node.
    let Some(id) = crate::state::node::mount(place.slot) else {
        return Err(Failure::StateTree(malformed));
    };
    let named = tree.value(id).ok_or(Failure::StateTree(malformed))?;
    if named != occupant.tree_physical {
        return Err(Failure::StateTree(malformed));
    }
    let reader =
        f_abi::state::Reader::at(occupant.tree, FRAME_SIZE as u32).map_err(Failure::StateTree)?;
    if reader.nodes() != occupant.tree_nodes || reader.snapshot() != occupant.tree_snapshot {
        return Err(Failure::StateTree(malformed));
    }

    // The child's root, taken out of the schema the reader validated. It is
    // what makes this line evidence rather than an assertion: four components
    // publishing five zeroed nodes each hash identically — the snapshot is over
    // bytes and never over interpretation, which is RFC 0013's decision and not
    // a defect — so a mount line carrying only a count and a hash would say the
    // same words about every place. The root's *name* is the one thing in a
    // child's region that differs per manifest before its first instruction,
    // and reading it back proves the frame descended into the schema it wrote
    // rather than merely finding a header where it left one.
    let root = reader.schema().first().ok_or(Failure::StateTree(malformed))?;
    let label = root.name;
    let width = root.label().len();

    report.mounted += 1;
    report.nodes += occupant.tree_nodes;
    tree.set(crate::state::node::COMPONENTS_MOUNTED, tree.mounted());
    tree.add(crate::state::node::COMPONENTS_PUBLISHED, 1);
    tree.add(crate::state::node::COMPONENTS_NODES, u64::from(occupant.tree_nodes));
    Ok((reader.snapshot(), label, width))
}

/// Take a place's mount out of the frame's root.
///
/// Called from [`tear_down`] and nowhere else, because a mount naming a frame
/// that has gone back to the account is the one shape of dangling pointer a
/// state tree can have: the next instance is about to be given those bytes, and
/// a reader following the old address would be reading a live component's tree
/// under a dead one's name.
fn unmount(tree: &crate::state::Tree, place: &Place) {
    let _ = tree.mount(place.slot, 0);
    tree.set(crate::state::node::COMPONENTS_MOUNTED, tree.mounted());
}

/// Write a component's state-tree header and schema block into a frame it owns.
///
/// The whole of RFC 0013's *the data block is generated from the same
/// declaration the schema is*, which that RFC names as a build-time obligation
/// and as the only defence against the two drifting. There is one declaration —
/// the manifest's, inside the content hash a spawn names — and this is the one
/// place it becomes bytes. `f_abi::manifest::Node::entry` is the other half of
/// the arithmetic, and it is the only thing that decides where a node's word
/// lives.
///
/// Answers how many nodes it wrote.
///
/// # Errors
///
/// [`Failure::StateTree`], carrying the packed refusal, for a declaration this
/// build cannot turn into a readable tree — which after [`admit`] and
/// `Record::read` means a bound this function got wrong rather than a manifest
/// that is wrong.
pub(crate) fn publish_tree(base: *mut u8, record: &Record) -> Result<u32, Failure> {
    let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);
    let declared = record.state_nodes();
    let nodes = u32::try_from(declared.len()).map_err(|_| Failure::StateTree(malformed))?;

    // The two blocks, at the offsets the format fixes: the schema immediately
    // after the header on the thirty-two byte boundary an entry array needs,
    // and the data block after it on an eight-byte one. Both are computed here
    // and checked by `Reader::at` against the frame's own length, so a bound
    // this function got wrong is a refusal rather than a write past the page.
    let schema_at = core::mem::size_of::<f_abi::state::TreeHeader>() as u32;
    let data_at = schema_at + nodes * 32;
    if u64::from(data_at) + u64::from(nodes) * u64::from(f_abi::state::WORD) > FRAME_SIZE {
        return Err(Failure::StateTree(malformed));
    }

    let header = f_abi::state::TreeHeader {
        magic: f_abi::state::TREE_MAGIC,
        version: f_abi::state::TREE_VERSION,
        nodes,
        schema_offset: schema_at,
        data_offset: data_at,
        // Zero, and it stays zero for this instance's life: a generation counts
        // *schema* republications, a component's schema is its manifest's, and
        // a spawn naming a different manifest is refused as a different place.
        generation: 0,
        _reserved: [0; 3],
    };
    // SAFETY: `base` is the direct-map address of a frame this function retyped
    // out of the account a few lines ago, zeroed by `charge` and handed to
    // nobody else. It is frame-aligned, which is stronger than the sixty-four
    // bytes a `TreeHeader` needs.
    unsafe { base.cast::<f_abi::state::TreeHeader>().write(header) };

    // SAFETY: `schema_at` is sixty-four, which is inside the frame and is
    // aligned for a `SchemaEntry` — thirty-two divides it.
    let block = unsafe { base.add(schema_at as usize) }.cast::<f_abi::state::SchemaEntry>();
    for (index, node) in declared.iter().enumerate() {
        // SAFETY: the schema block is `nodes` thirty-two byte entries at
        // `block`; the bound above places the whole of it, and the data block
        // after it, below `FRAME_SIZE`. `index` is a position in `declared`, so
        // it is below `nodes`.
        let slot = unsafe { block.add(index) };
        // SAFETY: as above; `slot` is inside the frame and aligned for the type.
        unsafe { slot.write(node.entry(index as u32)) };
    }
    Ok(nodes)
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
    supplied: &[Supplied],
) -> Result<Supply, Failure> {
    let mut out = Supply::EMPTY;
    for need in record.needs() {
        if need.route == route::POWERBOX {
            continue;
        }
        let Some(kind) = need.cap_type() else { return Err(Failure::Manifest(Refusal::Value)) };
        let held = need.rights | OFFERED;
        // Sourced by the caller before anything is charged, because a supplied
        // object is one the account must never be debited for and never hand
        // back. The extent is checked against the manifest here rather than at
        // the call site: the manifest is the contract, and a caller that
        // supplied the wrong size would otherwise produce a driver whose device
        // window stops halfway.
        //
        // **A least and not an equality**, which is what [`least_extent`] is
        // called and what `check_needs` has always compared. This read `!=`
        // until RFC 0094, and the comment above is the argument for `<`: a
        // window that stops halfway is a supply that is *too small*, and a
        // supply larger than the manifest asks for stops nothing. What made the
        // difference matter is a need whose size the component cannot know in
        // advance — the boot module is as large as the topology it contains, so
        // a manifest declaring its exact size would be a manifest rewritten by
        // every change to any component in the generation.
        //
        // The component is told the real extent on its board. A manifest's
        // number is what the account is sized against and what a spawn is
        // refused under; it was never the number the component reads.
        let given =
            supplied.iter().find(|item| item.component == record.label() && named(need, item.name));
        if let Some(item) = given {
            if item.bytes < least_extent(need) {
                return Err(Failure::Manifest(Refusal::Value));
            }
            let handle = grant_into(supervisor, frames, kind, held, item.at, item.bytes)?;
            let Some(slot) = out.handles.get_mut(out.count) else { return Err(Failure::Account) };
            *slot = handle;
            out.count += 1;
            continue;
        }
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
        spawn(
            frames,
            kernel,
            features,
            place,
            account,
            supervisor,
            reservations,
            Supply::EMPTY,
            &[],
        )
    };
    taken += refused(outcome, error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP))?;

    // 2. A handle supplied for a need the manifest does not declare. Nothing
    //    has to be in the slot for this: what is refused is the *count*, and a
    //    supply that claimed more than the record describes is refused before
    //    any of it is looked at.
    let mut over = Supply::EMPTY;
    over.count = declared + 1;
    // SAFETY: as above.
    let outcome = unsafe {
        spawn(frames, kernel, features, place, account, supervisor, reservations, over, &[])
    };
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
    let outcome = unsafe {
        spawn(frames, kernel, features, place, account, supervisor, reservations, wrong, &[])
    };
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
            spawn(frames, kernel, features, place, account, supervisor, reservations, weak, &[])
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
            spawn(frames, kernel, features, place, account, supervisor, reservations, small, &[])
        };
        taken += refused(outcome, error::pack(error::ADMISSION, error::admission::MEMORY))?;
        supervisor.relinquish(held).map_err(Failure::Capability)?;
    }

    // 6. A manifest that declares no state tree. RFC 0065, and it is the
    //    refusal that keeps *every component publishes one* true rather than
    //    aspirational — so it is provoked on every boot, for the reason every
    //    other probe here is: a refusal nobody has watched happen is
    //    indistinguishable from one that cannot.
    //
    //    Driven through `admit` and not through `spawn`, and the difference is
    //    worth stating rather than hiding. `spawn` reads its record out of the
    //    place's module, which is a component file on the boot medium and not
    //    something this function may bend; `admit` takes the record as an
    //    argument and is the function the refusal actually lives in — the same
    //    one a real spawn reaches on its way past. So the probe is the shipped
    //    predicate driven against a record whose *only* difference from the
    //    real one is the declaration, which is a sharper test than a second
    //    component file carried for the purpose would be.
    let mut mute = *record;
    mute.state_nodes = 0;
    let outcome = admit(&mute, supervisor, account, 1, reservations, place.reservation);
    let want = error::pack(error::ADMISSION, error::admission::NO_STATE_TREE);
    match outcome {
        Err(Failure::Admission(code)) if code == want => taken += 1,
        Err(why) => return Err(why),
        Ok(()) => return Err(Failure::Admission(want)),
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
    // A component that publishes nothing is refused rather than tolerated, and
    // it is refused **first** — before the reservation is even tested, because
    // this is the one check in the function that reads nothing but the record.
    // RFC 0013 puts a tree in every component and this is the line that makes
    // *every* mean every: the declaration is in the manifest, so the refusal
    // costs nothing and happens before a frame is spent, and a component that
    // got past here can be read whether or not anybody remembers to look.
    //
    // `ADMISSION` and not `ARGUMENT`, for the reason `admission::NO_STATE_TREE`
    // gives: the record is well formed and the supply is sound. What is missing
    // is something a supervisor requires of anything it will host, which is
    // where every other admission refusal is decided.
    if record.state_nodes().is_empty() {
        return Err(Failure::Admission(error::pack(
            error::ADMISSION,
            error::admission::NO_STATE_TREE,
        )));
    }
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

/// The frame's second control-ring server: the opcodes a **supervisor**
/// submits, as opposed to the ones a driver submits.
///
/// # Why this is not two more arms on `supervisor::Supervising`
///
/// RFC 0073, and the short version is that the two servers do not share a
/// mechanism. `Supervising` is device-shaped by construction: it borrows a
/// remapping `Unit`, a `vtd::Domain`, and — the detail that settles it — **the
/// client's** capability table, deliberately, so that a driver asking for a
/// translation is asking about somebody else's capability and gets somebody
/// else's rights.
///
/// A supervisor's opcodes need the opposite table. Every handle in an entry here
/// is resolved against the **submitter's** own table, because a stop is the
/// holder of an endpoint exercising a right it holds rather than asking about
/// anybody else's. Widening one struct to borrow both tables, for two opcodes
/// that never touch a device, would put the capability path and the
/// device-translation path behind one `match` — the merge RFC 0071 refused one
/// file over, with more force here.
///
/// # What it deliberately does not do
///
/// **The teardown is not in here.** [`Serving::stop`] makes the promise and
/// answers which deadline was kept; ending the occupant is the frame's
/// follow-through and stays where it was. That split is not tidiness: it is what
/// keeps the boot log byte-identical across this change, so `cargo xtask trace`
/// hashing an unmoved log is the evidence that the *path* moved and the
/// behaviour did not.
struct Serving<'a> {
    /// The place whose occupant these opcodes act on.
    place: &'a mut Place,
    /// Where the handles an entry *names* are resolved.
    ///
    /// # Why this is not the field below, since for two epochs it was
    ///
    /// Because they were one field doing two jobs, and the day a component
    /// started submitting, the two jobs stopped having the same answer.
    ///
    /// A capability named in an entry has to be resolved in **the submitter's**
    /// table — that is the whole of what a capability system is for, and a
    /// server that resolved a caller's index in its own table would be the
    /// confused deputy with extra steps. When the frame built these entries
    /// itself, the submitter was the frame, so one field was right by accident.
    /// Now `user/supervisor` submits, and the account it names is a handle in
    /// its own table.
    ///
    /// Shared and not exclusive, which is the shape of the claim: an entry's
    /// handles are *read*. Nothing a submitter names is minted, cleared or
    /// grown here.
    ///
    /// **It is a copy, taken before the submitter ran.** The core clears an
    /// occupant's table when the occupant ends, so the live table is empty by
    /// the time the frame drains a ring the occupant left behind. The copy is
    /// what the supervisor held at the moment it submitted, which is the
    /// question this field is asked — *may the submitter spend this?* — and
    /// answering it from a table emptied afterwards would refuse every entry
    /// with `AUTHORITY/REVOKED`. That is not a theory: it is the refusal the
    /// first boot of this path produced.
    submitter: &'a Table,
    /// Where handles are **minted**, and where what a spawn charged is recorded
    /// for the teardown that gives it back.
    ///
    /// The frame's, and RFC 0008 says it should be the supervisor's. That half
    /// has not moved and this is where the reason sits rather than in a
    /// paragraph somewhere else: `offer` mints one name per need and `spawn`
    /// records them in [`Instance::supplied`], and [`tear_down`] walks that list
    /// in *this* table. Moving the supply to the submitter's table means the
    /// teardown of a place has to walk whichever table its occupant was spawned
    /// from — which is a per-place field, not a rename — and the cross-table
    /// parent link that would make it a derive rather than a grant is RFC 0029's
    /// and deliberately did not land.
    ///
    /// *Reversal:* that link. Then these two fields become one again, from the
    /// other direction.
    supervisor: &'a mut Table,
    /// Where the frames a spawn charges come from.
    frames: &'a mut FrameAllocator,
    /// The account a spawn is charged to, and a teardown refunds into.
    account: &'a Account,
    /// The kernel's address space, which a new instance's tables are built
    /// against.
    kernel: &'a paging::AddressSpace,
    /// What the processor offers the tables built here.
    features: Features,
    /// What a spawn's admission is tested against.
    reservations: &'a Reservations,
    /// What the last spawn produced: epoch, frames charged, needs supplied.
    ///
    /// Here rather than in the completion because a `Cqe` carries one `u64` and
    /// this is three numbers the boot's log lines want. `Supervising` keeps
    /// `answered_at` for the same reason and this is that idiom, not a new one.
    spawned: (u32, usize, usize),
    /// How many entries this server has answered. Unit: entries.
    answered: u64,
}

impl Serving<'_> {
    /// One control-ring operation.
    ///
    /// R04 at the bottom, the same as the driver server's: an opcode this build
    /// does not implement is refused and never ignored. Today that is every
    /// opcode but one, and the refusal is the arm with a test written against it
    /// first — a success path landing beside an untested refusal is a refusal
    /// nobody finds out about until a component depends on it.
    fn execute(&mut self, entry: &f_abi::Sqe) -> Cqe {
        self.answered = self.answered.saturating_add(1);
        match entry.opcode {
            f_abi::control::op::SPAWN => self.spawn(entry),
            f_abi::control::op::STOP => self.stop(entry),
            other => f_ring::refusal(
                entry.user_data,
                error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
                u64::from(other),
                0,
            ),
        }
    }

    /// `op::SPAWN`: create a component in this place.
    ///
    /// `abi/src/control.rs` fixes the shape: `cap` names the `Untyped` that
    /// pays and `ext` names the manifest by content hash.
    ///
    /// # What this arm adds, and what it deliberately does not
    ///
    /// **No policy.** Every refusal a spawn owes is already implemented and
    /// already exercised — `admit` and `check_needs` are what the boot's six
    /// deliberate refusals go through — so this is a *route* to them and not a
    /// second copy of them. The two checks here are the ones that are about the
    /// entry rather than about the manifest: a `cap` that is not an `Untyped`
    /// the submitter may spend, and an `ext` naming a manifest this place does
    /// not hold. `spawn` checks the second again from the module itself, which
    /// is not redundancy for the reason the boot's two `admit` calls are not:
    /// this is checking what it was *asked*, and that one checks what is *there*.
    ///
    /// **The supply is still built by the frame.** RFC 0008 wants the
    /// supervisor to charge its own account and hand the handles over in the
    /// arena; there is no supervisor component yet to do it, and no arena here
    /// because the boot builds these entries directly. So [`offer`] runs on this
    /// side for now. That is the frame holding ground it does not want, exactly
    /// as `policy` is, and it moves in the same increment `policy` does.
    fn spawn(&mut self, entry: &f_abi::Sqe) -> Cqe {
        // The account that pays. `GRANT` because spending an `Untyped` on
        // somebody else's behalf is handing authority on, which is the right
        // that names doing so.
        if let Err(packed) =
            self.submitter.invoke(Handle::from_bits(entry.cap), CapType::Untyped, rights::GRANT)
        {
            return f_ring::refusal(entry.user_data, packed, u64::from(entry.cap), 0);
        }
        let record = match Record::read(self.place.module) {
            Ok(record) => record,
            // Unreadable here rather than in `spawn`, so that a place holding a
            // module that no longer parses is refused with the manifest's own
            // refusal rather than with a spawn failure the submitter cannot act
            // on.
            Err(_) => {
                return f_ring::refusal(
                    entry.user_data,
                    error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
                    entry.ext[0],
                    0,
                );
            }
        };
        // A different manifest is a different place (RFC 0041). Refused against
        // what the *place* holds rather than against what the module hashes to,
        // because those are the same today and stop being so the moment a
        // generation swap puts a newer image behind an existing place — which is
        // `E2-B06`, and is the case this refusal has to still be right for.
        if entry.ext[0] != self.place.manifest.bits() {
            return f_ring::refusal(
                entry.user_data,
                error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS),
                entry.ext[0],
                0,
            );
        }
        // Nothing supplied. A spawn arriving over the control ring is the
        // supervisor's, and a supervisor at ring 3 has no device window to hand
        // down — what it can offer is what its own account holds. A supplied
        // need reaches a place only from the boot that found the device.
        let offered =
            match offer(self.supervisor, self.account, self.place, record, self.frames, &[]) {
                Ok(offered) => offered,
                Err(failure) => {
                    return f_ring::refusal(entry.user_data, Self::packed(failure), 0, 0);
                }
            };
        // SAFETY: `demonstrate`'s guarantee, which this server is constructed
        // inside: the direct map is live and covers every module, and the
        // address space this builds tables against is the kernel's own.
        let spawned = unsafe {
            spawn(
                self.frames,
                self.kernel,
                self.features,
                self.place,
                self.account,
                self.supervisor,
                self.reservations,
                offered,
                &[],
            )
        };
        match spawned {
            Ok(spawned) => {
                self.spawned = spawned;
                // The endpoint to the place, which `abi/src/control.rs` says a
                // spawn completes with. It already exists in the submitter's
                // table — a place is made before its first occupant — so this
                // hands back the handle rather than minting a second one.
                Cqe {
                    user_data: entry.user_data,
                    result: 0,
                    flags: 0,
                    timestamp: 0,
                    ext: u64::from(self.place.endpoint.bits()),
                }
            }
            Err(failure) => f_ring::refusal(entry.user_data, Self::packed(failure), 0, 0),
        }
    }

    /// A [`Failure`] as the packed refusal a submitter gets back.
    ///
    /// The variants that already carry one hand it over unchanged — an
    /// admission refusal is in the `ADMISSION` domain and a need refusal names
    /// which of the five ways a supply can be wrong it was, and flattening
    /// either into a generic error would throw away the only part a submitter
    /// can act on. The rest are the frame failing at its own work rather than
    /// refusing the caller's, so they are `RESOURCE/EXHAUSTED`: true of the
    /// memory cases, and honest about the others in that the submitter cannot
    /// fix them and should not be told it can.
    const fn packed(failure: Failure) -> i32 {
        match failure {
            Failure::Admission(packed)
            | Failure::Need(packed)
            | Failure::Capability(packed)
            | Failure::Ring(packed)
            | Failure::Notice(packed)
            | Failure::StateTree(packed) => packed,
            Failure::Manifest(_) => error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
            Failure::WrongPlace => error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS),
            _ => error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
        }
    }

    /// `op::STOP`: promise to end the occupant of an endpoint by a deadline.
    ///
    /// `abi/src/control.rs` fixes the semantics and this implements them.
    /// A stop with [`f_abi::NO_DEADLINE`] is a promise nothing can refuse and
    /// the frame refuses to make it. A stop whose deadline has already passed is
    /// a kill, spelled the same way as a polite stop so that the simulator's
    /// *kill this driver at a seeded moment* is one opcode rather than two paths
    /// through the frame.
    ///
    /// The completion's `ext` is the deadline that was **kept**, which is not
    /// always the one submitted: a second stop may only move a promise earlier,
    /// so a submitter that asked for a later one has to be told what it actually
    /// holds. A bare success there would be a number the submitter misreads.
    fn stop(&mut self, entry: &f_abi::Sqe) -> Cqe {
        // Resolved against the submitter's own table, and `REVOKE` because
        // ending somebody is the same grade of authority as taking a capability
        // back. `invoke` is the one call that checks kind and rights together;
        // doing it in two steps is how a check that passes on the wrong type
        // gets written.
        if let Err(packed) =
            self.submitter.invoke(Handle::from_bits(entry.cap), CapType::Endpoint, rights::REVOKE)
        {
            return f_ring::refusal(entry.user_data, packed, u64::from(entry.cap), 0);
        }
        if entry.deadline == f_abi::NO_DEADLINE {
            return f_ring::refusal(
                entry.user_data,
                error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
                u64::from(entry.opcode),
                0,
            );
        }
        let Some(occupant) = self.place.occupant.as_mut() else {
            return f_ring::refusal(
                entry.user_data,
                error::pack(error::PEER, error::peer::EMPTY),
                u64::from(entry.cap),
                0,
            );
        };
        occupant.table.stop_by(entry.deadline);
        let kept = occupant.table.stop_deadline().unwrap_or_default();
        Cqe { user_data: entry.user_data, result: 0, flags: 0, timestamp: 0, ext: kept }
    }
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
    tree: &crate::state::Tree,
) -> Result<(u32, u32, u32, u32), Failure> {
    // The heap, read before the region goes back to the account. This is the
    // evidence that a component allocated: the numbers are in the region's own
    // prologue, written by the allocator, and read here by the frame — the same
    // arrangement as a state tree, and for the same reason. A component's word
    // for whether it allocated would be worth nothing.
    if let Some(occupant) = place.occupant.as_ref()
        && occupant.heap != 0
    {
        // SAFETY: `occupant.heap` is the direct-map address of a region this
        // frame mapped and described at spawn, and the occupant has ended — the
        // teardown below is what gives it back, and it has not run yet.
        let heap = unsafe { f_ring::heap::Heap::over(occupant.heap) };
        if heap.valid() {
            report.heap_peak = report.heap_peak.max(heap.peak());
            report.heap_starved |= heap.starved();
            report.heap_bytes = report.heap_bytes.max(heap.bytes());
        }
    }

    let mut withdrawn = (0, 0, 0, 0);
    // 0. Take the mount out of the frame's root, and take it out *first*. The
    //    frame this place's occupant published into is about to go back to the
    //    account and be handed to the next instance, so a root still naming it
    //    would be a reader following one place's mount into another
    //    component's live tree. Outside the branch below for the same reason
    //    step 5 is: an empty place has nothing mounted, and clearing a word
    //    that is already zero costs one store.
    unmount(tree, place);
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
///
/// # The one place that half must *not* be performed
///
/// `user/supervisor` reads its own ring. Draining on its behalf would take the
/// notices it is about to be told by (RFC 0076) and, worse, the frame's own
/// answers to the spawns it submitted — which are not notices at all, and which
/// this function refuses on sight because for every other component an entry on
/// a control ring is a notice or a mistake.
///
/// That refusal is correct and stays. What was missing is that a component which
/// drains for itself has a control ring the frame may post to and may not read,
/// so [`publish_only`] is that call and this one keeps its meaning.
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

/// Post what an occupant is owed onto its control ring, and **leave it there**.
///
/// For the one component that drains for itself. [`publish`]'s doc argues why
/// that needs its own call rather than a flag on the drain: the notices this
/// posts are what the supervisor is *told* by on its next run (RFC 0076), and
/// the frame's own answers to its spawns are already on the same ring and are
/// not notices at all.
///
/// The frame's own ledger is still pumped both ways, because that half is the
/// frame talking to itself and is where `Report::collected` comes from.
///
/// # Errors
///
/// As [`publish`].
fn publish_only(
    place: &mut Place,
    supervisor: &mut Table,
    ledger: &Mapping,
    report: &mut Report,
) -> Result<(), Failure> {
    if let Some(occupant) = place.occupant.as_mut() {
        let posted = post_only(&mut occupant.table, &occupant.ring, Handle::NULL)?;
        report.notices += posted;
        report.handed += posted;
    }
    let pumped = pump(supervisor, ledger, Handle::NULL)?;
    report.notices += pumped.0;
    report.collected += pumped.1;
    report.rounds += pumped.2;
    report.kinds |= pumped.3;
    Ok(())
}

/// [`pump`]'s posting half, without the drain.
///
/// One round rather than rounds, and that is a real bound rather than a
/// simplification: with nobody draining on this side, a second round would find
/// exactly the room the first one left. A table owing more than the ring holds
/// keeps owing it, which is precisely what RFC 0008 says pending state is for —
/// the ring's depth bounds what is *visible*, never what is true, and the rest
/// is posted the next time this component is consulted.
///
/// # Errors
///
/// As [`pump`], for a ring that stopped validating or a kind this build does not
/// define.
fn post_only(table: &mut Table, ring: &Mapping, control: Handle) -> Result<u32, Failure> {
    let poster = Poster::new(ring.completions()).ok_or_else(|| {
        Failure::Notice(error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER))
    })?;
    let mut posted = 0;
    while poster.free().map_err(ring_error)? > 0 {
        let Some(entry) = next_notice(table, control) else { break };
        if !f_abi::control::is_notice(&entry) || !notice::known(entry.result) {
            return Err(Failure::Notice(entry.result));
        }
        poster.post(entry).map_err(ring_error)?;
        posted += 1;
    }
    Ok(posted)
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
