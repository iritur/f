// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The capability table: what a process may name, and the tree that lets it be
//! taken back.
//!
//! # What this is
//!
//! One table per process. [`TABLE_SLOTS`] typed slots, each holding an object,
//! a rights bitmap and the handle of the capability it was derived from. Three
//! operations on it — derive, revoke, and the lookup every use of a capability
//! begins with — plus [`Table::grant`], which is the frame putting something in
//! and is not reachable from ring 3 at all.
//!
//! The wire half is [`f_abi::cap`]: the handle packing, the six types and the
//! rights bits. Nothing about *storage* is over there, and nothing about the
//! *format* is over here.
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
//! [`TABLE_SLOTS`]² in the worst case and has no recursion in it. A recursive
//! revoke in a kernel is a stack depth controlled by whoever built the tree,
//! and this kernel's stacks have a guard page precisely because that class of
//! bug is real. Bounded iteration over a fixed array cannot have it.
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
//! every slot is reached through `get`/`get_mut`. The dynamic half is
//! [`properties::check`] and the seven `cap=` boots.
//!
//! See `docs/design/ring-scene-boot.html` section 15 milestone M4,
//! `docs/rfc/0015-capabilities-at-the-door.md`, and E0-B11.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unreachable
)]

use f_abi::cap::{CapType, Handle, rights};
use f_abi::error;

use crate::mem::FRAME_SIZE;
use crate::percpu::PerCpu;

/// How many capabilities one process may hold.
///
/// Thirty-two, and the number is a bound rather than a design. It is small
/// enough that revocation's quadratic walk is a thousand iterations of nothing,
/// and small enough to sit in a `PerCpu` static — which is where a process's
/// table has to live while there is no allocator a process may draw on.
///
/// *Reversal:* a component that legitimately holds more than this, which is
/// E1's first real supervisor. At that point the table stops being a fixed
/// array and becomes an object the [`CapType::Untyped`] capability pays for,
/// which is the same change as giving a process a quota.
pub const TABLE_SLOTS: usize = 32;

// Revocation marks its victims in a bitmask, one bit per slot. The mask is the
// reason the bound above is not free to move without a look at this file.
const _: () = assert!(TABLE_SLOTS <= u32::BITS as usize);

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
    /// A [`CapType`] wire value, or zero for empty.
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
    const EMPTY: Self = Self {
        kind: 0,
        rights: rights::NONE,
        generation: Handle::FIRST_GENERATION,
        parent: Handle::NULL.bits(),
        object: 0,
        extent: 0,
        mapped: NOT_MAPPED,
    };

    /// Is anything here now?
    const fn occupied(self) -> bool {
        self.kind != 0
    }
}

/// What a revocation withdrew.
///
/// Two answers, because they are two different facts about the same event: how
/// many capabilities stopped existing, which is what a process is told, and
/// which mappings those capabilities had authorised, which is what the frame
/// still has work to do about.
#[derive(Clone, Copy)]
pub struct Revoked {
    /// How many capabilities were cleared.
    pub cleared: u32,
    pages: [u64; TABLE_SLOTS],
    count: usize,
}

impl Revoked {
    /// The addresses whose mappings the withdrawn capabilities authorised.
    ///
    /// Bounded by [`TABLE_SLOTS`] because a mapping is recorded in a slot and
    /// there are only so many slots — so this cannot be a list that outgrows
    /// the array it is in, which is the failure mode a revocation sweep must
    /// not have.
    #[must_use]
    pub fn pages(&self) -> &[u64] {
        self.pages.get(..self.count).unwrap_or(&[])
    }
}

/// One process's capabilities.
///
/// `Copy` because [`PerCpu`] needs a `const` initialiser, and never actually
/// copied: every access goes through a raw pointer to this core's slot. A
/// table copied by value would be a second authority that can drift from the
/// first.
#[derive(Clone, Copy)]
pub struct Table {
    slots: [Slot; TABLE_SLOTS],
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
    /// A table holding nothing.
    pub const EMPTY: Self = Self { slots: [Slot::EMPTY; TABLE_SLOTS] };

    /// Forget everything. Called when a process ends.
    ///
    /// Generations are *not* reset: a table that started every process at
    /// generation one would let a handle from the last process resolve in the
    /// next one, which is the whole failure the generation exists to prevent,
    /// reintroduced at the one boundary that matters most.
    pub fn clear_all(&mut self) {
        for index in 0..TABLE_SLOTS {
            if self.slots.get(index).is_some_and(|slot| slot.occupied()) {
                self.clear(index);
            }
        }
    }

    /// How many capabilities are held.
    #[must_use]
    pub fn used(&self) -> usize {
        self.slots.iter().filter(|slot| slot.occupied()).count()
    }

    /// How many slots have been used up and may never be filled again.
    ///
    /// Reported so that the honest cost of not wrapping the generation is
    /// visible rather than theoretical. Zero for the life of any process this
    /// kernel currently runs.
    #[must_use]
    pub fn retired(&self) -> usize {
        self.slots
            .iter()
            .filter(|slot| !slot.occupied() && slot.generation == Handle::RETIRED_GENERATION)
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
    /// [`error::RESOURCE`] when there is no free slot.
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
        let kind = CapType::from_wire(slot.kind).ok_or(no_such())?;
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
    /// define, and [`error::RESOURCE`] when the table is full or an untyped
    /// region is exhausted.
    pub fn derive(&mut self, handle: Handle, asked: u8) -> Result<Handle, i32> {
        if rights::unknown(asked) {
            return Err(error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG));
        }
        let parent = self.invoke_any(handle, rights::DERIVE)?;
        if !rights::narrows(parent.rights, asked) {
            return Err(error::pack(error::AUTHORITY, error::authority::RIGHT_NOT_HELD));
        }
        let (kind, object, extent) = self.retype(handle)?;
        self.place(kind, asked, object, extent, handle)
    }

    /// Clear everything derived from a capability, however deep, and say which
    /// mappings went with it.
    ///
    /// The capability itself survives: revoke withdraws what was handed on, and
    /// a holder that wants to give up its own authority is asking a different
    /// question.
    ///
    /// # Why the mappings come back rather than being undone here
    ///
    /// Because undoing one is a page table edit followed by an interrupt to
    /// every other core, and this file knows about neither. It knows which
    /// authority has been withdrawn, which is the question it is for; the
    /// caller knows which address space and which cores. `process::withdraw` is
    /// where the two meet.
    ///
    /// # Errors
    ///
    /// As [`Table::inspect`], plus [`error::authority::RIGHT_NOT_HELD`] when
    /// the capability does not carry [`rights::REVOKE`].
    pub fn revoke(&mut self, handle: Handle) -> Result<Revoked, i32> {
        self.invoke_any(handle, rights::REVOKE)?;
        let doomed = self.descendants(handle);

        let mut withdrawn = Revoked { cleared: 0, pages: [0; TABLE_SLOTS], count: 0 };
        for index in 0..TABLE_SLOTS {
            if doomed & bit(index) == 0 {
                continue;
            }
            if let Some(slot) = self.slots.get(index)
                && slot.mapped != NOT_MAPPED
                && let Some(at) = withdrawn.pages.get_mut(withdrawn.count)
            {
                *at = slot.mapped;
                withdrawn.count += 1;
            }
        }

        withdrawn.cleared = self.sweep(doomed);
        Ok(withdrawn)
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
        let slot = self.slots.get_mut(index).ok_or(no_such())?;
        if slot.mapped != NOT_MAPPED {
            return Err(error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS));
        }
        slot.mapped = virt;
        Ok(())
    }

    // ---- the parts the flawed fixtures in `properties` also build from -----

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
        // Checked, never masked. A mask is the bug this returns an error for.
        //
        // Absent under `mutate-unchecked-index`, which is the deliberate defect
        // property five's mutation harness builds. See [`Table::slot`].
        #[cfg(not(feature = "mutate-unchecked-index"))]
        if index >= TABLE_SLOTS {
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
        self.slots.get(index).copied().ok_or(no_such())
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
        for index in 0..TABLE_SLOTS {
            let Some(slot) = self.slots.get_mut(index) else { continue };
            // Equality rather than "at least", because `clear` saturates: a
            // generation cannot go past the retirement value, so the two are
            // the same test and clippy is right that the wider one hides that.
            if slot.occupied() || slot.generation == Handle::RETIRED_GENERATION {
                continue;
            }
            slot.kind = kind.to_wire();
            slot.rights = rights;
            slot.object = object;
            slot.extent = extent;
            slot.parent = parent.bits();
            slot.mapped = NOT_MAPPED;
            let generation = slot.generation;
            // `index` is below TABLE_SLOTS, which is far below u16::MAX.
            return Ok(Handle::new(index as u16, generation));
        }
        Err(error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED))
    }

    /// What a derivation of the capability at `handle` produces, advancing an
    /// untyped region's watermark if that is what it is.
    fn retype(&mut self, handle: Handle) -> Result<(CapType, u64, u64), i32> {
        let index = self.resolve(handle)?;
        let slot = self.slots.get_mut(index).ok_or(no_such())?;
        let kind = CapType::from_wire(slot.kind).ok_or(no_such())?;
        if kind != CapType::Untyped {
            return Ok((kind, slot.object, slot.extent));
        }
        if slot.extent < FRAME_SIZE {
            return Err(error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED));
        }
        let object = slot.object;
        // Checked immediately above, so neither of these can wrap.
        slot.object = object.wrapping_add(FRAME_SIZE);
        slot.extent = slot.extent.wrapping_sub(FRAME_SIZE);
        Ok((CapType::Frame, object, FRAME_SIZE))
    }

    /// Every slot below `handle` in the derivation tree, as a bitmask.
    ///
    /// Iterative and bounded: each pass marks at least one slot or is the last,
    /// so it runs at most [`TABLE_SLOTS`] times. See the module comment on why
    /// this is not the obvious recursion.
    fn descendants(&self, handle: Handle) -> u32 {
        let mut doomed: u32 = 0;
        loop {
            let mut found = false;
            for index in 0..TABLE_SLOTS {
                if doomed & bit(index) != 0 {
                    continue;
                }
                let Some(slot) = self.slots.get(index) else { continue };
                if !slot.occupied() {
                    continue;
                }
                let parent = Handle::from_bits(slot.parent);
                if !parent.is_issuable() {
                    continue;
                }
                if parent == handle || self.marked_parent(parent, doomed) {
                    doomed |= bit(index);
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
    fn marked_parent(&self, parent: Handle, doomed: u32) -> bool {
        let index = parent.index() as usize;
        if doomed & bit(index) == 0 {
            return false;
        }
        self.slots.get(index).is_some_and(|slot| slot.generation == parent.generation())
    }

    /// Clear every marked slot, and say how many that was.
    fn sweep(&mut self, doomed: u32) -> u32 {
        let mut count = 0;
        for index in 0..TABLE_SLOTS {
            if doomed & bit(index) != 0 {
                self.clear(index);
                count += 1;
            }
        }
        count
    }

    /// Empty one slot and move it on to its next generation.
    ///
    /// Saturating, and the saturation is the point: a generation that wrapped
    /// would make a handle held since before the wrap valid again. A slot that
    /// runs out is retired rather than reused, which turns a hole in the
    /// authority model into a table that is one slot smaller.
    fn clear(&mut self, index: usize) {
        let Some(slot) = self.slots.get_mut(index) else { return };
        slot.kind = 0;
        slot.rights = rights::NONE;
        slot.object = 0;
        slot.extent = 0;
        slot.parent = Handle::NULL.bits();
        slot.mapped = NOT_MAPPED;
        slot.generation = slot.generation.saturating_add(1);
    }
}

/// One bit per slot, and zero for an index that is not one.
const fn bit(index: usize) -> u32 {
    if index < TABLE_SLOTS { 1u32 << index } else { 0 }
}

/// The handle names nothing this table ever held.
fn no_such() -> i32 {
    error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP)
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
    use super::{FRAME_SIZE, TABLE_SLOTS, Table, bit, no_such};
    use f_abi::cap::{CapType, Handle, rights};
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
        /// Forget everything, keeping generations. What ends a process.
        fn reset(&mut self);
        /// The frame putting a capability in.
        ///
        /// # Errors
        /// As [`Table::grant`].
        fn seed(&mut self, kind: CapType, rights: u8, object: u64) -> Result<Handle, i32>;
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
        /// As [`Table::revoke`].
        fn revoke(&mut self, handle: Handle) -> Result<u32, i32>;
    }

    impl Authority for Table {
        fn reset(&mut self) {
            self.clear_all();
        }
        fn seed(&mut self, kind: CapType, rights: u8, object: u64) -> Result<Handle, i32> {
            self.grant(kind, rights, object, 0)
        }
        fn inspect(&self, handle: Handle) -> Result<super::Found, i32> {
            Table::inspect(self, handle)
        }
        fn derive(&mut self, handle: Handle, asked: u8) -> Result<Handle, i32> {
            Table::derive(self, handle, asked)
        }
        fn revoke(&mut self, handle: Handle) -> Result<u32, i32> {
            // The properties are about authority, not about address spaces, so
            // the mappings a revocation withdrew are dropped here rather than
            // checked. A flawed table that got them wrong would be caught by
            // the property it broke — every one of the five is stated in terms
            // of what a handle resolves to — and there is no fixture that could
            // hold a mapping, because these tables belong to no process.
            Table::revoke(self, handle).map(|revoked| revoked.cleared)
        }
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
        /// The slot index is masked into range rather than checked. The classic
        /// one: correct-looking, constant-time, and it resolves handles that
        /// name nothing.
        MasksTheIndex,
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
                Self::MasksTheIndex => Property::Total,
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
                Self::MasksTheIndex,
            ]
        }
    }

    /// A real table with one thing wrong with it.
    ///
    /// It reuses [`Table`]'s storage and its safe pieces — `place`, `clear`,
    /// `sweep` — and re-implements only the step its flaw changes. A fixture
    /// that shared the flawed step with the real code would be testing a
    /// switch rather than the code.
    pub struct Flawed {
        table: Table,
        flaw: Flaw,
    }

    impl Flawed {
        /// A broken table.
        #[must_use]
        pub const fn new(flaw: Flaw) -> Self {
            Self { table: Table::EMPTY, flaw }
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
                // The mistake, exactly as it is usually written: a mask is
                // branch-free and looks like a bounds check.
                Flaw::MasksTheIndex => self.table.resolve_at(index & (TABLE_SLOTS - 1), handle),
                // Bounds and occupancy, and no generation. Every handle to a
                // live slot resolves, whoever made it up.
                Flaw::IgnoresGeneration => {
                    if index >= TABLE_SLOTS {
                        return Err(no_such());
                    }
                    match self.table.slots.get(index) {
                        Some(slot) if slot.occupied() => Ok(index),
                        _ => Err(no_such()),
                    }
                }
                // Bounds and generation, and no occupancy. A slot the table was
                // never given answers as though it had been.
                Flaw::AnswersForEmptySlots => {
                    if index >= TABLE_SLOTS {
                        return Err(no_such());
                    }
                    match self.table.slots.get(index) {
                        Some(slot) if slot.generation == handle.generation() => Ok(index),
                        Some(slot) if handle.generation() < slot.generation => {
                            Err(super::revoked())
                        }
                        _ => Err(no_such()),
                    }
                }
                _ => self.table.resolve(handle),
            }
        }

        /// What the flawed lookup produces, with an empty slot answered as a
        /// capability rather than as a refusal.
        fn found(&self, index: usize) -> Result<super::Found, i32> {
            let slot = self.table.slots.get(index).copied().ok_or(no_such())?;
            let kind = CapType::from_wire(slot.kind).unwrap_or(CapType::Untyped);
            Ok(super::Found { kind, rights: slot.rights, object: slot.object, extent: slot.extent })
        }
    }

    impl Authority for Flawed {
        fn reset(&mut self) {
            self.table.clear_all();
        }

        fn seed(&mut self, kind: CapType, rights: u8, object: u64) -> Result<Handle, i32> {
            self.table.grant(kind, rights, object, 0)
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
                    let mut mask = 0u32;
                    for i in 0..TABLE_SLOTS {
                        let Some(slot) = self.table.slots.get(i) else { continue };
                        if slot.occupied() && Handle::from_bits(slot.parent) == handle {
                            mask |= bit(i);
                        }
                    }
                    mask
                }
                _ => self.table.descendants(handle),
            };
            Ok(self.table.sweep(doomed))
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
    }

    impl Failure {
        /// A sentence for the log.
        #[must_use]
        pub fn message(self) -> &'static str {
            match self {
                Self::Real(property) => property.message(),
                Self::NotCaught(_) => "a table broken on purpose passed the negative suite",
                Self::WrongProperty(..) => "a broken table was caught by the wrong property",
                Self::Types(why) => why,
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
            a.seed(CapType::Frame, rights::READ, OBJECT_A).map_err(|_| Property::Unnamed)?;
        let second =
            a.seed(CapType::Frame, rights::READ, OBJECT_A).map_err(|_| Property::Unnamed)?;
        let theirs =
            b.seed(CapType::Frame, rights::READ, OBJECT_B).map_err(|_| Property::Unnamed)?;

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
    fn forged(a: &mut dyn Authority) -> Result<(), Property> {
        a.reset();
        let one = a.seed(CapType::Frame, rights::READ, OBJECT_A).map_err(|_| Property::Forged)?;
        let two = a.seed(CapType::Untyped, rights::READ, OBJECT_B).map_err(|_| Property::Forged)?;

        for index in 0..TABLE_SLOTS {
            for generation in 0..8u16 {
                // `index` is below TABLE_SLOTS and far below u16::MAX.
                let handle = Handle::new(index as u16, generation);
                let expected = handle == one || handle == two;
                if a.inspect(handle).is_ok() != expected {
                    return Err(Property::Forged);
                }
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
        let parent = a.seed(CapType::Frame, root, OBJECT_A).map_err(|_| Property::Stale)?;
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
                Err(code) if code == super::revoked() => {}
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
        let parent = a.seed(CapType::Frame, held, OBJECT_A).map_err(|_| Property::Rights)?;

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
            a.seed(CapType::Frame, rights::READ, OBJECT_A).map_err(|_| Property::Rights)?;
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
        let live = a.seed(CapType::Frame, held, OBJECT_A).map_err(|_| Property::Total)?;

        let hostile = [
            Handle::NULL,
            Handle::new(0, 0),
            // Just past the end, and past it by exactly the table size — which
            // is the value a mask sends back to a slot that is occupied.
            Handle::new(TABLE_SLOTS as u16, live.generation()),
            Handle::new(TABLE_SLOTS as u16 + 1, live.generation()),
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
            if minted > TABLE_SLOTS as u32 {
                // It never refused. There is no bound on the table.
                return Err(Property::Total);
            }
        }
        if minted as usize != TABLE_SLOTS - 1 {
            return Err(Property::Total);
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
    /// # Errors
    ///
    /// A sentence naming what did not hold.
    pub fn self_test() -> Result<usize, Failure> {
        let mut real = Table::EMPTY;
        let mut other = Table::EMPTY;
        if let Err(property) = check(&mut real, &mut other) {
            return Err(Failure::Real(property));
        }

        // Every type in the table, exercised once. Three of the six have no
        // object behind them until M5 and E1, so this is the only place they
        // are held at all — and a type the table cannot carry would otherwise
        // be discovered by the milestone that first needs it.
        let mut types = Table::EMPTY;
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
                .derive(handle, rights::READ)
                .map_err(|_| Failure::Types("a type could not derive"))?;
            // Untyped is the one type whose child is a different type: that is
            // what retyping is, and it is the reason untyped is in the list.
            let expected = if kind == CapType::Untyped { CapType::Frame } else { kind };
            match types.inspect(child) {
                Ok(found) if found.kind == expected => {}
                _ => return Err(Failure::Types("a derivation produced the wrong type")),
            }
            if types.revoke(handle).map(|revoked| revoked.cleared) != Ok(1) {
                return Err(Failure::Types(
                    "revoking a capability did not withdraw the one derived from it",
                ));
            }
        }

        let mut caught = 0;
        for flaw in Flaw::all() {
            let mut broken = Flawed::new(flaw);
            let mut second = Flawed::new(flaw);
            match check(&mut broken, &mut second) {
                Ok(()) => return Err(Failure::NotCaught(flaw)),
                Err(property) if property == flaw.breaks() => caught += 1,
                Err(property) => return Err(Failure::WrongProperty(flaw, property)),
            }
        }
        Ok(caught)
    }
}
