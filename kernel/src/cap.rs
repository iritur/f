// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The capability table: what a process may name, and the tree that lets it be
//! taken back.
//!
//! # What this is
//!
//! One table per process. [`TABLE_SLOTS`] typed slots to begin with, each
//! holding an object, a rights bitmap and the handle of the capability it was
//! derived from. Three operations on it — derive, revoke, and the lookup every
//! use of a capability begins with — plus [`Table::grant`], which is the frame
//! putting something in and is not reachable from ring 3 at all.
//!
//! The wire half is [`f_abi::cap`]: the handle packing, the six types and the
//! rights bits. Nothing about *storage* is over there, and nothing about the
//! *format* is over here.
//!
//! # Why the table is an object a component pays for
//!
//! The count above is a floor and not a ceiling. When a derive finds nowhere to
//! put its child, the table buys itself another page of slots out of a
//! [`CapType::Untyped`] capability the component already holds, and a component
//! with nothing left to spend is refused
//! [`error::resource::QUOTA_EXHAUSTED`] rather than served out of anything the
//! frame keeps in reserve. That is RFC 0008's decision — everything a component
//! is made of is retyped from a supplied `Untyped`, the capability table
//! included — and E1-B13 is where the table half of it lands. RFC 0029 is the
//! argument for the shape it took.
//!
//! Two consequences are worth stating rather than deducing, because both are
//! the reason to prefer this over a larger array.
//!
//! **The refusal is local and it is deterministic.** A component that keeps
//! deriving runs out of *its own* account, at a point that is a function of what
//! it was handed and what it has spent, and nobody else's run changes when it
//! happens. `docs/design/deadline-all-the-way-down.html` section 03 names that
//! as a precondition for a simulation that reproduces, and a shared reserve —
//! however large — is the thing that would take it away.
//!
//! **The memory the table lives in is memory the component has spent.** Growth
//! advances the untyped region's watermark, exactly as a retype does, so the
//! frame that becomes table storage is a frame no later retype can hand back
//! out. A component therefore cannot obtain a [`CapType::Frame`] naming its own
//! capability table, and that is a structural consequence of the watermark
//! rather than a check somebody wrote.
//!
//! What did *not* have to change is the wire format. [`Handle`] carries a
//! sixteen-bit index, which addresses 65 536 slots; [`MAX_SLOTS`] is the
//! ceiling this build sets and is far below it. Growth is invisible to
//! `abi/src/cap.rs`, and a task that had to widen the index would have been an
//! ABI change and every peer's assumption.
//!
//! # Why a copy is a child here and a sibling in seL4
//!
//! There is no `copy` operation. A copy is [`Table::derive`] with the rights it
//! already has, which makes it a child in the derivation tree rather than a
//! sibling of its source.
//!
//! seL4 puts a copy beside its source, and revoking the source does not reach
//! it. That is defensible where the mapping database is also the accounting
//! structure. It is not defensible here, because
//! `docs/what-must-be-stated.html` lists *nothing can be revoked* as a
//! structural drawback of the interface F is replacing, and answers it with
//! "revoke recursively through a derivation tree". A revoke that a copy escapes
//! is a revoke that does not answer that, and the failure is silent: the
//! authority is still out there and the log says it was withdrawn.
//!
//! The cost is stated rather than hidden. Two holders of equal authority are
//! not equal here — whoever derived first can revoke the other — so a component
//! that wants to hand out authority it cannot later reach has to be given the
//! capability itself rather than a copy of it. *What would reverse this:* a
//! case where that asymmetry is the wrong default, which is most likely a
//! broker component holding capabilities on behalf of others.
//!
//! # Why the derivation tree is parent pointers and not a list
//!
//! A slot stores the [`Handle`] it was derived from, not a pointer and not a
//! child list. Three reasons, in order of how much they matter.
//!
//! A handle is checkable. It carries the generation of the slot it names, so a
//! parent link into a slot that has since been cleared and refilled does not
//! silently point at the new occupant — the generations disagree and the link
//! reads as broken, which is what it is.
//!
//! Revocation walks the whole table rather than following pointers, so it is
//! quadratic in the slots that exist and has no recursion in it. A recursive
//! revoke in a kernel is a stack depth controlled by whoever built the tree,
//! and this kernel's stacks have a guard page precisely because that class of
//! bug is real. Bounded iteration cannot have it.
//!
//! What growth changed is whose memory bounds that walk. It used to be
//! [`TABLE_SLOTS`]², a thousand iterations of nothing; it is now
//! [`Table::capacity`]², which is what the component has paid for. The walk
//! stayed iterative and the marks moved from a `u32` into [`Condemned`], which
//! is a bitmap on the caller's stack rather than a second structure living
//! beside the table — a distinction this file already makes about child lists.
//! [`MAX_PAGES`] is the ceiling, and the quadratic is the reason it is small.
//!
//! And a child list would be a second structure that can disagree with the
//! first. There is exactly one place a parent relationship is recorded.
//!
//! # Property 5, mechanically
//!
//! The negative suite's fifth property is that a process cannot make the kernel
//! panic by trying. In this module that reduces to two constructs — an index
//! that was masked rather than checked, and an `unwrap` on a lookup that a
//! hostile handle can fail — so both are denied at compile time below, and
//! every slot is reached through `Table::at`, which is the one place a slot is
//! addressed at all. The dynamic half is [`properties::check`] and the
//! eleven `cap=` boots.
//!
//! See `docs/design/ring-scene-boot.html` section 15 milestone M4,
//! `docs/rfc/0015-capabilities-at-the-door.md`,
//! `docs/rfc/0008-no-fork-no-signals.md`, `docs/rfc/0029-a-table-is-bought.md`,
//! E0-B11 and E1-B13.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, Handle, rights};
use f_abi::control::{Grade, Pending, Promise};
use f_abi::error;

use crate::mem::{FRAME_SIZE, Frame, FrameAllocator};
use crate::percpu::PerCpu;

/// How many capabilities a process holds before it has paid for any.
///
/// Thirty-two, and the number is a floor rather than a bound. It is what sits
/// in the `PerCpu` static, and the reason it exists at all is bootstrap: the
/// frame fills a process's first four slots before the process exists to spend
/// anything, so there has to be somewhere to put them that nobody bought.
///
/// It stopped being the whole table at E1-B13. A component that needs more buys
/// more, a page at a time, out of its own [`CapType::Untyped`] — see
/// [`Table::grow`] — so this is the size of the part that is free rather than
/// the size of the table.
pub const TABLE_SLOTS: usize = 32;

/// How many pages of slots a table may buy.
///
/// Four, and the number comes from the revocation walk rather than from
/// memory: the walk is quadratic in [`Table::capacity`], so a table at this
/// ceiling is seventeen times the slots of one that has bought nothing and
/// nearly three hundred times the walk. Four is where that is still a bounded
/// loop of a few hundred thousand iterations of nothing, and it is the honest
/// ceiling to state — a component that has paid for four pages and asks for a
/// fifth is refused [`error::resource::QUOTA_EXHAUSTED`], which is the same
/// code a component that cannot pay earns.
///
/// **That conflation is a cost and not an accident.** Two different things —
/// *your account is empty* and *this build will not grow a table further* —
/// arrive as one refusal, and a caller cannot tell them apart. It is tolerable
/// only because no component in this tree can reach the ceiling: the largest
/// `Untyped` the frame hands out is one frame, which buys one page.
///
/// *Reversal:* a component that reaches the ceiling with an account still in
/// credit. At that point the two causes need separate codes in
/// [`error::RESOURCE`], and the walk needs to stop being quadratic first,
/// because raising this without that is buying iterations.
pub const MAX_PAGES: usize = 4;

/// How many slots fit in one page.
///
/// Computed from the slot rather than written down, because a slot that grew by
/// a field would otherwise silently overrun the frame it was written into.
pub const SLOTS_PER_PAGE: usize = FRAME_SIZE as usize / core::mem::size_of::<Slot>();

/// The most slots a table can ever hold in this build.
pub const MAX_SLOTS: usize = TABLE_SLOTS + MAX_PAGES * SLOTS_PER_PAGE;

// A slot that outgrew a page would make `SLOTS_PER_PAGE` zero and every lookup
// past the free part unreachable, which is a silent bound rather than a loud
// one.
const _: () = assert!(SLOTS_PER_PAGE > 0);

// [`Handle`] carries a sixteen-bit index. This is the assertion that the wire
// format still addresses every slot this build can create — the one thing about
// growth that would have been an ABI change, checked rather than asserted in
// prose.
const _: () = assert!(MAX_SLOTS < u16::MAX as usize);

/// Words in the revocation bitmap, one bit per slot.
const MARK_WORDS: usize = MAX_SLOTS.div_ceil(u64::BITS as usize);

/// Where the memory a table buys is reached.
///
/// A table charges a [`CapType::Untyped`] capability and gets back a physical
/// address; it cannot write to one. This is the step in between, and it is a
/// trait rather than a call into `mem` for one reason: the property suite has
/// to be able to grow a table too, and a suite that could only grow one on the
/// path a running process takes would be testing the path rather than the
/// table.
///
/// # Safety
///
/// [`Backing::reach`] must answer with an address at which `SLOTS_PER_PAGE`
/// slots of memory are writable for as long as the table holds them, owned by
/// nobody else, and correctly aligned for a slot. The table writes every slot
/// before it reads one, so the memory need not arrive initialised; everything
/// else in that sentence it cannot check and does not try to.
pub unsafe trait Backing {
    /// Somewhere to put a page of slots, for the frame at `phys`.
    ///
    /// `None` refuses the growth, and the table reports it as a quota that
    /// could not be met — which is the truthful answer, because a page that
    /// cannot be reached is a page that was not bought.
    fn reach(&mut self, phys: u64) -> Option<u64>;
}

/// The kernel's direct map, as a table's backing.
///
/// The one implementation a running process uses, and the one the property
/// suite uses too, so that the fixtures grow their tables through the same step
/// the frame does.
pub struct Direct<'a> {
    frames: &'a FrameAllocator,
}

impl<'a> Direct<'a> {
    /// Reach frames through this allocator's window onto physical memory.
    ///
    /// # Safety
    ///
    /// Every frame a table using this backing will charge for — which is every
    /// frame inside a [`CapType::Untyped`] capability that table holds — must
    /// be a frame this allocator gave out and nobody else holds, and `frames`
    /// must be rebound onto the address space that is live. The first is what
    /// makes writing there sound; the second is what makes the address the
    /// window computes a real one.
    #[must_use]
    pub const unsafe fn new(frames: &'a FrameAllocator) -> Self {
        Self { frames }
    }
}

// SAFETY: `reach` answers with the direct-map address of a frame the caller of
// `Direct::new` has guaranteed is owned by the table's account and by nobody
// else, which is exactly the obligation the trait states. `FrameAllocator::virt`
// is the one place physical becomes virtual in this kernel, so the address is
// the window's rather than a second belief about where memory is; a frame is
// `FRAME_SIZE` bytes and a page of slots is `SLOTS_PER_PAGE * size_of::<Slot>()`
// bytes, which is no larger by construction. Frame alignment exceeds a slot's.
unsafe impl Backing for Direct<'_> {
    fn reach(&mut self, phys: u64) -> Option<u64> {
        Some(self.frames.virt(Frame::from_addr(phys)) as u64)
    }
}

/// What a lookup found.
///
/// Reported back to a process by the inspect call, and used by the frame
/// wherever a capability authorises something. The object is in it
/// because the frame needs it; a process is told it too, and that is deliberate
/// — a physical address it cannot map without the capability is not a secret,
/// and pretending otherwise would be the kind of obscurity that gets mistaken
/// for a boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Found {
    /// What kind of object.
    pub kind: CapType,
    /// What the holder may do with it.
    pub rights: u8,
    /// The object itself: a physical address for a frame or an untyped region,
    /// a top-level table for an address space, a vector for an interrupt.
    pub object: u64,
    /// How many bytes, for the two types that span a range. Zero for the rest.
    pub extent: u64,
}

/// One slot.
///
/// `kind` is zero when the slot is empty, which is why [`CapType`] has no zero
/// discriminant: a zeroed table is an empty one and needs no separate flag that
/// could disagree with the type.
#[derive(Clone, Copy)]
struct Slot {
    /// A [`CapType`] wire value in the low five bits, or zero for empty, and
    /// what the frame owes about this slot in the top three.
    ///
    /// # Why two fields share a byte
    ///
    /// Because the alternative costs a fifth of every table a component buys.
    /// A slot is exactly thirty-two bytes with no padding, so a sixth field
    /// would have made it forty and cut a bought page from a hundred and
    /// twenty-eight slots to a hundred and two — spent on a field with five
    /// states. [`CapType`]'s wire values run to seven and the type is closed by
    /// [`CapType::from_wire`], so the top five bits of this byte were already
    /// unreachable; three of them now hold a [`Pending`], whose five states RFC
    /// 0008 fixes and `f_abi::control` implements.
    ///
    /// The whole cost is that occupancy is a mask rather than a comparison —
    /// [`Slot::occupied`] — and that is worth stating rather than hiding,
    /// because a slot that is empty and still owes a *revoked* notice is a real
    /// state and would look occupied to a naive test.
    kind: u8,
    /// The rights this capability carries.
    rights: u8,
    /// Which occupant of this slot. Counts from
    /// [`Handle::FIRST_GENERATION`]; a slot that reaches
    /// [`Handle::RETIRED_GENERATION`] is never filled again.
    generation: u16,
    /// The handle this was derived from, or [`Handle::NULL`] for a capability
    /// the frame granted directly.
    parent: u32,
    /// See [`Found::object`].
    object: u64,
    /// See [`Found::extent`].
    extent: u64,
    /// Where this capability's object has been mapped, or [`NOT_MAPPED`].
    ///
    /// The one thing in a slot that is not about *naming*, and it is here for a
    /// reason worth stating rather than deducing. Revocation withdraws a name;
    /// a mapping the withdrawn name authorised is a translation, and nothing
    /// else in this kernel knows those two are the same object. Recording the
    /// address in the slot is what makes revocation able to reach the mapping —
    /// and having it *in the slot* rather than in a table beside it is what
    /// stops the two disagreeing, which is the argument the parent link already
    /// makes about child lists.
    ///
    /// One address and not a list. A capability may be mapped once here, and a
    /// second attempt is refused rather than recorded. That is a real bound and
    /// it is not the general answer — a frame capability legitimately mapped at
    /// two addresses is an ordinary thing to want — but the general answer is a
    /// mapping database, which is a structure with an owner and a quota, which
    /// is E1's `Untyped`. Refusing is the honest small version: it cannot lose
    /// a mapping.
    mapped: u64,
}

/// What [`Slot::mapped`] says when the capability authorises no mapping.
///
/// Not zero, because zero is an address — the null page is unmapped in every
/// process this kernel runs, but "unmapped in practice" is not the same claim
/// as "cannot be an address", and a sentinel that is also a legal value is the
/// bug this constant exists to not have.
const NOT_MAPPED: u64 = u64::MAX;

impl Slot {
    /// An empty slot that has never held anything.
    const EMPTY: Self = Self::fresh(Handle::FIRST_GENERATION);

    /// An empty slot whose first occupant will be issued at `generation`.
    ///
    /// A bought page starts at the table's generation floor rather than at
    /// one, and that is the whole of why growth does not reopen the hole
    /// E0-B10 closed: a page is dropped when a process ends and the next
    /// process buys different memory, so a slot that started every page at the
    /// first generation would hand the next occupant of this core a handle the
    /// last one still holds.
    const fn fresh(generation: u16) -> Self {
        Self {
            kind: 0,
            rights: rights::NONE,
            generation,
            parent: Handle::NULL.bits(),
            object: 0,
            extent: 0,
            mapped: NOT_MAPPED,
        }
    }

    /// Is anything here now?
    ///
    /// A mask and not a comparison, because the same byte carries the pending
    /// notice: an empty slot that still owes a *revoked* notice has a non-zero
    /// `kind` and holds nothing.
    const fn occupied(self) -> bool {
        self.kind & KIND_MASK != 0
    }

    /// What kind of object, or `None` for empty and for a wire value this build
    /// does not define.
    const fn cap_kind(self) -> Option<CapType> {
        CapType::from_wire(self.kind & KIND_MASK)
    }

    /// What the frame owes about this slot.
    ///
    /// A value outside the five reads as [`Pending::Quiet`], which is the
    /// fail-closed direction here rather than the fail-open one: three bits
    /// cannot hold a sixth state this build wrote, so a value outside the five
    /// is memory corruption, and owing nothing is the only answer that cannot
    /// publish a notice naming a kind the component was never told about.
    const fn notice(self) -> Pending {
        match Pending::from_wire(self.kind >> KIND_BITS) {
            Some(pending) => pending,
            None => Pending::Quiet,
        }
    }

    /// Set what the frame owes, leaving the type alone.
    const fn set_notice(&mut self, pending: Pending) {
        self.kind = (self.kind & KIND_MASK) | (pending.to_wire() << KIND_BITS);
    }

    /// Set the type, leaving what the frame owes alone.
    const fn set_kind(&mut self, kind: u8) {
        self.kind = (self.kind & !KIND_MASK) | (kind & KIND_MASK);
    }
}

/// How many of a slot's type byte hold the type.
///
/// Five, which is more than [`CapType`] needs and is where the byte splits
/// evenly enough to leave three for a [`Pending`]. A seventh capability type
/// costs nothing; a thirty-second would cost the notice field, and that is the
/// bound worth stating.
const KIND_BITS: u8 = 5;

/// Which bits of a slot's type byte hold the type.
const KIND_MASK: u8 = (1 << KIND_BITS) - 1;

// Every capability type this build defines has to fit under the mask, or a
// slot would report a type it was never given and the notice field would move
// under it. Checked rather than assumed, because adding a type is a diff in
// `abi` that nothing here would otherwise notice.
const _: () = assert!(CapType::BufferSet.to_wire() <= KIND_MASK);
// And every notice state has to fit above it.
const _: () = assert!(Pending::GrantedThenPeerGone.to_wire() < (1 << (8 - KIND_BITS)));

/// Which slots a revocation has condemned, and how far the caller has read.
///
/// One bit per slot, on the caller's stack. It used to be a `u32` because a
/// table was thirty-two slots; it is a bitmap now because a table is as many
/// slots as its holder has paid for, and it is still a value passed between the
/// three steps of a revocation rather than a field of the table — a second
/// structure living beside the table is the thing this file refuses to have,
/// and a mark that outlived the operation would be exactly that.
///
/// # Why a revocation is three steps and not one
///
/// Because the mappings a revocation withdraws are no longer bounded by
/// anything small. A capability records one address, a table holds
/// [`MAX_SLOTS`] capabilities, and a single return value carrying every address
/// would be four kilobytes of kernel stack for the case where a process
/// revoked two things. So the caller condemns, drains the addresses one at a
/// time, and sweeps — and the order is its own, which matters because the drain
/// is the part that talks to page tables and other cores.
#[derive(Clone, Copy)]
pub struct Condemned {
    words: [u64; MARK_WORDS],
    cursor: usize,
}

impl Condemned {
    /// Nothing condemned.
    const NONE: Self = Self { words: [0; MARK_WORDS], cursor: 0 };

    /// Is this slot condemned?
    fn holds(&self, index: usize) -> bool {
        let word = index / u64::BITS as usize;
        let bit = index % u64::BITS as usize;
        self.words.get(word).is_some_and(|word| word & (1u64 << bit) != 0)
    }

    /// Condemn one slot.
    ///
    /// There is deliberately no count here. How many capabilities a revocation
    /// withdrew is [`Table::sweep`]'s answer, because the sweep is what
    /// withdraws them; a second count kept alongside would be a number that can
    /// disagree with what happened, which is the shape of bug this file spends
    /// most of its comments avoiding.
    fn mark(&mut self, index: usize) {
        let word = index / u64::BITS as usize;
        let bit = index % u64::BITS as usize;
        if let Some(word) = self.words.get_mut(word) {
            *word |= 1u64 << bit;
        }
    }
}

/// One process's capabilities.
///
/// `Copy` because [`PerCpu`] needs a `const` initialiser, and never actually
/// copied: every access goes through a raw pointer to this core's slot. A
/// table copied by value would be a second authority that can drift from the
/// first — and since E1-B13 it would also be a second owner of the pages the
/// first one bought.
#[derive(Clone, Copy)]
pub struct Table {
    slots: [Slot; TABLE_SLOTS],
    /// Where each bought page of slots was reached, in the order they were
    /// bought. Zero past [`Table::grown`].
    ///
    /// An address rather than a pointer, and the reason is [`PerCpu`]: its
    /// `Sync` rests on `T: Send`, and a raw pointer in a field would make this
    /// type not `Send` and the static not compile. Reconstructing the pointer
    /// where the access happens is also where the `SAFETY` comment belongs,
    /// which is the same split `PerCpu::mine` makes for the same reason.
    pages: [u64; MAX_PAGES],
    /// How many of `pages` are real.
    grown: u8,
    /// The generation the first occupant of a newly bought slot is issued at.
    ///
    /// See [`Slot::fresh`]. It only ever goes up, and it saturates for the same
    /// reason a slot's generation does.
    floor: u16,
    /// Whether this table's holder has somewhere to receive a notice.
    ///
    /// # Why this is a flag and not simply always true
    ///
    /// Because a notice is a completion entry on a control ring, and until
    /// E1-B05 not every process had one. The pending field exists to bound what
    /// the frame *owes*; a process with no ring is owed nothing, because there
    /// is nowhere for the debt to be paid. Setting the field for one would
    /// change nothing except that its slots would stop being refillable — RFC
    /// 0008's rule that a slot which is not quiet is not refilled — which is a
    /// quota shrinking for a component that cannot drain.
    ///
    /// *Reversal, and it is a deletion rather than a measurement:* when every
    /// process in this system is a component with a control ring — RFC 0030
    /// says that waits on E1-B08's safe adoption — this flag has one value and
    /// should go.
    posts_notices: bool,
    /// The earliest deadline this component has been stopped against.
    ///
    /// One word, and it only ever moves earlier: `f_abi::control::Promise` is
    /// the rule and R08 is why it is a rule rather than a convention.
    stop: Promise,
    /// The memory-pressure grade of the account that pays for this component.
    /// Latest wins.
    pressure: Grade,
    /// The system generation. Latest wins. RFC 0006 and RFC 0012 say what a
    /// component does about it; RFC 0008 reserves the word and this reserves
    /// the storage.
    generation: Grade,
}

/// The table of the process running on this core.
///
/// Per-CPU because a process runs on one core and its calls arrive on that one
/// — the same argument the process's own per-core state makes. The reason it
/// is a static at all is that a process's table must exist before there is any
/// allocator a process is entitled to draw on.
static TABLE: PerCpu<Table> = PerCpu::new(Table::EMPTY);

/// A pointer to this core's table.
///
/// Safe to call and unsafe to dereference, for the reason [`PerCpu::mine`]
/// gives.
#[must_use]
pub fn mine() -> *mut Table {
    TABLE.mine()
}

/// A pointer to another core's table.
///
/// The escape hatch [`PerCpu::at`] documents, used for the one case it names:
/// a core cannot fill its own first capability table, because everything that
/// would go in it — an address space, frames, an untyped region — comes from an
/// allocator that belongs to a core that is already running. So the boot
/// processor fills it before handing the core a process. See
/// `process::prepare`.
///
/// # Panics
///
/// If `cpu` is not a core this kernel shards for, which is [`PerCpu::at`]'s
/// panic and its reasoning.
#[must_use]
pub fn of(cpu: usize) -> *mut Table {
    TABLE.at(cpu)
}

impl Table {
    /// A table holding nothing and owing nothing.
    pub const EMPTY: Self = Self {
        slots: [Slot::EMPTY; TABLE_SLOTS],
        pages: [0; MAX_PAGES],
        grown: 0,
        floor: Handle::FIRST_GENERATION,
        posts_notices: false,
        stop: Promise::NONE,
        pressure: Grade::NONE,
        generation: Grade::NONE,
    };

    /// How many slots exist right now: the free ones plus the bought ones.
    ///
    /// Every loop in this file runs to this rather than to [`TABLE_SLOTS`],
    /// which is what makes the bound "what the holder paid for" rather than a
    /// constant.
    #[must_use]
    pub fn capacity(&self) -> usize {
        TABLE_SLOTS + self.grown as usize * SLOTS_PER_PAGE
    }

    /// Forget everything. Called when a process ends.
    ///
    /// Generations are *not* reset: a table that started every process at
    /// generation one would let a handle from the last process resolve in the
    /// next one, which is the whole failure the generation exists to prevent,
    /// reintroduced at the one boundary that matters most.
    ///
    /// The bought pages go too, because they were bought out of the ending
    /// process's `Untyped` and that memory returns to whoever paid for it. The
    /// addresses are simply forgotten here: the frame that a page was written
    /// into is inside the untyped region the caller is about to give back
    /// whole, so freeing it a second time from this file would be freeing it
    /// twice. `process::reap` is the one place that gives it back.
    ///
    /// What does *not* go is the generation floor, and that is the whole reason
    /// this method is more than three lines. The next process buys different
    /// memory for the same slot indices, so the floor has to carry across the
    /// boundary the way a slot's own generation does — see [`Slot::fresh`].
    pub fn clear_all(&mut self) {
        for index in 0..self.capacity() {
            if self.at(index).is_some_and(|slot| slot.occupied()) {
                self.clear(index);
            }
        }
        for index in TABLE_SLOTS..self.capacity() {
            // Every slot has just been cleared, so its generation is one past
            // the last handle it ever issued. The floor is the largest of
            // those, and a slot that was never filled contributes the floor it
            // already had.
            let reached = self.at(index).map_or(self.floor, |slot| slot.generation);
            self.floor = self.floor.max(reached);
        }
        self.pages = [0; MAX_PAGES];
        self.grown = 0;

        // Nothing is owed to a component that no longer exists. RFC 0008 is
        // explicit that a component's own state tree goes with its memory and
        // that there are no last words after a fault; the same is true of a
        // notice, and keeping one would be the frame holding a debt to a
        // creditor it has already torn the ring down for.
        for index in 0..TABLE_SLOTS {
            if let Some(slot) = self.slots.get_mut(index) {
                slot.set_notice(Pending::Quiet);
            }
        }
        self.posts_notices = false;
        self.stop = Promise::NONE;
        self.pressure = Grade::NONE;
        self.generation = Grade::NONE;
    }

    /// Say that this table's holder has a control ring, so notices are owed.
    ///
    /// Called once, by the spawn that creates the ring, and never unset except
    /// by [`Table::clear_all`]. See [`Table::posts_notices`].
    pub const fn owes_notices(&mut self) {
        self.posts_notices = true;
    }

    /// Is a notice owed for anything at all?
    ///
    /// The question a polling point asks before it walks. Cheap for the two
    /// promise words and linear in the slots, which is the same bound
    /// everything else in this file has.
    #[must_use]
    pub fn owes(&self) -> u32 {
        let mut owed = u32::from(self.stop.is_owed())
            + u32::from(self.pressure.is_owed())
            + u32::from(self.generation.is_owed());
        for index in 0..self.capacity() {
            owed += self.at(index).map_or(0, |slot| slot.notice().owed());
        }
        owed
    }

    /// Stop this component by `deadline`, keeping the earlier of it and any
    /// stop already pending.
    ///
    /// Answers whether the promise moved, which is what lets the caller
    /// complete a second stop with *which deadline it kept* rather than with a
    /// bare success the submitter would misread. RFC 0008: a promise that can
    /// be silently relaxed by whoever made it is not a deadline.
    pub const fn stop_by(&mut self, deadline: u64) -> bool {
        self.stop.promise(deadline)
    }

    /// The deadline this component has been stopped against, if any.
    #[must_use]
    pub const fn stop_deadline(&self) -> Option<u64> {
        self.stop.deadline()
    }

    /// The pressure grade changed for the account that pays for this component.
    pub const fn pressure_is(&mut self, grade: u64) -> bool {
        self.pressure.set(grade)
    }

    /// The system generation changed.
    pub const fn generation_is(&mut self, generation: u64) -> bool {
        self.generation.set(generation)
    }

    /// Note that the far end of what this handle names has ended.
    ///
    /// The frame's side of a peer death: the holder is told, and the capability
    /// is *not* withdrawn — an endpoint survives its occupant, which is the
    /// whole of RFC 0008's *a place is not an instance*.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`]. A handle that names nothing is a frame bug rather
    /// than a component's, and it is refused rather than asserted for the
    /// reason property five gives.
    pub fn note_peer_gone(&mut self, handle: Handle) -> Result<(), i32> {
        let index = self.resolve(handle)?;
        let owes = self.posts_notices;
        let slot = self.at_mut(index).ok_or_else(no_such)?;
        if owes {
            let next = slot.notice().peer_gone();
            slot.set_notice(next);
        }
        Ok(())
    }

    /// The next notice owed about a *slot*, in slot order.
    ///
    /// The first phase of `f_abi::control::ORDER`. `None` when every slot is
    /// quiet.
    pub fn next_slot_notice(&mut self, timestamp: u64) -> Option<f_abi::Cqe> {
        for index in 0..self.capacity() {
            let slot = self.at(index)?;
            let (kind, next) = slot.notice().drain();
            let Some(kind) = kind else { continue };
            let handle = Handle::new(index as u16, slot.generation);
            self.at_mut(index)?.set_notice(next);
            return Some(f_abi::control::entry(kind, u64::from(handle.bits()), 0, timestamp));
        }
        None
    }

    /// The stop notice, if one is owed. The second phase.
    ///
    /// `user_data` is the control ring's own handle, which the caller supplies
    /// because the table does not know which of its slots that is — the frame
    /// puts it there and the frame is the one publishing.
    pub const fn next_stop_notice(&mut self, ring: Handle, timestamp: u64) -> Option<f_abi::Cqe> {
        match self.stop.drain() {
            Some(deadline) => Some(f_abi::control::entry(
                f_abi::control::notice::STOP,
                ring.bits() as u64,
                deadline,
                timestamp,
            )),
            None => None,
        }
    }

    /// The two grades, in the fixed order pressure-then-generation. The fourth
    /// phase; the third is reclaim, which is the scheduler's and lives beside
    /// the allocation rather than in a table — RFC 0008 puts it there because
    /// it is bounded by cores held rather than by slots bought, and
    /// `component::Instance` splices it in at the position `ORDER` fixes.
    pub const fn next_grade_notice(&mut self, timestamp: u64) -> Option<f_abi::Cqe> {
        if let Some(grade) = self.pressure.drain() {
            return Some(f_abi::control::entry(
                f_abi::control::notice::PRESSURE,
                0,
                grade,
                timestamp,
            ));
        }
        match self.generation.drain() {
            Some(generation) => Some(f_abi::control::entry(
                f_abi::control::notice::GENERATION,
                0,
                generation,
                timestamp,
            )),
            None => None,
        }
    }

    /// How many capabilities are held.
    #[must_use]
    pub fn used(&self) -> usize {
        (0..self.capacity()).filter(|index| self.at(*index).is_some_and(Slot::occupied)).count()
    }

    /// How many slots have been used up and may never be filled again.
    ///
    /// Reported so that the honest cost of not wrapping the generation is
    /// visible rather than theoretical. Zero for the life of any process this
    /// kernel currently runs.
    #[must_use]
    pub fn retired(&self) -> usize {
        (0..self.capacity())
            .filter(|index| {
                self.at(*index).is_some_and(|slot| {
                    !slot.occupied() && slot.generation == Handle::RETIRED_GENERATION
                })
            })
            .count()
    }

    /// Put a capability in, as the root of a derivation tree.
    ///
    /// This is the frame granting authority and there is no call behind it: a
    /// process cannot reach it, which is what "authority arrives by grant and
    /// by nothing else" means mechanically.
    ///
    /// # Errors
    ///
    /// [`error::RESOURCE`] when there is no free slot. A grant does not grow
    /// the table, and the reason is that the frame has nothing to charge: a
    /// grant is authority arriving from outside, at a moment when the component
    /// may not exist yet, and a table that could buy itself a page on the
    /// frame's say-so would be the kernel reserve RFC 0008 refuses to have.
    /// Only [`Table::derive`] grows, because only a component spends its own
    /// account.
    ///
    /// *What E1-B05 has to decide:* RFC 0008 has the frame placing capabilities
    /// into a running component's table — a powerbox grant, a spawn's needs —
    /// and every one of those is a grant that may find the table full. The
    /// answer is either that the placing component pays out of the `Untyped` it
    /// is already spending on the spawn, or that a grant into a full table is
    /// refused and the supervisor grows the child first. This task does not
    /// choose, because there is no second component yet to choose for.
    pub fn grant(
        &mut self,
        kind: CapType,
        rights: u8,
        object: u64,
        extent: u64,
    ) -> Result<Handle, i32> {
        self.place(kind, rights, object, extent, Handle::NULL)
    }

    /// What a handle names, or why it does not name anything.
    ///
    /// # Errors
    ///
    /// [`error::authority::NO_SUCH_CAP`] when the handle names no capability
    /// this table ever held, and [`error::authority::REVOKED`] when it named
    /// one that is gone. The two are separate because a component recovers from
    /// them differently: one is a bug in the component, the other is an event
    /// that happened to it.
    pub fn inspect(&self, handle: Handle) -> Result<Found, i32> {
        let index = self.resolve(handle)?;
        let slot = self.slot(index)?;
        let kind = slot.cap_kind().ok_or(no_such())?;
        Ok(Found { kind, rights: slot.rights, object: slot.object, extent: slot.extent })
    }

    /// What a handle names, refusing it unless it carries every right in
    /// `asked` and is of the kind `kind`.
    ///
    /// The one entry point for *using* a capability. Every caller in the frame
    /// goes through it rather than calling [`Table::inspect`] and checking
    /// afterwards, because a check that a caller performs is a check a caller
    /// can forget — and the forgetting looks like working code.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`], plus [`error::authority::WRONG_TYPE`] and
    /// [`error::authority::RIGHT_NOT_HELD`].
    pub fn invoke(&self, handle: Handle, kind: CapType, asked: u8) -> Result<Found, i32> {
        let found = self.inspect(handle)?;
        if found.kind != kind {
            return Err(error::pack(error::AUTHORITY, error::authority::WRONG_TYPE));
        }
        if !rights::holds(found.rights, asked) {
            return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
        }
        Ok(found)
    }

    /// Mint a weaker capability from one this table holds.
    ///
    /// The child is a child in the derivation tree whether or not it is weaker,
    /// so a copy — `asked` equal to what the parent carries — is revoked with
    /// its source. The module comment argues that against seL4.
    ///
    /// Deriving from [`CapType::Untyped`] retypes: the child is a
    /// [`CapType::Frame`] naming the next unclaimed frame of the region, and
    /// the parent's watermark advances. There is no way to *copy* an untyped
    /// capability at M4 and that is a stated limit, not an oversight — the
    /// operation that separates the two is an explicit retype with the target
    /// type and sub-range as operands, and it belongs on a ring (M5) rather
    /// than at the door.
    ///
    /// # Errors
    ///
    /// [`error::authority::RIGHT_NOT_HELD`] when the parent does not carry
    /// [`rights::DERIVE`] or when `asked` is not a narrowing of what it holds,
    /// [`error::argument::UNKNOWN_FLAG`] for a rights bit this build does not
    /// define, and [`error::resource::QUOTA_EXHAUSTED`] when an untyped region
    /// is exhausted or the table is full and nothing this component holds can
    /// pay for another page of it.
    pub fn derive(
        &mut self,
        handle: Handle,
        asked: u8,
        backing: &mut dyn Backing,
    ) -> Result<Handle, i32> {
        if rights::unknown(asked) {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }
        let parent = self.invoke_any(handle, rights::DERIVE)?;
        if !rights::narrows(parent.rights, asked) {
            return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
        }
        // Somewhere to put it, before anything is spent. The table being full
        // is, since E1-B13, a question rather than an answer — *can this
        // component buy another page?* — and asking it here rather than after
        // the retype is not tidiness: a retype advances an untyped region's
        // watermark and there is no un-advancing one, so a derive that
        // discovered the table was full afterwards would have charged a
        // component for a frame it then refused to hand over. A grow that
        // succeeds leaves a whole page of vacancies, so the placement below
        // cannot fail for want of one.
        if self.vacancy().is_none() {
            self.grow(backing)?;
        }
        let (kind, object, extent) = self.retype(handle)?;
        self.place(kind, asked, object, extent, handle)
    }

    /// Buy one more page of slots, charging a capability this table holds.
    ///
    /// The account is the lowest-indexed [`CapType::Untyped`] that carries
    /// [`rights::DERIVE`] and has a frame left in it. Lowest for the reason
    /// [`Table::place`] fills the lowest free slot: two runs of one program
    /// must charge the same account, or the boot log stops being a fixture.
    /// `DERIVE` because growing is retyping — the same operation, advancing the
    /// same watermark — and an untyped region a component may not retype is one
    /// it may not spend either.
    ///
    /// There is no call behind this today. The request that reaches it is a
    /// derive with nowhere to put its child, which is `on request` in the only
    /// sense the door still supports; RFC 0008 shrinks the door rather than
    /// growing it, so an explicit grow is an opcode on the control ring and
    /// belongs to E1-B05.
    ///
    /// # Errors
    ///
    /// [`error::resource::QUOTA_EXHAUSTED`] when nothing can pay, when the
    /// backing cannot reach the frame that was going to be charged, or when the
    /// table is already at [`MAX_PAGES`]. [`MAX_PAGES`] says why the last of
    /// those three shares a code with the first two and what would reverse it.
    pub fn grow(&mut self, backing: &mut dyn Backing) -> Result<(), i32> {
        let page = self.grown as usize;
        if page >= MAX_PAGES {
            return Err(exhausted());
        }
        let mut account = None;
        for index in 0..self.capacity() {
            let Some(slot) = self.at(index) else { continue };
            if slot.occupied()
                && slot.cap_kind() == Some(CapType::Untyped)
                && rights::holds(slot.rights, rights::DERIVE)
                && slot.extent >= FRAME_SIZE
            {
                account = Some(index);
                break;
            }
        }
        let Some(account) = account else { return Err(exhausted()) };
        let phys = self.at(account).map(|slot| slot.object).ok_or_else(exhausted)?;
        let base = backing.reach(phys).ok_or_else(exhausted)?;

        // Written before the page is counted, so that nothing can read a slot
        // this loop has not filled in — which is also what makes the shared
        // slice `at` builds afterwards a slice of initialised memory.
        let floor = self.floor;
        for index in 0..SLOTS_PER_PAGE {
            let slot = (base as *mut Slot).wrapping_add(index);
            // SAFETY: `Backing::reach` promises `SLOTS_PER_PAGE` slots of
            // writable memory at `base`, aligned for a slot and owned by nobody
            // else, for as long as this table holds them — and `index` is below
            // that count. Nothing has read this page: it is not in `pages` yet
            // and `grown` has not moved, so `capacity` does not reach it.
            unsafe { slot.write(Slot::fresh(floor)) };
        }
        let Some(at) = self.pages.get_mut(page) else { return Err(exhausted()) };
        *at = base;
        self.grown = self.grown.saturating_add(1);

        // Charged last, and only once the page exists. A component that was
        // billed for a page it did not get would have no way to find out.
        if let Some(slot) = self.at_mut(account) {
            slot.object = slot.object.wrapping_add(FRAME_SIZE);
            slot.extent = slot.extent.wrapping_sub(FRAME_SIZE);
        }
        Ok(())
    }

    /// Give up a capability this table holds, and everything derived from it.
    ///
    /// # Why this is not the revoke a component makes
    ///
    /// [`Table::condemn`] deliberately spares the capability it is given:
    /// revoke withdraws what was *handed on*, and a holder that wants to give up
    /// its own authority is asking a different question — which
    /// `docs/rfc/0015-capabilities-at-the-door.md` leaves unanswered on purpose,
    /// because "drop this" and "take back what I gave" have different callers
    /// and different mistakes.
    ///
    /// This is the frame answering it for itself, and only for itself. A
    /// supervisor that minted a `Frame` out of its account on a component's
    /// behalf still holds that name after the component is torn down, and the
    /// account is about to hand the same page to the next instance — so a frame
    /// that did not drop the name would be holding authority over memory the
    /// next occupant is being given, which is the one thing a restart may not
    /// leave behind. There is no opcode behind it and there must not be until
    /// somebody argues for one.
    ///
    /// Answers how many slots were emptied, this one included.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`], plus [`error::authority::RIGHT_NOT_HELD`] when the
    /// capability does not carry [`rights::REVOKE`] — the same right giving up
    /// its descendants needs, because giving up the parent gives them up too.
    pub fn relinquish(&mut self, handle: Handle) -> Result<u32, i32> {
        let mut condemned = self.condemn(handle)?;
        let index = self.resolve(handle)?;
        condemned.mark(index);
        Ok(self.sweep(&condemned))
    }

    /// The mappings a relinquish would have to withdraw first.
    ///
    /// The same drain [`Table::next_mapping`] performs for a revocation, and it
    /// is the caller's to run before [`Table::relinquish`] for the same reason:
    /// authority is withdrawn only after every translation behind it is gone.
    pub fn condemn_own(&self, handle: Handle) -> Result<Condemned, i32> {
        let mut condemned = self.condemn(handle)?;
        condemned.mark(self.resolve(handle)?);
        Ok(condemned)
    }

    /// Give the top of an untyped region back to it.
    ///
    /// # Why this exists and why there is no call behind it
    ///
    /// RFC 0008 step 4 of a component's death: *return the memory to the
    /// `Untyped` it was retyped from* — what an account paid for comes back to
    /// that account, not to a global free list, which is what makes a
    /// supervisor's quota a real number after its children have lived and died.
    ///
    /// A watermark can only give back the top, and that is the whole of the
    /// bound here: this is sound because a place has one occupant at a time, so
    /// the frames an instance was made of are exactly the last ones retyped and
    /// nothing has been retyped since. The general answer — an account that can
    /// take back memory from anywhere in its middle — is a free list per
    /// account, which is a structure with an owner and a quota, and that is not
    /// a structure the frame gets to invent on a component's behalf.
    ///
    /// There is no opcode behind this and there must not be. A component that
    /// could rewind its own watermark could un-spend: hand out a frame, have it
    /// retyped into somebody else's table, rewind, and retype the same frame
    /// again. This is the frame refunding an account for a component the frame
    /// itself has just torn down, which is the one caller for which "nothing
    /// has been retyped since" is a fact rather than a hope.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`], plus [`error::authority::WRONG_TYPE`] when the
    /// handle does not name an untyped region and
    /// [`error::argument::BAD_ADDRESS`] when the refund would take the region
    /// below where it started — which is the frame having lost count, and is
    /// refused rather than allowed to hand out memory nobody owns.
    pub fn refund(&mut self, handle: Handle, bytes: u64, floor: u64) -> Result<(), i32> {
        let index = self.resolve(handle)?;
        let slot = self.at_mut(index).ok_or_else(no_such)?;
        if slot.cap_kind() != Some(CapType::Untyped) {
            return Err(error::pack(error::AUTHORITY, error::authority::WRONG_TYPE));
        }
        if slot.object < floor.saturating_add(bytes) {
            return Err(error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS));
        }
        slot.object -= bytes;
        slot.extent = slot.extent.saturating_add(bytes);
        Ok(())
    }

    /// Mark everything derived from a capability, however deep.
    ///
    /// The first of a revocation's three steps; [`Condemned`] says why there
    /// are three. The capability itself is not among them: revoke withdraws
    /// what was handed on, and a holder that wants to give up its own authority
    /// is asking a different question.
    ///
    /// Nothing has changed when this returns. The marks are the caller's, and a
    /// caller that condemns and then does not sweep has withdrawn nothing —
    /// which is the failure mode worth having, because the other order would
    /// withdraw authority and then discover it could not unmap.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`], plus [`error::authority::RIGHT_NOT_HELD`] when
    /// the capability does not carry [`rights::REVOKE`].
    pub fn condemn(&self, handle: Handle) -> Result<Condemned, i32> {
        self.invoke_any(handle, rights::REVOKE)?;
        Ok(self.descendants(handle))
    }

    /// The next address a condemned capability had authorised a mapping at.
    ///
    /// # Why the mappings come back rather than being undone here
    ///
    /// Because undoing one is a page table edit followed by an interrupt to
    /// every other core, and this file knows about neither. It knows which
    /// authority has been withdrawn, which is the question it is for; the
    /// caller knows which address space and which cores. `process::withdraw` is
    /// where the two meet.
    ///
    /// One at a time, because there are as many of these as the holder has paid
    /// for slots and a single return value carrying all of them would be a
    /// kernel stack frame sized by a component's quota.
    pub fn next_mapping(&self, condemned: &mut Condemned) -> Option<u64> {
        while condemned.cursor < self.capacity() {
            let index = condemned.cursor;
            condemned.cursor += 1;
            if !condemned.holds(index) {
                continue;
            }
            if let Some(slot) = self.at(index)
                && slot.mapped != NOT_MAPPED
            {
                return Some(slot.mapped);
            }
        }
        None
    }

    /// Clear every condemned slot, and say how many that was.
    pub fn sweep(&mut self, condemned: &Condemned) -> u32 {
        let mut count = 0;
        for index in 0..self.capacity() {
            if condemned.holds(index) {
                self.clear(index);
                count += 1;
            }
        }
        count
    }

    /// Record that this capability's object is now mapped at `virt`.
    ///
    /// Called by the frame after a mapping has actually been made, never
    /// before: a slot that recorded an address the tables do not have would
    /// make the next revoke unmap somebody else's page.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`], plus [`error::argument::BAD_ADDRESS`] when this
    /// capability is already mapped somewhere. [`Slot::mapped`] says why that
    /// is a refusal rather than a list.
    pub fn note_mapping(&mut self, handle: Handle, virt: u64) -> Result<(), i32> {
        let index = self.resolve(handle)?;
        let slot = self.at_mut(index).ok_or_else(no_such)?;
        if slot.mapped != NOT_MAPPED {
            return Err(error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS));
        }
        slot.mapped = virt;
        Ok(())
    }

    // ---- the parts the flawed fixtures in `properties` also build from -----

    /// The slot at `index`, whichever page it is in.
    ///
    /// The one place a slot is addressed, which is what makes property five
    /// checkable at all: a table in two pieces would otherwise be two places to
    /// get a bound wrong. Everything above runs `0..capacity()` and reads
    /// through here, so an index past what was paid for is `None` rather than
    /// memory.
    fn at(&self, index: usize) -> Option<Slot> {
        if index < TABLE_SLOTS {
            return self.slots.get(index).copied();
        }
        let above = index - TABLE_SLOTS;
        let page = above / SLOTS_PER_PAGE;
        if page >= self.grown as usize {
            return None;
        }
        let base = *self.pages.get(page)?;
        // SAFETY: `base` is what `Backing::reach` answered for this page, which
        // promises `SLOTS_PER_PAGE` slots of memory owned by nobody else for as
        // long as this table holds it, and `grow` wrote every one of them
        // before it counted the page — so this is a slice of initialised slots.
        // It is dropped from `pages` by `clear_all` before the process that
        // paid for it gives the memory back, so the promise has not expired.
        let slots = unsafe { core::slice::from_raw_parts(base as *const Slot, SLOTS_PER_PAGE) };
        slots.get(above % SLOTS_PER_PAGE).copied()
    }

    /// The slot at `index`, to be written.
    fn at_mut(&mut self, index: usize) -> Option<&mut Slot> {
        if index < TABLE_SLOTS {
            return self.slots.get_mut(index);
        }
        let above = index - TABLE_SLOTS;
        let page = above / SLOTS_PER_PAGE;
        if page >= self.grown as usize {
            return None;
        }
        let base = *self.pages.get(page)?;
        // SAFETY: as `at`, and exclusive rather than shared because the
        // `&mut self` this borrows from is the only way to reach the page:
        // `pages` is private, a table is never copied while it owns one, and
        // the reference this returns cannot outlive the borrow it came from.
        let slots = unsafe { core::slice::from_raw_parts_mut(base as *mut Slot, SLOTS_PER_PAGE) };
        slots.get_mut(above % SLOTS_PER_PAGE)
    }

    /// Which slot a handle names, or why none.
    ///
    /// The whole of the unforgeability check, in one place so that there is one
    /// place to read when asking whether it is right.
    fn resolve(&self, handle: Handle) -> Result<usize, i32> {
        // Generation zero is never issued, so a zeroed word — a submission that
        // was memset, a register nobody set — names nothing rather than naming
        // slot zero.
        if !handle.is_issuable() {
            return Err(no_such());
        }
        let index = handle.index() as usize;
        // Checked, never masked, and against what this table has *paid for*
        // rather than against a constant. A mask is the bug this returns an
        // error for.
        //
        // Absent under `mutate-unchecked-index`, which is the deliberate defect
        // property five's mutation harness builds. See [`Table::slot`].
        #[cfg(not(feature = "mutate-unchecked-index"))]
        if index >= self.capacity() {
            return Err(no_such());
        }
        self.resolve_at(index, handle)
    }

    /// The slot at `index`, or the refusal an index that names none earns.
    ///
    /// The one place this table is subscripted, which is what makes property
    /// five checkable at all: *a process cannot panic the kernel by trying*
    /// reduces, in this module, to two constructs — an index that was masked
    /// rather than checked, and one that was neither — and both of them are
    /// here.
    #[cfg(not(feature = "mutate-unchecked-index"))]
    fn slot(&self, index: usize) -> Result<Slot, i32> {
        self.at(index).ok_or_else(no_such)
    }

    /// The same lookup with one deliberate defect in it: the index is used
    /// rather than checked.
    ///
    /// # Why this is in the shipped source and not in a test
    ///
    /// Because it is the half of E0-P08 a fixture cannot be. The other four
    /// properties have a flawed table in [`properties::Flawed`] that breaks them
    /// at run time and is caught by [`properties::check`]; this one cannot,
    /// because a fixture that panics takes the machine down rather than being
    /// caught, and there is no host harness to catch it in — `kernel/Cargo.toml`
    /// says why there is not.
    ///
    /// So the mutation is a *build* rather than a fixture. `cargo xtask mutate`
    /// builds the kernel with `mutate-unchecked-index`, boots it into the
    /// forging sweep, and requires the boot to go red with a kernel panic; then
    /// builds it without and requires the same boot to go green. That pair is
    /// what makes the property falsifiable, and neither half of it means
    /// anything alone.
    ///
    /// It differs from the real lookup in exactly one step, which is the lesson
    /// E0-B11 recorded the hard way: a fixture that breaks two things at once is
    /// caught by whichever check notices first, and the check it was written for
    /// stays unexercised. Everything else about a lookup — the generation, the
    /// occupancy, the refusal codes — is shared with the function above.
    ///
    /// The `allow` is the marker. The module denies `indexing_slicing` outright
    /// so that this construct cannot be written by accident; writing it on
    /// purpose takes an attribute that says so, and `cargo xtask lint-mutations`
    /// checks that no build has the feature on by default.
    ///
    /// It subscripts the free part of the table rather than [`Table::at`],
    /// which is the same defect in the shape growth left it: the mistake being
    /// modelled is *the index was used and not checked*, and using it means
    /// using it on the array that is actually there.
    #[cfg(feature = "mutate-unchecked-index")]
    #[allow(clippy::indexing_slicing, reason = "the deliberate defect; see the doc comment")]
    fn slot(&self, index: usize) -> Result<Slot, i32> {
        Ok(self.slots[index])
    }

    /// The rest of a lookup, once the index is known to be one.
    ///
    /// Split from [`Table::resolve`] so that the flawed fixture whose mistake
    /// is a masked index can reuse everything else. A mutation that changed two
    /// things at once would be caught by whichever check noticed first, and the
    /// check it was written for would stay unexercised — which is exactly the
    /// failure `properties::self_test` reports as a wrong property.
    fn resolve_at(&self, index: usize, handle: Handle) -> Result<usize, i32> {
        let slot = self.slot(index)?;

        if handle.generation() != slot.generation {
            // Older than the slot means it named an occupant that has been
            // cleared. Newer means it was never issued by anybody.
            return Err(if handle.generation() < slot.generation { revoked() } else { no_such() });
        }
        if !slot.occupied() {
            return Err(no_such());
        }
        Ok(index)
    }

    /// [`Table::invoke`] without a type check, for the two operations that act
    /// on a capability of any kind.
    fn invoke_any(&self, handle: Handle, asked: u8) -> Result<Found, i32> {
        let found = self.inspect(handle)?;
        if !rights::holds(found.rights, asked) {
            return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
        }
        Ok(found)
    }

    /// Put a capability in the lowest free slot.
    ///
    /// Lowest rather than next, so that the same sequence of operations
    /// produces the same handles on every run — a boot log is a fixture, and a
    /// handle in it that depended on allocation history would not be one.
    fn place(
        &mut self,
        kind: CapType,
        rights: u8,
        object: u64,
        extent: u64,
        parent: Handle,
    ) -> Result<Handle, i32> {
        let index = self.vacancy().ok_or_else(exhausted)?;
        let owes = self.posts_notices;
        let slot = self.at_mut(index).ok_or_else(exhausted)?;
        slot.set_kind(kind.to_wire());
        slot.rights = rights;
        slot.object = object;
        slot.extent = extent;
        slot.parent = parent.bits();
        slot.mapped = NOT_MAPPED;
        // The slot was quiet — `vacancy` says so — so this cannot overwrite a
        // notice. A component with nowhere to receive one is owed nothing; see
        // [`Table::posts_notices`].
        if owes {
            let next = slot.notice().granted();
            slot.set_notice(next);
        }
        let generation = slot.generation;
        // `index` is below MAX_SLOTS, which the assertion at the top of this
        // file keeps below u16::MAX.
        Ok(Handle::new(index as u16, generation))
    }

    /// The lowest slot a capability could go in.
    ///
    /// Asked separately from [`Table::place`] because whether there is one has
    /// to be known *before* a derive retypes anything — see [`Table::derive`].
    ///
    /// A retired slot is not one: the generation test is equality rather than
    /// "at least" because `clear` saturates, so a generation cannot go past the
    /// retirement value and the two are the same test.
    fn vacancy(&self) -> Option<usize> {
        (0..self.capacity()).find(|index| {
            self.at(*index).is_some_and(|slot| {
                // Not occupied, not retired, and **not owing a notice**. The
                // third is RFC 0008's, and it is what keeps a handle's
                // generation honest under a pending one: a *revoked* notice
                // always names a handle whose slot has not been reissued, so a
                // component can match it against what it holds rather than
                // against whatever arrived in the meantime. The cost is that a
                // component which never drains its control ring runs out of
                // table before it runs out of memory — which is the failure we
                // want, local and refused, and is still a failure somebody will
                // meet.
                !slot.occupied()
                    && slot.generation != Handle::RETIRED_GENERATION
                    && slot.notice().is_quiet()
            })
        })
    }

    /// What a derivation of the capability at `handle` produces, advancing an
    /// untyped region's watermark if that is what it is.
    fn retype(&mut self, handle: Handle) -> Result<(CapType, u64, u64), i32> {
        let index = self.resolve(handle)?;
        let slot = self.at_mut(index).ok_or_else(no_such)?;
        let kind = slot.cap_kind().ok_or_else(no_such)?;
        if kind != CapType::Untyped {
            return Ok((kind, slot.object, slot.extent));
        }
        if slot.extent < FRAME_SIZE {
            return Err(exhausted());
        }
        let object = slot.object;
        // Checked immediately above, so neither of these can wrap.
        slot.object = object.wrapping_add(FRAME_SIZE);
        slot.extent = slot.extent.wrapping_sub(FRAME_SIZE);
        Ok((CapType::Frame, object, FRAME_SIZE))
    }

    /// Every slot below `handle` in the derivation tree.
    ///
    /// Iterative and bounded: each pass marks at least one slot or is the last,
    /// so it runs at most [`Table::capacity`] times over a loop of the same
    /// length. See the module comment on why this is not the obvious recursion,
    /// and [`MAX_PAGES`] on why the square of a number a component chooses is
    /// the reason there is a ceiling at all.
    fn descendants(&self, handle: Handle) -> Condemned {
        let mut doomed = Condemned::NONE;
        loop {
            let mut found = false;
            for index in 0..self.capacity() {
                if doomed.holds(index) {
                    continue;
                }
                let Some(slot) = self.at(index) else { continue };
                if !slot.occupied() {
                    continue;
                }
                let parent = Handle::from_bits(slot.parent);
                if !parent.is_issuable() {
                    continue;
                }
                if parent == handle || self.marked_parent(parent, &doomed) {
                    doomed.mark(index);
                    found = true;
                }
            }
            if !found {
                return doomed;
            }
        }
    }

    /// Is this parent link one of the slots already condemned?
    ///
    /// The generation is compared as well as the index, because a parent link
    /// into a slot that has since been refilled names the old occupant and not
    /// the new one — and treating it as the new one would revoke a capability
    /// that has nothing to do with the one being withdrawn.
    fn marked_parent(&self, parent: Handle, doomed: &Condemned) -> bool {
        let index = parent.index() as usize;
        if !doomed.holds(index) {
            return false;
        }
        self.at(index).is_some_and(|slot| slot.generation == parent.generation())
    }

    /// Empty one slot and move it on to its next generation.
    ///
    /// Saturating, and the saturation is the point: a generation that wrapped
    /// would make a handle held since before the wrap valid again. A slot that
    /// runs out is retired rather than reused, which turns a hole in the
    /// authority model into a table that is one slot smaller.
    fn clear(&mut self, index: usize) {
        let owes = self.posts_notices;
        let Some(slot) = self.at_mut(index) else { return };
        if owes {
            // Rule 1 and rule 3 of RFC 0008's collision table, both of them in
            // `Pending::revoked`: an undelivered grant that is revoked posts
            // nothing and goes quiet, and revoked otherwise supersedes peer
            // gone. Neither is decided here, which is the point of the state
            // machine being in `abi` — there is one implementation of the three
            // rules and it is tested at every collision on the host.
            let next = slot.notice().revoked();
            slot.set_notice(next);
        }
        slot.set_kind(0);
        slot.rights = rights::NONE;
        slot.object = 0;
        slot.extent = 0;
        slot.parent = Handle::NULL.bits();
        slot.mapped = NOT_MAPPED;
        slot.generation = slot.generation.saturating_add(1);
    }
}

/// The handle names nothing this table ever held.
fn no_such() -> i32 {
    error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP)
}

/// A quota was reached: the component's own, or the frame's ceiling on how far
/// a table may be grown. [`MAX_PAGES`] says why those two share a code.
fn exhausted() -> i32 {
    error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED)
}

/// The handle named something that has been withdrawn.
fn revoked() -> i32 {
    error::pack(error::AUTHORITY, error::authority::REVOKED)
}

/// The five properties the negative suite is, and the implementations that
/// break one each.
///
/// # Why the broken implementations ship in the kernel image
///
/// Because the kernel has no host test harness — `kernel/Cargo.toml` says why
/// — so a fixture that only exists under `cfg(test)` is a fixture that never
/// runs. `f_env::contract` has the same shape and solves it the other way,
/// with six environments broken on purpose in a test module, because `env` is
/// a host crate and can.
///
/// They are constructed in [`self_test`] and nowhere else, and nothing in ring
/// 3 can reach one. That is the same status the deliberately hostile program
/// in `arch::x86_64::probe` already has, and the same status `fault=` has: this
/// tree ships its adversaries, because an adversary that is not in the image is
/// an adversary nobody has run.
///
/// # What a fixture buys
///
/// A checker nobody has watched fail is a checker nobody has tested. Each flaw
/// below is a real mistake — an index masked instead of bounds-checked, a
/// revoke that stops at the first generation, a derive that lets rights widen
/// — and each one must be caught by the property it breaks and by that property
/// alone.
pub mod properties {
    use super::{
        Backing, Condemned, Direct, FRAME_SIZE, MAX_SLOTS, Slot, TABLE_SLOTS, Table, no_such,
        revoked,
    };
    use crate::mem::{FrameAllocator, Order};
    use f_abi::cap::{CapType, Handle, rights};
    use f_abi::control::notice;
    use f_abi::error;

    /// A property of a capability table, and what it means when it fails.
    ///
    /// These are the five from `docs/design/ring-scene-boot.html` section 15,
    /// M4 — the phase-00 exit criterion written as code. E0-P08.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Property {
        /// A process resolved a handle to a slot its table was never given.
        Unnamed,
        /// A handle nobody issued resolved: a generation that was never handed
        /// out, or the zero word.
        Forged,
        /// A capability that had been revoked was still usable, or was refused
        /// without being named as revoked.
        Stale,
        /// Rights were widened by a derivation, or an operation was allowed a
        /// right its capability does not carry.
        Rights,
        /// An operation reached outside the table, or answered a hostile handle
        /// with something other than a refusal.
        Total,
    }

    impl Property {
        /// A sentence for the log.
        #[must_use]
        pub fn message(self) -> &'static str {
            match self {
                Self::Unnamed => "a handle resolved to a slot the table was never given",
                Self::Forged => "a handle nobody issued resolved",
                Self::Stale => "a revoked capability was still usable",
                Self::Rights => "a capability carried a right it was not granted",
                Self::Total => "a hostile handle produced something other than a refusal",
            }
        }

        /// All five, in the order [`check`] tests them.
        #[must_use]
        pub const fn all() -> [Self; 5] {
            [Self::Unnamed, Self::Forged, Self::Stale, Self::Rights, Self::Total]
        }
    }

    /// What [`check`] needs of a table.
    ///
    /// A trait rather than a concrete type for one reason: it is what lets the
    /// same five checks run against the real table and against a table broken
    /// on purpose. `f_env::contract` is the same argument.
    pub trait Authority {
        /// Forget everything, keeping generations, then put back the account
        /// this table charges its growth to — and, if this fixture is one of
        /// the bought ones, the page that account paid for.
        ///
        /// What ends a process, plus the two steps that make the *next* one
        /// look like the process a running table belongs to. A component always
        /// has an account: RFC 0008 spawns it with one, and a table with none
        /// could not grow at all, which would make the size half of these
        /// checks unreachable.
        fn reset(&mut self);
        /// The [`CapType::Untyped`] [`Authority::reset`] put in.
        ///
        /// Named so that the checks can tell it apart from what they seeded
        /// themselves. A property that treated it as forged would be asserting
        /// that a component cannot hold the account it was spawned with.
        fn account(&self) -> Handle;
        /// How many slots exist now — [`Table::capacity`].
        fn capacity(&self) -> usize;
        /// The frame putting a capability in.
        ///
        /// # Errors
        /// As [`Table::grant`].
        fn seed(
            &mut self,
            kind: CapType,
            rights: u8,
            object: u64,
            extent: u64,
        ) -> Result<Handle, i32>;
        /// What a handle names.
        ///
        /// # Errors
        /// As [`Table::inspect`].
        fn inspect(&self, handle: Handle) -> Result<super::Found, i32>;
        /// Mint a child.
        ///
        /// # Errors
        /// As [`Table::derive`].
        fn derive(&mut self, handle: Handle, asked: u8) -> Result<Handle, i32>;
        /// Withdraw everything below a capability.
        ///
        /// # Errors
        /// As [`Table::condemn`].
        fn revoke(&mut self, handle: Handle) -> Result<u32, i32>;
    }

    /// A real table, with the account it grows out of and the memory that
    /// account buys.
    ///
    /// The three are one fixture rather than three arguments because growth is
    /// the thing under test: a table handed to the checks without an account
    /// would pass every one of them at the size it started at, which is the
    /// size E1-B13 exists to stop being the only one.
    pub struct Sound<'a> {
        table: Table,
        ground: Direct<'a>,
        /// The frame the account hands out — a real one, so that the fixture
        /// grows through the same step the frame does rather than through a
        /// second path written for the test.
        frame: u64,
        /// Whether [`Authority::reset`] leaves this table already bought.
        bought: bool,
        account: Handle,
    }

    impl<'a> Sound<'a> {
        /// A table whose account names `frame`.
        ///
        /// # Safety
        ///
        /// As [`Direct::new`]: `frame` must be a frame `frames` gave out and
        /// nobody else holds, for as long as this fixture lives.
        #[must_use]
        pub const unsafe fn new(frames: &'a FrameAllocator, frame: u64, bought: bool) -> Self {
            // SAFETY: the caller's guarantee, which is the same sentence.
            let ground = unsafe { Direct::new(frames) };
            Self { table: Table::EMPTY, ground, frame, bought, account: Handle::NULL }
        }
    }

    impl Authority for Sound<'_> {
        fn reset(&mut self) {
            self.table.clear_all();
            self.account = restock(&mut self.table, &mut self.ground, self.frame, self.bought);
        }
        fn account(&self) -> Handle {
            self.account
        }
        fn capacity(&self) -> usize {
            self.table.capacity()
        }
        fn seed(
            &mut self,
            kind: CapType,
            rights: u8,
            object: u64,
            extent: u64,
        ) -> Result<Handle, i32> {
            self.table.grant(kind, rights, object, extent)
        }
        fn inspect(&self, handle: Handle) -> Result<super::Found, i32> {
            Table::inspect(&self.table, handle)
        }
        fn derive(&mut self, handle: Handle, asked: u8) -> Result<Handle, i32> {
            self.table.derive(handle, asked, &mut self.ground)
        }
        fn revoke(&mut self, handle: Handle) -> Result<u32, i32> {
            // The properties are about authority, not about address spaces, so
            // the mappings a revocation withdrew are dropped here rather than
            // checked — the drain step is skipped and the sweep is not. A
            // flawed table that got the mappings wrong would be caught by the
            // property it broke, every one of the five being stated in terms of
            // what a handle resolves to, and there is no fixture that could
            // hold a mapping, because these tables belong to no process.
            let condemned = self.table.condemn(handle)?;
            Ok(self.table.sweep(&condemned))
        }
    }

    /// Put the account back, and the page it bought if this fixture is one of
    /// the bought ones.
    ///
    /// Shared by both fixtures so that a bought table and a bought broken table
    /// are bought the same way. The account keeps [`rights::DERIVE`] because
    /// growing is retyping, and [`rights::REVOKE`] so that a check which
    /// revokes from it is exercising a capability a component would really
    /// hold.
    fn restock(table: &mut Table, ground: &mut dyn Backing, frame: u64, bought: bool) -> Handle {
        let held = rights::READ | rights::DERIVE | rights::REVOKE;
        let Ok(account) = table.grant(CapType::Untyped, held, frame, FRAME_SIZE) else {
            return Handle::NULL;
        };
        if bought {
            // A failure here leaves the table at its free size, and `total`
            // fails on the assertion that a table can be grown at all — which
            // is the right place for it to be noticed.
            let _ = table.grow(ground);
        }
        account
    }

    /// A mistake a capability table can make.
    ///
    /// Each one is a bug somebody has shipped in something, which is why these
    /// and not five arbitrary corruptions.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Flaw {
        /// An empty slot in the current generation resolves anyway. The table
        /// answers for authority nobody granted.
        AnswersForEmptySlots,
        /// The generation is not compared. Any handle to an occupied slot
        /// resolves, whoever made it up.
        IgnoresGeneration,
        /// Revocation stops at the direct children instead of walking the tree.
        RevokesOneLevel,
        /// A derivation may ask for rights its parent does not hold.
        LetsRightsWiden,
        /// The slot index is forced into range rather than checked. The classic
        /// one: correct-looking, constant-time, and it resolves handles that
        /// name nothing.
        ///
        /// It used to be spelled as a mask, because a table used to be a power
        /// of two. A table that has bought part of itself is not, and the same
        /// mistake written for it is a remainder — which is the more general
        /// form and the one worth having in the suite. A mask against the
        /// *fixed* part would be a second bug on top of this one: it aliases
        /// bought slots onto free ones, so it would be caught by
        /// [`Property::Forged`] with an in-range handle, and a fixture caught by
        /// the wrong check has not tested the check it was written for.
        ForcesIntoRange,
    }

    impl Flaw {
        /// Which property this flaw breaks. Asserted by [`self_test`], because
        /// a fixture caught by the wrong check has not tested that check.
        #[must_use]
        pub const fn breaks(self) -> Property {
            match self {
                Self::AnswersForEmptySlots => Property::Unnamed,
                Self::IgnoresGeneration => Property::Forged,
                Self::RevokesOneLevel => Property::Stale,
                Self::LetsRightsWiden => Property::Rights,
                Self::ForcesIntoRange => Property::Total,
            }
        }

        /// All five.
        #[must_use]
        pub const fn all() -> [Self; 5] {
            [
                Self::AnswersForEmptySlots,
                Self::IgnoresGeneration,
                Self::RevokesOneLevel,
                Self::LetsRightsWiden,
                Self::ForcesIntoRange,
            ]
        }
    }

    /// A real table with one thing wrong with it.
    ///
    /// It reuses [`Table`]'s storage and its safe pieces — `place`, `clear`,
    /// `sweep` — and re-implements only the step its flaw changes. A fixture
    /// that shared the flawed step with the real code would be testing a
    /// switch rather than the code.
    pub struct Flawed<'a> {
        table: Table,
        flaw: Flaw,
        ground: Direct<'a>,
        frame: u64,
        bought: bool,
        account: Handle,
    }

    impl<'a> Flawed<'a> {
        /// A broken table, with the same account and the same memory a sound
        /// one gets.
        ///
        /// # Safety
        ///
        /// As [`Sound::new`].
        #[must_use]
        pub const unsafe fn new(
            flaw: Flaw,
            frames: &'a FrameAllocator,
            frame: u64,
            bought: bool,
        ) -> Self {
            // SAFETY: the caller's guarantee, which is the same sentence.
            let ground = unsafe { Direct::new(frames) };
            Self { table: Table::EMPTY, flaw, ground, frame, bought, account: Handle::NULL }
        }

        /// The flawed lookup.
        ///
        /// Each arm differs from [`Table::resolve`] in exactly one step and
        /// shares the rest, which is what makes it a mutation rather than a
        /// second implementation. A fixture that reimplemented the lookup would
        /// drift from the real one and start being caught for reasons that have
        /// nothing to do with the flaw it is named after.
        fn locate(&self, handle: Handle) -> Result<usize, i32> {
            if !handle.is_issuable() {
                return Err(no_such());
            }
            let index = handle.index() as usize;
            match self.flaw {
                // The mistake, one step from how it is usually written: forcing
                // an index into range is branch-free and looks like a bounds
                // check. Against the table's own size, so that every index a
                // component could legitimately hold maps to itself and only a
                // handle that names nothing is coerced into naming something —
                // which is what makes this a fixture for
                // [`Property::Total`] and for nothing else.
                Flaw::ForcesIntoRange => {
                    self.table.resolve_at(index % self.table.capacity(), handle)
                }
                // Bounds and occupancy, and no generation. Every handle to a
                // live slot resolves, whoever made it up.
                Flaw::IgnoresGeneration => match self.table.at(index) {
                    Some(slot) if slot.occupied() => Ok(index),
                    _ => Err(no_such()),
                },
                // Bounds and generation, and no occupancy. A slot the table was
                // never given answers as though it had been.
                Flaw::AnswersForEmptySlots => match self.table.at(index) {
                    Some(slot) if slot.generation == handle.generation() => Ok(index),
                    Some(slot) if handle.generation() < slot.generation => Err(revoked()),
                    _ => Err(no_such()),
                },
                _ => self.table.resolve(handle),
            }
        }

        /// What the flawed lookup produces, with an empty slot answered as a
        /// capability rather than as a refusal.
        fn found(&self, index: usize) -> Result<super::Found, i32> {
            let slot = self.table.at(index).ok_or_else(no_such)?;
            let kind = slot.cap_kind().unwrap_or(CapType::Untyped);
            Ok(super::Found { kind, rights: slot.rights, object: slot.object, extent: slot.extent })
        }
    }

    impl Authority for Flawed<'_> {
        fn reset(&mut self) {
            self.table.clear_all();
            self.account = restock(&mut self.table, &mut self.ground, self.frame, self.bought);
        }

        fn account(&self) -> Handle {
            self.account
        }

        fn capacity(&self) -> usize {
            self.table.capacity()
        }

        fn seed(
            &mut self,
            kind: CapType,
            rights: u8,
            object: u64,
            extent: u64,
        ) -> Result<Handle, i32> {
            self.table.grant(kind, rights, object, extent)
        }

        fn inspect(&self, handle: Handle) -> Result<super::Found, i32> {
            let index = self.locate(handle)?;
            self.found(index)
        }

        fn derive(&mut self, handle: Handle, asked: u8) -> Result<Handle, i32> {
            if rights::unknown(asked) {
                return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
            }
            let index = self.locate(handle)?;
            let parent = self.found(index)?;
            if !rights::holds(parent.rights, rights::DERIVE) {
                return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
            }
            // The one step this flaw removes.
            if self.flaw != Flaw::LetsRightsWiden && !rights::narrows(parent.rights, asked) {
                return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
            }
            // Growth is shared with the real table on purpose, in the same
            // order and for the same reason. A fixture that could not grow
            // would stop at the free size and be caught by `total` for a reason
            // that has nothing to do with the flaw it is named after, which is
            // the failure `self_test` reports as a wrong property.
            if self.table.vacancy().is_none() {
                self.table.grow(&mut self.ground)?;
            }
            let (kind, object, extent) = self.table.retype(handle)?;
            self.table.place(kind, asked, object, extent, handle)
        }

        fn revoke(&mut self, handle: Handle) -> Result<u32, i32> {
            let index = self.locate(handle)?;
            let holder = self.found(index)?;
            if !rights::holds(holder.rights, rights::REVOKE) {
                return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
            }
            let doomed = match self.flaw {
                // Direct children only. The tree below them survives, which is
                // the bug that makes a revocation look like it worked.
                Flaw::RevokesOneLevel => {
                    let mut marks = Condemned::NONE;
                    for i in 0..self.table.capacity() {
                        let Some(slot) = self.table.at(i) else { continue };
                        if slot.occupied() && Handle::from_bits(slot.parent) == handle {
                            marks.mark(i);
                        }
                    }
                    marks
                }
                _ => self.table.descendants(handle),
            };
            Ok(self.table.sweep(&doomed))
        }
    }

    /// What went wrong when the suite checked itself.
    ///
    /// Three of the four are about the *fixtures* rather than about the table,
    /// and that is deliberate: a suite that stopped being able to fail would
    /// otherwise keep reporting that everything holds. The distinction between
    /// them matters when reading a failed boot — one says the table is wrong,
    /// and the others say the evidence that it is right has stopped being
    /// evidence.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    pub enum Failure {
        /// The real table failed a property.
        Real(Property),
        /// A table broken on purpose passed the whole suite.
        NotCaught(Flaw),
        /// A broken table was caught, by a property other than the one it
        /// breaks. The suite still fails: a fixture caught by the wrong check
        /// has not tested that check, and the check it was meant for is still
        /// unexercised.
        WrongProperty(Flaw, Property),
        /// The table could not carry one of the six types, or a derivation of
        /// one went wrong.
        Types(&'static str),
        /// The suite could not get the memory its fixtures grow into, so the
        /// half of it that is about size did not run. Reported rather than
        /// skipped: a suite that quietly shrinks is a suite that keeps saying
        /// everything holds.
        NoGround,
        /// A storage property did not hold. Carries the sentence, because
        /// there are five of them and they are about the bytes rather than
        /// about the authority model — see [`storage`].
        Storage(&'static str),
    }

    impl Failure {
        /// A sentence for the log.
        #[must_use]
        pub fn message(self) -> &'static str {
            match self {
                Self::Real(property) => property.message(),
                Self::NotCaught(_) => "a table broken on purpose passed the negative suite",
                Self::WrongProperty(..) => "a broken table was caught by the wrong property",
                Self::Types(why) | Self::Storage(why) => why,
                Self::NoGround => "there was no frame for the suite's tables to be grown into",
            }
        }

        /// Which flaw, when one is involved.
        #[must_use]
        pub const fn flaw(self) -> Option<Flaw> {
            match self {
                Self::NotCaught(flaw) | Self::WrongProperty(flaw, _) => Some(flaw),
                _ => None,
            }
        }

        /// Which property, when one is involved.
        #[must_use]
        pub const fn property(self) -> Option<Property> {
            match self {
                Self::Real(property) | Self::WrongProperty(_, property) => Some(property),
                _ => None,
            }
        }
    }

    /// A physical address to hang a capability on. Nothing dereferences these:
    /// the checks are about the table, not about memory.
    const OBJECT_A: u64 = 0x0000_0000_1000_0000;
    /// A second one, distinct so that a table answering for another's slot is
    /// visible in the object it reports rather than only in a return code.
    const OBJECT_B: u64 = 0x0000_0000_2000_0000;

    /// Run the five properties against a table, using a second table for the
    /// one property that is about two of them.
    ///
    /// Both are reset as it goes, so a caller may pass tables with anything in
    /// them.
    ///
    /// # Errors
    ///
    /// The first [`Property`] that does not hold.
    pub fn check(a: &mut dyn Authority, b: &mut dyn Authority) -> Result<(), Property> {
        unnamed(a, b)?;
        forged(a)?;
        stale(a)?;
        narrowing(a)?;
        total(a)?;
        Ok(())
    }

    /// A process cannot name a capability it was not given.
    ///
    /// Two tables, because the interesting form of this is not "an empty slot
    /// is refused" but "the same integer means different things in different
    /// components" — which is the whole reason a handle is an index into a
    /// per-process table rather than a system-wide identifier.
    fn unnamed(a: &mut dyn Authority, b: &mut dyn Authority) -> Result<(), Property> {
        a.reset();
        b.reset();

        let first =
            a.seed(CapType::Frame, rights::READ, OBJECT_A, 0).map_err(|_| Property::Unnamed)?;
        let second =
            a.seed(CapType::Frame, rights::READ, OBJECT_A, 0).map_err(|_| Property::Unnamed)?;
        let theirs =
            b.seed(CapType::Frame, rights::READ, OBJECT_B, 0).map_err(|_| Property::Unnamed)?;

        // B was given one capability, so A's second handle names a slot B never
        // filled — whatever the integer happens to be.
        if b.inspect(second).is_ok() {
            return Err(Property::Unnamed);
        }
        // And where the integers do coincide, the handle resolves in B to B's
        // capability. A handle is not a global name.
        if first != theirs {
            return Err(Property::Unnamed);
        }
        match b.inspect(first) {
            Ok(found) if found.object == OBJECT_B => Ok(()),
            _ => Err(Property::Unnamed),
        }
    }

    /// A process cannot forge a handle.
    ///
    /// Every in-range handle over a range of generations, against a table whose
    /// contents are known: exactly the issued ones resolve and nothing else
    /// does. The sweep is in-range on purpose — out-of-range is
    /// [`Property::Total`]'s question, so that a fixture that masks the index
    /// is caught by the check that is about masking.
    ///
    /// In range means *as far as this table has been paid for*, so on a bought
    /// table the sweep covers slots the component bought as well as the ones it
    /// was given. The second half is what growth added to this property, and it
    /// is the one E0-B10 would recognise: a slot that has been bought, used and
    /// given back must not answer to the handle it answered to last time.
    fn forged(a: &mut dyn Authority) -> Result<(), Property> {
        a.reset();
        let one =
            a.seed(CapType::Frame, rights::READ, OBJECT_A, 0).map_err(|_| Property::Forged)?;
        let two =
            a.seed(CapType::Untyped, rights::READ, OBJECT_B, 0).map_err(|_| Property::Forged)?;
        let account = a.account();

        for index in 0..a.capacity() {
            for generation in 0..8u16 {
                // `index` is below MAX_SLOTS, which is far below u16::MAX.
                let handle = Handle::new(index as u16, generation);
                let expected = handle == one || handle == two || handle == account;
                if a.inspect(handle).is_ok() != expected {
                    return Err(Property::Forged);
                }
            }
        }

        // Now a handle into a slot the table *bought*, held across the boundary
        // that ends a process. The page is dropped when the account that paid
        // for it goes, and the next process buys different memory for the same
        // indices — so a table whose bought slots started at the first
        // generation would hand the next occupant of this core a handle the
        // last one is still holding. That is the failure `clear_all` already
        // refuses to have in the free part, and growth is where it would have
        // come back.
        let root = a
            .seed(CapType::Frame, rights::READ | rights::DERIVE, OBJECT_A, 0)
            .map_err(|_| Property::Forged)?;
        let mut bought = None;
        while bought.is_none() {
            let minted = a.derive(root, rights::READ).map_err(|_| Property::Forged)?;
            if minted.index() as usize >= TABLE_SLOTS {
                bought = Some(minted);
            }
        }
        let Some(bought) = bought else { return Err(Property::Forged) };

        a.reset();
        // Only where the reset leaves the slot existing again is there anything
        // to say: a table that is not bought back has no slot at that index and
        // refuses for the ordinary reason, which is a different check.
        if a.capacity() > bought.index() as usize {
            match a.inspect(bought) {
                Err(code) if code == revoked() => {}
                _ => return Err(Property::Forged),
            }
        }
        Ok(())
    }

    /// A process cannot use a revoked handle.
    ///
    /// Three deep, because a revoke that stops at the children is the mistake
    /// that looks like it worked: the log says two capabilities were withdrawn
    /// and the grandchild is still holding the object.
    fn stale(a: &mut dyn Authority) -> Result<(), Property> {
        a.reset();
        let root = rights::READ | rights::DERIVE | rights::REVOKE;
        let parent = a.seed(CapType::Frame, root, OBJECT_A, 0).map_err(|_| Property::Stale)?;
        let child = a.derive(parent, root).map_err(|_| Property::Stale)?;
        let grandchild = a.derive(child, root).map_err(|_| Property::Stale)?;

        if a.revoke(parent) != Ok(2) {
            return Err(Property::Stale);
        }
        // Both gone, and both *named* as gone: a component recovers from a
        // revocation differently than from its own bug, so the two must not
        // arrive as the same code.
        for handle in [child, grandchild] {
            match a.inspect(handle) {
                Err(code) if code == revoked() => {}
                _ => return Err(Property::Stale),
            }
        }
        // The capability revoked *from* is not itself withdrawn.
        if a.inspect(parent).is_err() {
            return Err(Property::Stale);
        }
        Ok(())
    }

    /// A process cannot exceed granted rights.
    fn narrowing(a: &mut dyn Authority) -> Result<(), Property> {
        a.reset();
        let held = rights::READ | rights::WRITE | rights::DERIVE;
        let parent = a.seed(CapType::Frame, held, OBJECT_A, 0).map_err(|_| Property::Rights)?;

        // The direct attempt: ask for a right the parent does not carry.
        if a.derive(parent, held | rights::REVOKE).is_ok() {
            return Err(Property::Rights);
        }
        // The indirect one: drop a right, then try to recover it from the
        // child. The child keeps `DERIVE`, so this is refused for widening and
        // not for lacking the operation — which is the distinction a table that
        // checks against the *original* grant rather than the immediate parent
        // gets wrong.
        let narrowed = rights::READ | rights::DERIVE;
        let child = a.derive(parent, narrowed).map_err(|_| Property::Rights)?;
        if a.derive(child, narrowed | rights::WRITE).is_ok() {
            return Err(Property::Rights);
        }
        // Narrowing itself must still work, or the checks above pass for the
        // wrong reason.
        match a.inspect(child) {
            Ok(found) if found.rights == narrowed => {}
            _ => return Err(Property::Rights),
        }
        if a.derive(child, rights::READ).is_err() {
            return Err(Property::Rights);
        }
        // A capability with no derive right cannot mint at all, even a copy.
        let sealed =
            a.seed(CapType::Frame, rights::READ, OBJECT_A, 0).map_err(|_| Property::Rights)?;
        if a.derive(sealed, rights::READ).is_ok() {
            return Err(Property::Rights);
        }
        // And one with no revoke right cannot withdraw.
        if a.revoke(sealed).is_ok() {
            return Err(Property::Rights);
        }
        Ok(())
    }

    /// A process cannot make the kernel panic by trying.
    ///
    /// What that reduces to for a table is totality: every handle a process can
    /// write into a register is answered, and none of them reaches a slot that
    /// is not in the table. The static half of the same property is the
    /// `deny(clippy::indexing_slicing)` at the top of this file — an index that
    /// is masked rather than checked is the construct that turns a hostile
    /// handle into a fault, and it cannot be written here.
    fn total(a: &mut dyn Authority) -> Result<(), Property> {
        a.reset();
        let held = rights::READ | rights::DERIVE | rights::REVOKE;
        let live = a.seed(CapType::Frame, held, OBJECT_A, 0).map_err(|_| Property::Total)?;

        let hostile = [
            Handle::NULL,
            Handle::new(0, 0),
            // The end of the free part, which on a table that has bought a page
            // is an ordinary empty slot and on one that has not is nothing at
            // all. Both must refuse, and the two are the same handle — which is
            // the point: what a table answers must depend on what it holds and
            // not on a constant somebody wrote down.
            Handle::new(TABLE_SLOTS as u16, live.generation()),
            // Just past what this table has paid for, and past it by exactly
            // the free size — which is the value a mask sends back to a slot
            // that is occupied.
            Handle::new(a.capacity() as u16, live.generation()),
            Handle::new(a.capacity() as u16 + 1, live.generation()),
            // Past the ceiling this build will ever grow a table to, which no
            // amount of paying could bring into range.
            Handle::new(MAX_SLOTS as u16, live.generation()),
            Handle::new(u16::MAX, live.generation()),
            Handle::new(u16::MAX, u16::MAX),
            Handle::from_bits(u32::MAX),
        ];

        for handle in hostile {
            if a.inspect(handle).is_ok() {
                return Err(Property::Total);
            }
            if a.derive(handle, rights::NONE).is_ok() {
                return Err(Property::Total);
            }
            if a.revoke(handle).is_ok() {
                return Err(Property::Total);
            }
        }

        // A rights bit this build does not define is refused rather than
        // ignored: R04, and the same rule a reserved header field follows.
        if a.derive(live, rights::ALL | 1 << 7).is_ok() {
            return Err(Property::Total);
        }

        // Filling the table is an error and not a fault. A process that derives
        // in a loop is the cheapest denial of service there is.
        //
        // Since E1-B13 it is also where the quota becomes visible: the loop
        // fills the free part, the table buys one page out of the account
        // `reset` left in it, the loop fills that too, and then there is
        // nothing left to buy the next page with. Both endings are the same
        // refusal, and the count is what says growth happened — a table that
        // had quietly stopped growing would refuse in the right way at the
        // wrong size.
        let mut minted = 0u32;
        loop {
            match a.derive(live, held) {
                Ok(_) => minted += 1,
                Err(code) => {
                    let expected = error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED);
                    if code != expected {
                        return Err(Property::Total);
                    }
                    break;
                }
            }
            if minted as usize > MAX_SLOTS {
                // It never refused. There is no bound on the table.
                return Err(Property::Total);
            }
        }
        // Two slots were spoken for before the loop: the account and `live`.
        if minted as usize != a.capacity() - 2 {
            return Err(Property::Total);
        }
        // And a table that never grew would satisfy the line above while
        // holding the fixed count, which is the thing this task exists to stop
        // being the only size a table can be.
        if a.capacity() <= TABLE_SLOTS {
            return Err(Property::Total);
        }
        // The account is empty now, so the refusal above was *cannot pay* and
        // not *nothing asked*. Asking again must earn the same refusal rather
        // than a fault or a slot from somewhere else.
        match a.derive(live, held) {
            Err(code) if code == error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED) => {
                Ok(())
            }
            _ => Err(Property::Total),
        }
    }

    /// How many storage properties [`storage`] checks.
    ///
    /// Reported by the boot log beside the five above, so that a suite that
    /// quietly stopped running half of itself is visible rather than silent.
    pub const STORAGE_CHECKS: usize = 5;

    /// What a table's *storage* must do, as against what its authority model
    /// must.
    ///
    /// The five properties above are about who may name what, and they run
    /// through [`Authority`] so that a broken table can be substituted for a
    /// sound one. Nothing E1-B05 added to this file is reachable through that
    /// trait: a notice packed into the spare bits of a type byte, a watermark
    /// that moves *back*, and a name given up along with everything below it
    /// are all operations the frame performs on its own behalf and no component
    /// can ask for. So they are checked here instead, in the one part of this
    /// file that runs on every boot.
    ///
    /// Five, and each is a mistake somebody could make in one line: a mask
    /// written as a comparison, a notice overwritten by a type, a refund that
    /// walks past the floor, a relinquish that spares what it should take, and
    /// a slot refilled while it still owes a notice. The last is the one a
    /// component's quota depends on, and RFC 0008 is emphatic about it: a
    /// *revoked* notice always names a handle whose slot has not been reissued.
    ///
    /// # Errors
    ///
    /// [`Failure::Storage`], carrying the sentence that did not hold.
    fn storage(frames: &FrameAllocator) -> Result<(), Failure> {
        let mut table = Table::EMPTY;
        // SAFETY: `frames` is rebound onto the direct map that is live for the
        // whole boot, and this table is never grown — everything below fits in
        // the free part — so the backing exists to satisfy `derive` and is
        // never asked for a page.
        let mut ground = unsafe { Direct::new(frames) };
        table.owes_notices();

        // ---- 1. The type and the notice share one byte and leave each other
        //         alone. The packing is what keeps a slot thirty-two bytes and
        //         a bought page a hundred and twenty-eight slots, and the cost
        //         of it is exactly this: two writers of one byte.
        let held = rights::ALL & !rights::EXECUTE;
        let root = table
            .grant(CapType::Untyped, held, OBJECT_A, FRAME_SIZE * 4)
            .map_err(|_| Failure::Storage("a table could not hold a storage check's account"))?;
        if table.owes() != 1 {
            return Err(Failure::Storage("a grant into a table that owes notices owed none"));
        }
        match table.next_slot_notice(0) {
            Some(entry)
                if entry.result == notice::GRANTED && entry.user_data == u64::from(root.bits()) => {
            }
            _ => {
                return Err(Failure::Storage(
                    "a granted notice did not name the handle it was for",
                ));
            }
        }
        match table.inspect(root) {
            Ok(found) if found.kind == CapType::Untyped && found.rights == held => {}
            _ => return Err(Failure::Storage("a notice in a slot's type byte changed its type")),
        }

        // ---- 2. A watermark that goes back exactly as far as it came, and
        //         refuses to go further. `Table::refund` is the one operation
        //         in this file that *unspends*, and it is sound only because
        //         nothing has been retyped since.
        let before = table
            .inspect(root)
            .map_err(|_| Failure::Storage("an account stopped answering for itself"))?;
        let child = table
            .derive(
                root,
                rights::READ | rights::WRITE | rights::DERIVE | rights::REVOKE,
                &mut ground,
            )
            .map_err(|_| Failure::Storage("an account could not be retyped from"))?;
        let spent = table
            .inspect(root)
            .map_err(|_| Failure::Storage("an account stopped answering after a retype"))?;
        if spent.object != before.object + FRAME_SIZE || spent.extent != before.extent - FRAME_SIZE
        {
            return Err(Failure::Storage("retyping moved an account by the wrong amount"));
        }
        let past = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        if table.refund(root, FRAME_SIZE * 2, before.object) != Err(past) {
            return Err(Failure::Storage("a refund past an account's floor was not refused"));
        }
        table
            .refund(root, FRAME_SIZE, before.object)
            .map_err(|_| Failure::Storage("a legal refund was refused"))?;
        let back = table
            .inspect(root)
            .map_err(|_| Failure::Storage("an account stopped answering after a refund"))?;
        if back.object != before.object || back.extent != before.extent {
            return Err(Failure::Storage("a refund did not restore what the retype took"));
        }

        // ---- 3. Giving up a name gives up everything below it, where revoking
        //         spares the capability it is given. Two different questions
        //         with two different answers, and a file that answered them the
        //         same way would leave a supervisor holding a name over memory
        //         the next instance is being handed.
        let grandchild = table
            .derive(child, rights::READ, &mut ground)
            .map_err(|_| Failure::Storage("a frame could not be derived from"))?;
        let condemned =
            table.condemn(child).map_err(|_| Failure::Storage("a revoke was refused"))?;
        if table.sweep(&condemned) != 1 || table.inspect(child).is_err() {
            return Err(Failure::Storage("a revoke did not spare the capability it was given"));
        }
        if table.inspect(grandchild).is_ok() {
            return Err(Failure::Storage("a revoke left a descendant standing"));
        }
        let again = table
            .derive(child, rights::READ, &mut ground)
            .map_err(|_| Failure::Storage("a frame could not be derived from twice"))?;
        // Every notice owed so far is published before the relinquish, because
        // rule 1 says an undelivered *grant* that is revoked posts nothing — so
        // a check that skipped this would be checking rule 1 and calling it
        // rule 2.
        while table.next_slot_notice(0).is_some() {}
        let index = child.index() as usize;
        if table.relinquish(child).map_err(|_| Failure::Storage("a relinquish was refused"))? != 2 {
            return Err(Failure::Storage(
                "a relinquish did not take the capability and its descendant",
            ));
        }
        if table.inspect(child).is_ok() || table.inspect(again).is_ok() {
            return Err(Failure::Storage("a relinquish left a name standing"));
        }

        // ---- 4. A slot that is empty and still owes a *revoked* notice is a
        //         real state: it holds nothing, and it is not refilled. The
        //         second half is what keeps a handle's generation honest under
        //         a pending notice, and it is the rule a component's quota
        //         depends on — a component that never drains runs out of table
        //         before it runs out of memory, which is the failure we want.
        if table.owes() != 2 {
            return Err(Failure::Storage("giving up a capability owed no revoked notice"));
        }
        if table.at(index).is_some_and(Slot::occupied) {
            return Err(Failure::Storage("a slot owing a revoked notice still held a capability"));
        }
        if table.vacancy() == Some(index) {
            return Err(Failure::Storage("a slot owing a notice was offered to the next grant"));
        }
        match table.next_slot_notice(0) {
            Some(entry) if entry.result == notice::REVOKED => {}
            _ => return Err(Failure::Storage("a revoked notice was not published in slot order")),
        }

        // ---- 5. The words a table holds beside its slots: a promise that may
        //         only ever move earlier, and grades where the latest wins. R08
        //         is why the first is not simply a field somebody assigns.
        if !table.stop_by(100) || table.stop_by(200) || !table.stop_by(50) {
            return Err(Failure::Storage("a stop deadline moved the wrong way"));
        }
        if table.stop_deadline() != Some(50) {
            return Err(Failure::Storage("a stop kept a deadline it was not promised"));
        }
        table.pressure_is(1);
        if !table.pressure_is(2) {
            return Err(Failure::Storage("a grade did not take the later value"));
        }
        match table.next_stop_notice(root, 0) {
            Some(entry) if entry.result == notice::STOP && entry.ext == 50 => {}
            _ => return Err(Failure::Storage("a stop notice did not carry the deadline it kept")),
        }
        match table.next_grade_notice(0) {
            Some(entry) if entry.result == notice::PRESSURE && entry.ext == 2 => {}
            _ => return Err(Failure::Storage("a pressure notice did not carry the latest grade")),
        }

        // Nothing is owed to a table that is about to stop existing, and
        // nothing was bought, so there is no page to give back here.
        table.clear_all();
        if table.owes() != 0 {
            return Err(Failure::Storage("a table that ended still owed a notice"));
        }
        Ok(())
    }

    /// Check the properties against a real table, and check that the checks can
    /// fail.
    ///
    /// Returns how many flawed tables were caught, which is the number the boot
    /// log prints — a suite that silently stopped constructing its fixtures
    /// would otherwise still say `ok`.
    ///
    /// # Why this needs the frame allocator
    ///
    /// Because a table that has been paid for is a different table, and the
    /// only honest way to have one is to pay. Two frames go in — one per
    /// fixture, since two tables are live at a time — and every fixture's
    /// account names a real frame, so the suite grows its tables through
    /// exactly the step a running process's derive takes. A fixture with a
    /// pretend account would have been testing a second path.
    ///
    /// Both frames come back before this returns. A suite that leaked would be
    /// caught by the allocator's own free count at the end of the boot, which
    /// is a worse place to find out.
    ///
    /// # Errors
    ///
    /// A sentence naming what did not hold.
    pub fn self_test(frames: &mut FrameAllocator) -> Result<usize, Failure> {
        let first = frames.alloc_zeroed(Order::FRAME).ok_or(Failure::NoGround)?;
        let Some(second) = frames.alloc_zeroed(Order::FRAME) else {
            // SAFETY: `first` came from this allocator two lines ago and
            // nothing has been handed a reference to it.
            unsafe { frames.free(first) };
            return Err(Failure::NoGround);
        };

        let outcome = run(frames, first.addr(), second.addr());

        // SAFETY: both frames came from this allocator, every fixture that
        // could still be naming one has been dropped by `run` returning, and
        // neither has been freed before.
        unsafe { frames.free(first) };
        // SAFETY: as above.
        unsafe { frames.free(second) };
        outcome
    }

    /// The whole suite, at both sizes, with the memory already in hand.
    ///
    /// Split out so that the frames are freed on every path out of
    /// [`self_test`] rather than on the ones somebody remembered.
    fn run(frames: &FrameAllocator, first: u64, second: u64) -> Result<usize, Failure> {
        // Twice, and the second time is the point of E1-B13: once on tables
        // that hold only what the frame gave them, and once on tables that have
        // bought a page out of their own account. The five properties are about
        // a table and not about a size, and a suite that only ever ran at the
        // fixed count would have been the evidence that the fixed count was
        // the design.
        for bought in [false, true] {
            // SAFETY: `first` and `second` are frames `self_test` took from
            // this allocator and has not freed, and nothing else holds them for
            // as long as these fixtures live.
            let mut real = unsafe { Sound::new(frames, first, bought) };
            // SAFETY: as above, and a different frame — two tables are live at
            // once and each buys its own page.
            let mut other = unsafe { Sound::new(frames, second, bought) };
            if let Err(property) = check(&mut real, &mut other) {
                return Err(Failure::Real(property));
            }
        }

        // Every type in the table, exercised once. Three of the six have no
        // object behind them until M5 and E1, so this is the only place they
        // are held at all — and a type the table cannot carry would otherwise
        // be discovered by the milestone that first needs it.
        let mut types = Table::EMPTY;
        // SAFETY: as above. This table is never grown — six capabilities fit in
        // the free part — so the backing is here to satisfy `derive` and is
        // never asked for a page.
        let mut ground = unsafe { Direct::new(frames) };
        for (index, kind) in [
            CapType::Untyped,
            CapType::Frame,
            CapType::AddressSpace,
            CapType::Channel,
            CapType::Endpoint,
            CapType::Irq,
        ]
        .into_iter()
        .enumerate()
        {
            let object = OBJECT_A + (index as u64) * FRAME_SIZE;
            let held = rights::READ | rights::DERIVE | rights::REVOKE;
            let handle = types
                .grant(kind, held, object, FRAME_SIZE)
                .map_err(|_| Failure::Types("a table could not hold one of the six types"))?;
            match types.inspect(handle) {
                Ok(found) if found.kind == kind && found.object == object => {}
                _ => {
                    return Err(Failure::Types(
                        "a capability came back as a different type than it went in",
                    ));
                }
            }
            let child = types
                .derive(handle, rights::READ, &mut ground)
                .map_err(|_| Failure::Types("a type could not derive"))?;
            // Untyped is the one type whose child is a different type: that is
            // what retyping is, and it is the reason untyped is in the list.
            let expected = if kind == CapType::Untyped { CapType::Frame } else { kind };
            match types.inspect(child) {
                Ok(found) if found.kind == expected => {}
                _ => return Err(Failure::Types("a derivation produced the wrong type")),
            }
            let condemned = types
                .condemn(handle)
                .map_err(|_| Failure::Types("a capability could not be revoked from"))?;
            if types.sweep(&condemned) != 1 {
                return Err(Failure::Types(
                    "revoking a capability did not withdraw the one derived from it",
                ));
            }
        }

        // What the five above cannot see, because none of it is reachable
        // through `Authority`. Before the flawed fixtures, so that a failure
        // here reads as *the table is wrong* rather than as *a fixture is*.
        storage(frames)?;

        let mut caught = 0;
        for bought in [false, true] {
            for flaw in Flaw::all() {
                // SAFETY: as the fixtures above, and the sound ones are gone.
                let mut broken = unsafe { Flawed::new(flaw, frames, first, bought) };
                // SAFETY: as above, on the second frame.
                let mut other = unsafe { Flawed::new(flaw, frames, second, bought) };
                match check(&mut broken, &mut other) {
                    Ok(()) => return Err(Failure::NotCaught(flaw)),
                    Err(property) if property == flaw.breaks() => caught += 1,
                    Err(property) => return Err(Failure::WrongProperty(flaw, property)),
                }
            }
        }
        Ok(caught)
    }
}
