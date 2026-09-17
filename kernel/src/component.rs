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
    /// declared no `board` need — which is every component but the supervisor.
    ///
    /// Written by the frame before the first instruction and read by it after
    /// the last one, which is the two halves `f_supervisor::routing` describes.
    /// A kernel address and not the component's, because the frame reaches it
    /// through the direct map and never through the component's page tables.
    /// Unit: bytes.
    board: u64,
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
    /// How many of the places' occupants were handed a core. Unit: components.
    ///
    /// One, and the one is the supervisor. A count rather than a flag because
    /// the number this should be is a decision — RFC 0073's bootstrap argument
    /// says the frame starts exactly one — and a flag would record that it
    /// happened without recording that it happened *once*.
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
pub unsafe fn demonstrate(
    frames: &mut FrameAllocator,
    kernel: &paging::AddressSpace,
    features: Features,
    boot: &BootInfo,
    now: u64,
    tree: &crate::state::Tree,
    worker: Option<(usize, u64)>,
) -> Result<Report, Failure> {
    // SAFETY: the caller's guarantee that the direct map is live and covers
    // every module.
    let (modules, count) = unsafe { modules(boot) };
    let module = *modules.first().filter(|_| count > 0).ok_or(Failure::NoComponent)?;
    let record = Record::read(module).map_err(Failure::Manifest)?;

    let before = frames.free_count();
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
        scheduled_line(cpu, consulted.announced, consulted.death);
        supervised_line(&consulted, u32::from(open.place.occupant.is_some()));

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
        let offered = offer(&mut supervisor, &account, &place, record, frames)?;
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

    if frames.free_count() != before {
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
        let offered = offer(supervisor, &account, &place, record, frames)?;
        // SAFETY: the caller's guarantee, passed down. The place was built a few
        // lines ago and is empty.
        let spawned = unsafe {
            spawn(frames, kernel, features, &mut place, &account, supervisor, reservations, offered)
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

/// The first occupant of a place ever handed a core.
///
/// A line of its own rather than a field on `supervisor ok`, because this is the
/// sentence `kernel/src/runtime.rs` has carried since RFC 0033 becoming false,
/// and a reader comparing two boots across the change should meet it where it
/// happened rather than in a summary at the end.
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
fn scheduled_line(cpu: usize, announced: bool, death: crate::process::Death) {
    crate::kprintln!(
        "  scheduled     place supervisor on core {cpu} — the first occupant of a place given \
         one; it {} itself from ring 3",
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
unsafe fn write_board(
    frames: &FrameAllocator,
    occupant: &mut Instance,
    target: &Place,
    account: &Account,
    supervisor: &Table,
    watches: Handle,
    now: u64,
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

unsafe fn consult(
    asking: &mut Consulting,
    occupant: &mut Instance,
    target: &mut Place,
    account: &Account,
    watches: Handle,
    now: u64,
) -> Result<Consulted, Failure> {
    let Consulting { frames, kernel, features, supervisor, reservations, on: (cpu, tsc_khz) } =
        asking;
    let (features, cpu, tsc_khz) = (*features, *cpu, *tsc_khz);
    // Everything the component needs to know that is not a constant: its ring,
    // the account it may spend, the place it may act on, and the tally the frame
    // is holding for it. Written *before* the core is told to run, which is the
    // whole of why it may be believed.
    // SAFETY: `occupant.board` is a frame this frame allocated for this instance
    // and mapped into its address space and nobody else's; the direct map covers
    // it and the core that will read it is idle.
    unsafe { write_board(frames, occupant, target, account, supervisor, watches, now) }?;

    // What the supervisor holds at the moment it is started, kept for the server
    // below to resolve its entries against. Taken here — after the grants above
    // and before the core is told to run — because that is precisely the state
    // the question *may the submitter spend this?* is about. The table the core
    // hands back is not it: an occupant's table is cleared when it ends.
    let submitted_from = occupant.table;

    // Selector zero: the life every component has. `first` is the occupant's own
    // handle for its account, so its first act can be a capability call rather
    // than a guess about which slot it holds.
    let argument = f_abi::door::Entry::new(0, occupant.first).bits();
    let root = occupant.space.root();
    // SAFETY: `root` is the address space this frame built for this occupant and
    // nothing has torn it down; its text is mapped executable at `process::TEXT`
    // and its stack is writable below `process::SPAWN_STACK_TOP`, both by
    // `spawn`. `cpu` is the caller's, and the table handed over is taken back
    // below before this instance is touched again.
    let previous = unsafe {
        crate::process::schedule_occupant(
            cpu,
            root,
            features,
            occupant.table,
            argument,
            OCCUPANT_HZ,
            OCCUPANT_TICKS,
        )
    };
    // SAFETY: the boot processor, with the kernel's space in `CR3` and the job
    // published into `cpu`'s own shard above. `run_on` makes the `Release` store
    // that publishes it.
    let ran = unsafe { crate::smp::run_on(cpu, kernel.root(), tsc_khz, OCCUPANT_MICROS) };
    ran.map_err(|_| Failure::NoAnswer)?;

    // What the core recorded, read before the shards are reused.
    // SAFETY: the core reported finished, which is what `Ok` above means.
    let (announced, death, _ticks) = unsafe { crate::process::occupant_outcome(cpu) };
    // The other half of the swap, taken back *before* anything else looks at
    // this instance. What comes back is not what went in: a component may have
    // derived, and a teardown has to revoke what it ended holding.
    // SAFETY: as above — the core is finished, so nothing over there is holding
    // either table.
    occupant.table = unsafe { crate::process::reclaim_occupant_table(cpu, previous) };
    // SAFETY: as above, and the page is still mapped — an address space is torn
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
    // And its board, on the same terms. Zero for every component that is not the
    // supervisor, which is what makes `write_board` refusing on zero a real
    // check rather than a formality: a frame that tried to fill in a board for a
    // component with no `board` need would be writing into the direct map at
    // offset zero.
    let mut board_at = 0u64;
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

        // The board, and it is the second need the frame maps rather than merely
        // granting — for the first one's reason, one step earlier. A supervisor
        // has to read this page to know which ring to adopt, so there is no
        // instruction it could have executed before the page was there.
        //
        // It is a *need* and not a fixed part, which is the whole of why this
        // costs no arithmetic anywhere else: the account pays for it because the
        // manifest declares it, `admit` already sizes declared needs, and
        // `cargo xtask lint-manifests` already refuses a manifest that stops
        // adding up. A component that declares no `board` gets no board and no
        // mapping — which is every component but one.
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
        let offered = match offer(self.supervisor, self.account, self.place, record, self.frames) {
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
