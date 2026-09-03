// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Registration: the service's side of a buffer name.
//!
//! [`buffers`](crate::buffers) is the client's half of RFC 0024 — which side
//! holds the bytes, said in types the compiler checks. This is the other half,
//! and it is checked at runtime because it has to be: the far end of a ring is
//! bound by nothing but its own checks, and a peer that writes raw entries is
//! the hostile peer `ring-scene-boot` section 06 already assumes.
//!
//! # Registration is an operation, and its state is not shared
//!
//! A component asks for a set with an entry — [`f_abi::buf::Request`], on the
//! two opcodes [`f_abi::buf::opcode`] reserves — and gets back a
//! [`SetId`] in the completion. Every submission afterwards
//! names that id, and every one of them is checked against the [`Table`] the
//! service holds.
//!
//! The table is **ordinary private memory of the service**. Nothing about a
//! registration is written into the shared region, and that is a decision
//! rather than an omission: the obvious home for a slot's generation is the
//! four reserved words of [`ChannelHeader`](f_abi::ChannelHeader), which is
//! memory the peer writes. `E0-B15` made the same call about the doorbell
//! counts and the sentence is worth repeating — *evidence of delivery a peer
//! can forge is not evidence*. Here it is stronger: a generation a peer can
//! write is a peer that can un-revoke its own retired set.
//!
//! # A generation that wraps is the bug the generation was added to prevent
//!
//! A slot's generation `retire`s rather than wrapping, and a slot that runs
//! out is never filled again. RFC 0024 chose sixteen bits of generation over a
//! plain index so that a refilled slot could not name a different set under the
//! same number; a counter that wrapped would give that failure back after
//! sixty-five thousand registrations instead of after one, and it would give it
//! back silently. `abi/src/cap.rs` reached the same conclusion for the handle
//! this id is packed like, and [`Table::retired`] is how the cost of it is seen.
//!
//! # Two paths, one behaviour
//!
//! [`Transport`] is what the two paths differ in on this side, exactly as
//! [`Naming`](crate::buffers::Naming) is on the client's. [`Registered`]
//! resolves a set id and an index against the table. [`SharedVirtual`] resolves
//! an address by asking the IOMMU whether the device reaches it by walking the
//! submitter's own page tables. Both answer a [`Reach`], both refuse an
//! out-of-range name with `ARGUMENT`/`BAD_ADDRESS`, and both are refused at
//! *setup* — not at first use — when the channel did not negotiate what they
//! require, which is [`ChannelHeader::negotiate`](f_abi::ChannelHeader::negotiate)'s
//! rule applied one layer up.
//!
//! # The half that has never run on hardware
//!
//! **The shared-virtual-memory path.** There is no device under it and there
//! will not be one in E1: QEMU's virtio offers no address translation services,
//! so nothing this project can currently boot walks a component's page tables.
//! [`PageWalk`] is therefore a seam and not a driver — `E1-B01` supplies the
//! IOMMU behind it, and until then the only implementations are test doubles.
//! What the tests below establish is that the *ownership rules* hold identically
//! over both transports, which is what `E1-B10`'s exit asks for; they establish
//! nothing whatever about ATS, PASID or page-fault latency, and a reader who
//! wants those should look at `E1-B01` and find them absent there too.
//!
//! The registered path has no hardware under it either, for the same reason —
//! [`Domains::map`] is where an IOMMU domain would be programmed. The
//! difference worth stating is that the registered path's *refusals* are this
//! module's own arithmetic and are exercised in full, while the virtual path's
//! central check is delegated to a device that does not exist.
//!
//! # What a bounds check is worth when the hardware speculates past it
//!
//! Every index a peer wrote is checked and then **masked** before it reaches an
//! array. The check is the refusal a correct peer gets. The mask is what a
//! mispredicted branch gets: RFC 0005 says a boundary the hardware speculates
//! through is not a confidentiality boundary, and a slot number confined only
//! by a branch is exactly such a boundary. [`Table`] requires a power-of-two
//! slot count so the mask is one `AND` rather than a clamp with a longer
//! dependency chain — the same reason the ring requires one.

use f_abi::buf::{Name, Request, SetId};
use f_abi::{Cqe, Negotiated, Sqe, error, feature};

use crate::{completion, refusal};

/// A packed [`f_abi::error`] result and the detail RFC 0010 says it carries.
///
/// The pair goes straight into a completion, which is why it is this shape
/// rather than a Rust enum: everything in this module refuses a *peer*, and a
/// peer reads completions.
pub type Refusal = (i32, u64);

/// Memory the device can reach, as the answer to one buffer name.
///
/// Deliberately not a slice. On the registered path the service has no mapping
/// of the client's memory at all — the whole point is that the device reaches
/// it and the CPU does not — so a type that could be dereferenced here would be
/// a type inviting the copy `E1-B02` counts.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Reach {
    /// Where the device addresses it.
    /// Unit: bytes, in the device's address space — which is the IOMMU
    /// domain's on the registered path and the submitter's own on the virtual
    /// one. Zero is never answered: both paths refuse a null.
    pub address: u64,
    /// How much of it this operation named.
    /// Unit: bytes. Zero is a zero-length operation, which is valid and
    /// distinct from an absent one.
    pub len: u32,
}

/// The names this build's transports go by.
///
/// A path is a string here rather than an enum because the only thing anything
/// does with it is say which one ran. [`ALL`](path::ALL) is what the test suite
/// counts against, so a transport declared and never exercised is a test
/// failure rather than a silence — and adding a third path is one constant,
/// one `impl`, and one line in that suite.
pub mod path {
    /// Explicit ranges the component registered. Every device can take it.
    pub const REGISTERED: &str = "registered";
    /// The device walks the component's own page tables. Behind
    /// [`feature::SHARED_VIRTUAL_MEMORY`](f_abi::feature::SHARED_VIRTUAL_MEMORY).
    pub const VIRTUAL: &str = "shared-virtual-memory";
    /// Every path this build offers.
    pub const ALL: &[&str] = &[REGISTERED, VIRTUAL];
}

/// What the frame's IOMMU offers the registered path.
///
/// `E1-B01` is the implementation. A service asks for a translation when a set
/// is registered and gives it back when the set is retired, and that is the
/// whole of what registration costs beyond a table entry —
/// `claims/0004-buffer-registration-cost.toml` is the number.
pub trait Domains {
    /// Give this component's domain a translation for `len` bytes of the memory
    /// `cap` names, and answer the address the device will use for it.
    ///
    /// # Errors
    ///
    /// Whatever the frame refuses with — `AUTHORITY` for a capability the
    /// component does not hold or holds without `GRANT`, `RESOURCE` when the
    /// domain has no room. Passed through to the client unchanged, because a
    /// refusal the service invented a code for is a refusal the client cannot
    /// act on.
    fn map(&mut self, cap: u32, len: u32) -> Result<u64, Refusal>;

    /// Take the translation away again.
    ///
    /// Answers nothing, and cannot refuse. Teardown that can fail is teardown
    /// a peer can decline: RFC 0008 revokes a dead component's buffer sets
    /// whether or not anything is convenient, and
    /// [`InFlight::reclaim`](crate::buffers::InFlight::reclaim) rests on the
    /// transfer faulting rather than landing once this has run.
    fn unmap(&mut self, cap: u32, address: u64, len: u32);
}

/// What the frame's IOMMU offers the shared-virtual-memory path.
///
/// Nothing is registered on this path, so there is nothing to map: the question
/// is only whether the device, walking the submitter's page tables, reaches the
/// bytes an entry named. `E1-B01` supplies it. **No hardware this project can
/// boot answers it** — see the module documentation.
pub trait PageWalk {
    /// Does the device reach `len` bytes at `address` in the submitter's own
    /// address space?
    ///
    /// A negative answer is a refusal and never a page fault this side waits
    /// on: RFC 0010's `ARGUMENT`/`BAD_ADDRESS`, and the entry is answered
    /// rather than parked. A device that takes a fault and recovers is a
    /// device whose driver never asked this.
    fn reaches(&self, address: u64, len: u32) -> bool;
}

/// How a service turns one buffer name into memory the device can reach.
///
/// The only thing the two paths differ in on this side. Everything about *who
/// holds the buffer* is the same across both, which is the property
/// `E1-B10`'s comparison rests on and the reason this is a trait rather than
/// two services.
pub trait Transport {
    /// Feature bits the channel must have negotiated for this transport to be
    /// legal on it. Checked once, at bind; an entry that used a path the
    /// channel did not agree to is refused with
    /// `ARGUMENT`/`FEATURE_NOT_NEGOTIATED` even so, because the two checks
    /// answer different questions — one is about the channel and one is about
    /// an entry.
    const REQUIRES: u64;

    /// Which of [`path::ALL`] this is.
    const PATH: &'static str;

    /// Resolve one name, and record that the device now holds it.
    ///
    /// # Errors
    ///
    /// A [`Refusal`], with the domain and code RFC 0024 names for each case.
    fn resolve(&mut self, name: Name, len: u32) -> Result<Reach, Refusal>;

    /// The device is finished with it.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] for a name that was not out, which is this side's own
    /// bookkeeping gone wrong rather than a peer's doing.
    fn release(&mut self, name: Name) -> Result<(), Refusal>;
}

/// Is `T` legal on a channel that agreed `agreed`?
///
/// One function, called by both transports' `bind`, so the RFC 0011 refusal is
/// written once. Refused rather than downgraded, for the reason
/// [`BufferSet::bind`](crate::buffers::BufferSet::bind) gives: a service that
/// asked for shared virtual memory and quietly got registration would be two
/// peers with different beliefs about what an entry names.
///
/// # Errors
///
/// `PEER`/[`FEATURE_REQUIRED`](error::peer::FEATURE_REQUIRED), the same refusal
/// `negotiate` gives a peer that requires what the other side does not offer.
pub const fn negotiated_for<T: Transport>(agreed: Negotiated) -> Result<(), i32> {
    if agreed.features & T::REQUIRES != T::REQUIRES {
        return Err(error::pack(error::PEER, error::peer::FEATURE_REQUIRED));
    }
    Ok(())
}

/// Buffers one set may hold.
///
/// Sixty-four, because the in-flight bits of a set are one machine word and a
/// bitmap that fits a register is a bitmap with no second-order questions about
/// where it lives. A set that wants more buffers is more sets — RFC 0024
/// already says a client wanting two geometries over one registration wants two
/// sets, and this is the same sentence with a number in it.
///
/// *Reversal:* a driver whose queue depth genuinely exceeds this, which is a
/// real possibility for a network device. Then the bitmap becomes an array and
/// this constant becomes a per-set length; nothing above it changes.
pub const BUFFERS_MAX: u32 = 64;

/// One registration.
#[derive(Clone, Copy)]
struct Slot {
    /// Which occupant of this slot, counting from one. Zero means the slot has
    /// never been filled, which is what makes an id for it unissuable rather
    /// than merely retired.
    ///
    /// Advanced when the registration is *retired*, not when the next one is
    /// made, so an id from before a teardown is stale the moment the teardown
    /// happens rather than the moment something else takes the slot. It
    /// saturates at [`SetId::RETIRED_GENERATION`], and a slot that reaches
    /// that value is never filled again — see [`retire`].
    generation: u16,
    /// Whether the registration is current. A slot that has been used and
    /// retired keeps its generation, so an id naming it is answered `REVOKED`
    /// rather than `NO_SUCH_CAP` — the two need different handling and RFC 0010
    /// says so.
    live: bool,
    /// The memory capability this set was derived from.
    cap: u32,
    /// Where the device addresses the first buffer.
    address: u64,
    /// Bytes per buffer.
    stride: u32,
    /// Buffers in the set.
    buffers: u32,
    /// One bit per buffer: set while the device holds it. The per-buffer cost
    /// RFC 0024 writes down beside the rule it pays for — without it the double
    /// submission is a thing the service cannot see.
    lent: u64,
}

impl Slot {
    const EMPTY: Self =
        Self { generation: 0, live: false, cap: 0, address: 0, stride: 0, buffers: 0, lent: 0 };
}

/// The registrations one channel holds, in the service's own memory.
///
/// `SLOTS` is a power of two and at most 65 536, because a
/// [`SetId`]'s slot index is sixteen bits and because the index is masked
/// rather than clamped — see the module documentation on speculation.
///
/// There is no allocator here and no growth: a service that runs out of slots
/// refuses with `RESOURCE`/`QUOTA_EXHAUSTED`, which is a peer being told it
/// asked for too much rather than a peer deciding how much memory this side
/// commits. That is the same argument `E1-B13` makes about the capability
/// table, one size down.
pub struct Table<const SLOTS: usize> {
    slots: [Slot; SLOTS],
}

impl<const SLOTS: usize> Default for Table<SLOTS> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const SLOTS: usize> Table<SLOTS> {
    /// An empty table.
    #[must_use]
    pub const fn new() -> Self {
        const {
            assert!(
                SLOTS.is_power_of_two(),
                "a slot count that is not a power of two cannot be masked"
            );
            assert!(SLOTS <= 1 << 16, "a SetId names a slot in sixteen bits");
        }
        Self { slots: [Slot::EMPTY; SLOTS] }
    }

    /// Registrations currently live. Unit: buffer sets.
    #[must_use]
    pub fn live(&self) -> usize {
        self.slots.iter().filter(|slot| slot.live).count()
    }

    /// Slots used up, which may never be filled again. Unit: buffer sets.
    ///
    /// Reported for the reason `kernel/src/cap.rs` reports the same number: not
    /// wrapping the generation has a cost, and a cost nothing can observe is a
    /// cost somebody will discover as a capacity bug. A slot reaches this after
    /// [`SetId::RETIRED_GENERATION`] minus one registrations, so a table that
    /// shows anything here is a table whose turnover is worth looking at.
    #[must_use]
    pub fn retired(&self) -> usize {
        self.slots.iter().filter(|slot| slot.generation == SetId::RETIRED_GENERATION).count()
    }

    /// Slots this table has. Unit: buffer sets.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        SLOTS
    }

    /// Register `len` bytes of the memory `cap` names, as `buffers` equal
    /// buffers.
    ///
    /// The translation is asked for *before* the slot is filled, so a domain
    /// that refuses leaves no half-registration behind and no generation spent.
    ///
    /// # Errors
    ///
    /// `ARGUMENT`/[`BAD_ADDRESS`](error::argument::BAD_ADDRESS) for a geometry
    /// that is not a set — no bytes, no buffers, or a region that does not
    /// divide evenly. `RESOURCE`/[`QUOTA_EXHAUSTED`](error::resource::QUOTA_EXHAUSTED)
    /// for more buffers than [`BUFFERS_MAX`], or for no free slot — which
    /// includes a table whose free slots have all been *retired*, and is the
    /// same refusal with the same detail, because "there is no slot for you" is
    /// what the peer needs to act on and how the slot was spent is not its
    /// business. Whatever [`Domains::map`] refuses with, unchanged.
    pub fn register<D: Domains>(
        &mut self,
        cap: u32,
        len: u32,
        buffers: u32,
        domains: &mut D,
    ) -> Result<SetId, Refusal> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let quota = error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED);

        if buffers == 0 {
            return Err((bad, 0));
        }
        if buffers > BUFFERS_MAX {
            return Err((quota, u64::from(BUFFERS_MAX)));
        }
        if len == 0 || !len.is_multiple_of(buffers) {
            return Err((bad, u64::from(len)));
        }

        // The lowest free slot that has not been used up, always. An allocation
        // order that depended on anything else — most recently freed, a hash of
        // the capability — would be a place a seeded run stopped reproducing,
        // and RFC 0004 says the only source of that is `f_env::Env`.
        #[cfg(not(feature = "mutate-reusable-slot"))]
        let free = |slot: &Slot| !slot.live && slot.generation != SetId::RETIRED_GENERATION;

        // `E1-P05`'s defect for the *ledger* oracle: a slot that has run out of
        // generations is filled again, so the sixty-five thousand five hundred
        // and thirty-fifth registration of one slot answers to a name an earlier
        // client may still hold — with no event anywhere. It is the exact
        // failure RFC 0024 rejected a plain index to avoid, arriving sixty-five
        // thousand registrations late instead of immediately.
        //
        // The defect is **here** and not in `retire`, and finding that out cost
        // a run: `retire`'s `saturating_add` and a `wrapping_add` in its place
        // are indistinguishable, because this predicate already refuses a slot
        // at `RETIRED_GENERATION` and so `retire` is never asked to add to it.
        // The two guards are defence in depth and exactly one of them is
        // load-bearing. RFC 0048.
        #[cfg(feature = "mutate-reusable-slot")]
        let free = |slot: &Slot| !slot.live;
        let Some(index) = self.slots.iter().position(free) else {
            return Err((quota, SLOTS as u64));
        };

        let address = domains.map(cap, len)?;
        // Already advanced, by whatever retired the previous occupant — so the
        // id this issues is stale for anybody holding one from before that, and
        // was stale from the moment of the retirement rather than from now.
        let generation = if self.slots[index].generation == 0 {
            SetId::FIRST_GENERATION
        } else {
            self.slots[index].generation
        };
        self.slots[index] =
            Slot { generation, live: true, cap, address, stride: len / buffers, buffers, lent: 0 };
        // Fits: `index < SLOTS` and `SLOTS <= 1 << 16`, asserted at construction.
        #[allow(clippy::cast_possible_truncation)]
        let slot = index as u16;
        Ok(SetId::new(slot, generation))
    }

    /// Retire a set.
    ///
    /// Succeeds with buffers still in flight, and that is the decision rather
    /// than an oversight: the memory is the client's and it is entitled to take
    /// it back. What makes that safe is the same thing that makes
    /// [`InFlight::reclaim`](crate::buffers::InFlight::reclaim) safe — the
    /// translation goes away with the registration, so a transfer the device
    /// had already started faults instead of landing in memory somebody is
    /// about to reuse.
    ///
    /// # Errors
    ///
    /// `AUTHORITY`/[`NO_SUCH_CAP`](error::authority::NO_SUCH_CAP) for an id
    /// this table never issued, `AUTHORITY`/[`REVOKED`](error::authority::REVOKED)
    /// for one whose generation has been retired.
    pub fn unregister<D: Domains>(&mut self, set: SetId, domains: &mut D) -> Result<(), Refusal> {
        let index = self.slot_of(set)?;
        let slot = self.slots[index];
        domains.unmap(slot.cap, slot.address, slot.stride * slot.buffers);
        self.slots[index].live = false;
        self.slots[index].lent = 0;
        // Spent here, the same as in `retire_all` and for the same reason: an
        // id is stale from the moment its set is retired, not from the moment
        // something else happens to want the slot.
        self.slots[index].generation = retire(slot.generation);
        Ok(())
    }

    /// The peer restarted: every set it holds is stale.
    ///
    /// The generation is spent here rather than at the next registration — as
    /// it is in [`Table::unregister`], which is the same event one set at a
    /// time — so that an id from before the restart is `REVOKED` immediately
    /// and not only once something else has taken the slot. That is the whole
    /// reason a [`SetId`] carries a generation at all — RFC 0024 rejected a
    /// plain index because a refilled slot would name a different set under the
    /// same number with no event anywhere, which is also why the generation
    /// `retire`s rather than wrapping.
    ///
    /// Answers how many registrations were retired. Unit: buffer sets.
    pub fn retire_all<D: Domains>(&mut self, domains: &mut D) -> usize {
        let mut retired = 0;
        for slot in &mut self.slots {
            if !slot.live {
                continue;
            }
            domains.unmap(slot.cap, slot.address, slot.stride * slot.buffers);
            slot.live = false;
            slot.lent = 0;
            slot.generation = retire(slot.generation);
            retired += 1;
        }
        retired
    }

    /// Resolve one buffer of one set, and record that the device holds it.
    ///
    /// # Errors
    ///
    /// `AUTHORITY`/`NO_SUCH_CAP` and `AUTHORITY`/`REVOKED` as
    /// [`Table::unregister`]. `ARGUMENT`/[`BAD_ADDRESS`](error::argument::BAD_ADDRESS)
    /// for an index past the set, a length past the buffer, or a buffer the
    /// device already holds — whose text already says *already occupied* —
    /// detail the offending field.
    pub fn resolve(&mut self, set: SetId, index: u32, len: u32) -> Result<Reach, Refusal> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let slot = self.slot_of(set)?;
        let slot = &mut self.slots[slot];

        // `E1-P05`'s defect for the *reach* oracle, and it is the sentence below
        // this block taken literally: *checked above and masked here*. With the
        // check gone the mask is all that is left, so an index past the end of
        // the set resolves — to a plausible address inside somebody else's
        // buffer, with no refusal and no crash. A fuzzer watching for a panic
        // sees nothing; an oracle that knows where the buffer should have been
        // sees it at once. It is applied to `Table::release` as well, because
        // half a lenient index is a service that hands out a buffer it will not
        // take back, which is a different bug. RFC 0048.
        #[cfg(not(feature = "mutate-lenient-index"))]
        if index >= slot.buffers {
            return Err((bad, u64::from(index)));
        }
        if len > slot.stride {
            return Err((bad, u64::from(len)));
        }
        // Checked above and masked here, for the reason the module gives. The
        // shift is then total whatever a branch predictor believes, which is
        // what stops a mispredicted path shifting by a peer-chosen amount.
        let bit = 1u64 << (index & (BUFFERS_MAX - 1));
        if slot.lent & bit != 0 {
            return Err((bad, u64::from(index)));
        }
        slot.lent |= bit;

        Ok(Reach { address: slot.address + u64::from(index) * u64::from(slot.stride), len })
    }

    /// The device is finished with one buffer.
    ///
    /// # Errors
    ///
    /// `AUTHORITY`/`NO_SUCH_CAP` and `AUTHORITY`/`REVOKED` as above.
    /// `ARGUMENT`/`BAD_ADDRESS` for a buffer this table did not have out, which
    /// is a service completing something twice rather than a peer misbehaving —
    /// refused so that the second completion is visible instead of quietly
    /// making a live buffer look free.
    pub fn release(&mut self, set: SetId, index: u32) -> Result<(), Refusal> {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let slot = self.slot_of(set)?;
        let slot = &mut self.slots[slot];

        // The other half of `mutate-lenient-index`; `Table::resolve` says why.
        #[cfg(not(feature = "mutate-lenient-index"))]
        if index >= slot.buffers {
            return Err((bad, u64::from(index)));
        }
        let bit = 1u64 << (index & (BUFFERS_MAX - 1));
        if slot.lent & bit == 0 {
            return Err((bad, u64::from(index)));
        }
        slot.lent &= !bit;
        Ok(())
    }

    /// Execute one registration entry and produce the completion it earns.
    ///
    /// A registration always completes, whatever [`f_abi::flags::NO_CQE`] says.
    /// The flag means *I do not need to be told this worked*, and a client that
    /// registered a set and was not told its id holds a set it cannot name —
    /// which is not a fire-and-forget operation, it is a leak.
    ///
    /// `now` is passed in rather than read, because this crate observes no
    /// clock: RFC 0004, and the determinism lint would refuse a call to one.
    pub fn execute<D: Domains>(&mut self, entry: &Sqe, domains: &mut D, now: u64) -> Cqe {
        let token = entry.user_data;
        let answered = match Request::read(entry) {
            Ok(Request::Register { cap, len, buffers }) => {
                match self.register(cap, len, buffers, domains) {
                    Ok(set) => Ok(self.issued(token, set, now)),
                    Err(refused) => Err(refused),
                }
            }
            // Answers a zero `ext`, which is not a set id and is not meant to
            // be read as one: `SetId::from_completion` refuses it. What a
            // client learns from an unregistration is that it succeeded.
            Ok(Request::Unregister { set }) => {
                self.unregister(set, domains).map(|()| completion(token, 0, now))
            }
            Err(refused) => Err(refused),
        };

        match answered {
            Ok(cqe) => cqe,
            Err((packed, detail)) => refusal(token, packed, detail, now),
        }
    }

    /// The completion that issues an id: this table's answer to a registration.
    ///
    /// Beside [`crate::completion`] and [`crate::refusal`] and for the same
    /// reason — one account of what an answer looks like rather than one per
    /// service. What makes it a method on the table rather than a free function
    /// is what a caller must have in hand to call it: a table that *holds the
    /// set*. An id this table does not hold earns the refusal
    /// `slot_of` gives it, so the completion carries an id only when
    /// something with a registration in it said so.
    ///
    /// That is a witness and not a proof. Nothing stops a client standing up a
    /// [`Table`] of its own, registering into it, and reading its own answer
    /// back — which is a program lying to itself rather than a forged
    /// authority, because the id it mints names a slot in its own table and
    /// nothing in the service's. `f_ring::buffers` says the same thing where a
    /// reader of the ownership types will see it.
    #[must_use]
    pub fn issued(&self, user_data: u64, set: SetId, timestamp: u64) -> Cqe {
        match self.slot_of(set) {
            Ok(_) => Cqe { user_data, result: 0, flags: 0, timestamp, ext: set.bits() as u64 },
            Err((packed, detail)) => refusal(user_data, packed, detail, timestamp),
        }
    }

    /// Which slot an id names, if this table issued it and has not retired it.
    fn slot_of(&self, set: SetId) -> Result<usize, Refusal> {
        let named = u64::from(set.bits());
        let unknown = (error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP), named);
        let revoked = (error::pack(error::AUTHORITY, error::authority::REVOKED), named);

        if !set.is_issuable() {
            return Err(unknown);
        }
        let index = set.index() as usize;
        if index >= SLOTS {
            return Err(unknown);
        }
        // Checked, then masked. The check is the refusal a correct peer gets;
        // the mask is what a mispredicted branch gets, and RFC 0005 is why the
        // difference is worth an instruction. `SLOTS` is a power of two, so
        // this is one `AND`.
        let index = index & (SLOTS - 1);

        let slot = &self.slots[index];
        if slot.generation == 0 {
            // Never filled. An id for it was issued by nobody.
            return Err(unknown);
        }
        if slot.generation != set.generation() || !slot.live {
            // Retired, or overtaken. Both are `REVOKED` rather than `NO_SUCH_CAP`
            // because both name a slot this table has used, and a peer that
            // forged a generation lands here too — being told *revoked* tells it
            // nothing it did not already supply.
            return Err(revoked);
        }
        Ok(index)
    }
}

/// Move a slot on to its next occupant, saturating.
///
/// **The saturation is the whole of it.** A generation that wrapped would bring
/// a retired id back to life: the same slot, the same sixteen bits, resolving
/// once more — into whatever memory now occupies the slot, with no event
/// anywhere. That is precisely the failure RFC 0024 rejected a plain index to
/// avoid, so a counter that wraps reinstates it after sixty-five thousand
/// registrations instead of after one. `abi/src/cap.rs` reached this conclusion
/// first, for a handle packed the same way, and `kernel/src/cap.rs` implements
/// it the same way: a slot that reaches [`SetId::RETIRED_GENERATION`] is
/// retired and never filled again, which turns a soundness hole into running
/// out of slots — and running out of slots is something a peer can be told.
///
/// *Reversal:* a table whose slots turn over so fast that retirement is a real
/// bound in a running system. [`Table::retired`] is what makes that visible
/// before it is a problem; the fix is a wider generation field, which is an ABI
/// change, and not a wrap.
const fn retire(generation: u16) -> u16 {
    generation.saturating_add(1)
}

/// The registered path: an entry names a set and an index, and the service
/// resolves it against the table it holds. No address ever crosses the boundary.
pub struct Registered<'t, const SLOTS: usize> {
    table: &'t mut Table<SLOTS>,
}

impl<'t, const SLOTS: usize> Registered<'t, SLOTS> {
    /// Bind to a table on a channel that negotiated `agreed`.
    ///
    /// # Errors
    ///
    /// Never today — the registered path requires no feature, because it is the
    /// path every device can take. The signature is the same as
    /// [`SharedVirtual::bind`]'s so that the two are one call at the call site
    /// and so that the day registration acquires a feature bit of its own is a
    /// change to one constant.
    pub const fn bind(agreed: Negotiated, table: &'t mut Table<SLOTS>) -> Result<Self, i32> {
        match negotiated_for::<Self>(agreed) {
            Ok(()) => Ok(Self { table }),
            Err(refused) => Err(refused),
        }
    }

    /// The table this resolves against.
    pub const fn table(&mut self) -> &mut Table<SLOTS> {
        self.table
    }
}

impl<const SLOTS: usize> Transport for Registered<'_, SLOTS> {
    const REQUIRES: u64 = 0;
    const PATH: &'static str = path::REGISTERED;

    fn resolve(&mut self, name: Name, len: u32) -> Result<Reach, Refusal> {
        match name {
            Name::Registered { set, index } => self.table.resolve(set, index, len),
            // An address, on a service that resolves through a registration
            // table. The channel may even have negotiated shared virtual memory
            // — the path is per entry — but this service did not bind that
            // transport, so the entry uses a feature outside what *it* agreed.
            // RFC 0024 names this refusal.
            Name::Virtual { .. } => Err((
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
                feature::SHARED_VIRTUAL_MEMORY,
            )),
        }
    }

    fn release(&mut self, name: Name) -> Result<(), Refusal> {
        match name {
            Name::Registered { set, index } => self.table.release(set, index),
            Name::Virtual { .. } => Err((
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
                feature::SHARED_VIRTUAL_MEMORY,
            )),
        }
    }
}

/// The shared-virtual-memory path: an entry names an address in the submitter's
/// own space and the device walks the submitter's page tables to reach it.
///
/// Nothing is registered, so there is no table, no generation and no in-flight
/// bit. RFC 0024 states the consequence rather than hiding it: a client that
/// bypasses the ownership types and submits the same address twice is refused
/// by nothing here. It tears its own memory, which is its own bug and not the
/// service's breach — and the honest place to say so is beside the code that
/// does not check it.
///
/// **Nothing under this has ever run on hardware.** See the module docs.
pub struct SharedVirtual<'w, W: PageWalk> {
    walk: &'w W,
}

impl<'w, W: PageWalk> SharedVirtual<'w, W> {
    /// Bind to an IOMMU on a channel that negotiated `agreed`.
    ///
    /// # Errors
    ///
    /// `PEER`/[`FEATURE_REQUIRED`](error::peer::FEATURE_REQUIRED) when the
    /// channel did not agree [`feature::SHARED_VIRTUAL_MEMORY`]. Refused here,
    /// at setup, and not at the first entry that carries an address: a service
    /// that discovered at first use that it could not read the entries it had
    /// been sent would have accepted a channel it cannot serve.
    pub const fn bind(agreed: Negotiated, walk: &'w W) -> Result<Self, i32> {
        match negotiated_for::<Self>(agreed) {
            Ok(()) => Ok(Self { walk }),
            Err(refused) => Err(refused),
        }
    }
}

impl<W: PageWalk> Transport for SharedVirtual<'_, W> {
    const REQUIRES: u64 = feature::SHARED_VIRTUAL_MEMORY;
    const PATH: &'static str = path::VIRTUAL;

    fn resolve(&mut self, name: Name, len: u32) -> Result<Reach, Refusal> {
        match name {
            Name::Virtual { address } => {
                if self.walk.reaches(address, len) {
                    Ok(Reach { address, len })
                } else {
                    Err((error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS), address))
                }
            }
            // Nothing was registered on this path, so no id names anything —
            // including one a client made up, which is every one of them here.
            Name::Registered { set, .. } => Err((
                error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                u64::from(set.bits()),
            )),
        }
    }

    fn release(&mut self, name: Name) -> Result<(), Refusal> {
        match name {
            // Nothing to give back. The ledger of who holds what is the
            // client's here — `f_ring::buffers` — and that is precisely the part
            // section 04 says must survive when registration does not.
            Name::Virtual { .. } => Ok(()),
            Name::Registered { set, .. } => Err((
                error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                u64::from(set.bits()),
            )),
        }
    }
}

/// The entry that asks a service to register a region.
///
/// `token` is returned in the completion and is how the answer is matched, the
/// same as any other submission. The class is [`Sqe::ZERO`]'s, which is
/// `BATCH`: registration is setup, and setup that claimed the one
/// admission-controlled class would be a component claiming urgency for its own
/// bookkeeping.
#[must_use]
pub fn registration(token: u64, cap: u32, len: u32, buffers: u32) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.user_data = token;
    Request::Register { cap, len, buffers }.write(&mut entry);
    entry
}

/// The entry that retires one.
#[must_use]
pub fn unregistration(token: u64, set: SetId) -> Sqe {
    let mut entry = Sqe::ZERO;
    entry.user_data = token;
    Request::Unregister { set }.write(&mut entry);
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame that hands out one address and counts what it was asked.
    ///
    /// Standing in for `E1-B01`. What it models is the only thing this module
    /// depends on: that a registration acquires a translation and a retirement
    /// gives it back, in that order and in pairs.
    struct Pinned {
        base: u64,
        mapped: u32,
        unmapped: u32,
        refuse: bool,
    }

    impl Pinned {
        const fn at(base: u64) -> Self {
            Self { base, mapped: 0, unmapped: 0, refuse: false }
        }
    }

    impl Domains for Pinned {
        fn map(&mut self, _cap: u32, _len: u32) -> Result<u64, Refusal> {
            if self.refuse {
                return Err((error::pack(error::RESOURCE, error::resource::DEVICE_FULL), 0));
            }
            self.mapped += 1;
            Ok(self.base)
        }

        fn unmap(&mut self, _cap: u32, _address: u64, _len: u32) {
            self.unmapped += 1;
        }
    }

    fn table() -> (Table<8>, Pinned) {
        (Table::new(), Pinned::at(0x1000))
    }

    #[test]
    fn a_registration_issues_an_id_and_a_translation() {
        let (mut table, mut frame) = table();
        let set = table.register(3, 256, 4, &mut frame).expect("a region that divides");

        assert_eq!(set.index(), 0, "the lowest free slot");
        assert_eq!(set.generation(), SetId::FIRST_GENERATION);
        assert_eq!(frame.mapped, 1, "one registration is one translation");
        assert_eq!(table.live(), 1);
        assert_eq!(table.capacity(), 8);

        // Every buffer of the set resolves to its own place in the region.
        for index in 0..4 {
            let reach = table.resolve(set, index, 64).expect("inside the set");
            assert_eq!(reach, Reach { address: 0x1000 + u64::from(index) * 64, len: 64 });
        }
    }

    #[test]
    fn a_geometry_that_is_not_a_set_is_refused() {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let quota = error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED);
        let (mut table, mut frame) = table();

        assert_eq!(table.register(0, 256, 0, &mut frame), Err((bad, 0)), "no buffers");
        assert_eq!(table.register(0, 0, 4, &mut frame), Err((bad, 0)), "no bytes");
        assert_eq!(table.register(0, 100, 3, &mut frame), Err((bad, 100)), "does not divide");
        assert_eq!(
            table.register(0, 4096, BUFFERS_MAX + 1, &mut frame),
            Err((quota, u64::from(BUFFERS_MAX))),
            "more buffers than a set's bitmap holds"
        );
        assert_eq!(frame.mapped, 0, "a refused registration asks the frame for nothing");
    }

    #[test]
    fn a_table_with_no_room_refuses_rather_than_evicting() {
        let (mut table, mut frame) = table();
        for _ in 0..8 {
            table.register(0, 64, 1, &mut frame).expect("a free slot");
        }
        assert_eq!(
            table.register(0, 64, 1, &mut frame),
            Err((error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED), 8))
        );
    }

    #[test]
    fn a_domain_that_refuses_leaves_no_half_registration() {
        let (mut table, mut frame) = table();
        frame.refuse = true;
        assert!(table.register(1, 64, 1, &mut frame).is_err());
        assert_eq!(table.live(), 0);

        // And the generation was not spent: the first id this table issues is
        // still generation one, so a refusal is not a way to walk a slot's
        // generation forward.
        frame.refuse = false;
        let set = table.register(1, 64, 1, &mut frame).unwrap();
        assert_eq!(set.generation(), SetId::FIRST_GENERATION);
    }

    #[test]
    fn an_id_nobody_issued_is_refused_and_a_retired_one_is_refused_differently() {
        let unknown = error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP);
        let revoked = error::pack(error::AUTHORITY, error::authority::REVOKED);
        let (mut table, mut frame) = table();
        let set = table.register(0, 256, 4, &mut frame).unwrap();

        // Generation zero: unissuable by construction.
        let zeroed = SetId::new(0, 0);
        assert_eq!(table.resolve(zeroed, 0, 4), Err((unknown, u64::from(zeroed.bits()))));
        // A slot past the table, and a slot this table has never filled.
        let past = SetId::new(99, 1);
        assert_eq!(table.resolve(past, 0, 4), Err((unknown, u64::from(past.bits()))));
        let empty = SetId::new(3, 1);
        assert_eq!(table.resolve(empty, 0, 4), Err((unknown, u64::from(empty.bits()))));

        // Retired: the same slot, and the id it was issued under.
        table.unregister(set, &mut frame).expect("a live set");
        assert_eq!(frame.unmapped, 1);
        assert_eq!(table.resolve(set, 0, 4), Err((revoked, u64::from(set.bits()))));
        assert_eq!(table.unregister(set, &mut frame), Err((revoked, u64::from(set.bits()))));
    }

    #[test]
    fn a_buffer_from_a_previous_registration_cannot_be_named_after_the_slot_is_reused() {
        // The failure a plain index would have had, and the reason RFC 0024
        // paid two bytes for a generation: the slot is refilled, the old id
        // still names slot zero, and without the generation it would resolve
        // into somebody else's memory with no event anywhere.
        let (mut table, mut frame) = table();
        let first = table.register(1, 256, 4, &mut frame).unwrap();
        table.unregister(first, &mut frame).unwrap();

        let mut elsewhere = Pinned::at(0x9000);
        let second = table.register(2, 256, 4, &mut elsewhere).unwrap();
        assert_eq!(second.index(), first.index(), "the same slot");
        assert_ne!(second.generation(), first.generation(), "and not the same set");

        assert_eq!(
            table.resolve(first, 0, 4),
            Err((
                error::pack(error::AUTHORITY, error::authority::REVOKED),
                u64::from(first.bits())
            ))
        );
        assert_eq!(table.resolve(second, 0, 4).unwrap().address, 0x9000);
    }

    #[test]
    fn a_slot_is_retired_rather_than_wrapping_its_generation() {
        // The failure this is here to refuse: sixty-five thousand five hundred
        // and thirty-four registrations of one slot, and then an id issued at
        // generation one again — at which point the very first id, long since
        // revoked, resolves into whatever is in the slot now. `Table<1>` so the
        // slot has nowhere else to go and the exhaustion is visible.
        let mut table = Table::<1>::new();
        let mut frame = Pinned::at(0x1000);

        let first = table.register(0, 64, 1, &mut frame).expect("a free slot");
        assert_eq!(first.generation(), SetId::FIRST_GENERATION);
        table.unregister(first, &mut frame).unwrap();

        let mut last = first;
        loop {
            match table.register(0, 64, 1, &mut frame) {
                Ok(set) => {
                    last = set;
                    table.unregister(set, &mut frame).unwrap();
                }
                Err(refused) => {
                    assert_eq!(
                        refused,
                        (error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED), 1),
                        "a table of retired slots is a table with no room, and says so"
                    );
                    break;
                }
            }
        }

        // Every id was distinct, and the last one stopped short of the value
        // that means *spent* — so no id was ever issued twice.
        assert_eq!(u32::from(last.generation()), u32::from(SetId::RETIRED_GENERATION) - 1);
        assert_ne!(last.generation(), first.generation());

        // The first id is still refused, which is the property the whole
        // arrangement exists for. Under a wrapping counter it would be live.
        assert_eq!(
            table.resolve(first, 0, 4),
            Err((
                error::pack(error::AUTHORITY, error::authority::REVOKED),
                u64::from(first.bits())
            ))
        );

        // And the cost of not wrapping is reportable rather than theoretical.
        assert_eq!(table.retired(), 1);
        assert_eq!(table.live(), 0);
    }

    #[test]
    fn a_retired_slot_is_stepped_over_and_the_rest_of_the_table_still_works() {
        // Retirement is one slot's, not the table's: the neighbours are
        // untouched, which is what makes it "a table one slot smaller" rather
        // than a table that stops.
        let mut table = Table::<2>::new();
        let mut frame = Pinned::at(0x1000);

        while let Ok(set) = table.register(0, 64, 1, &mut frame) {
            if set.index() == 1 {
                // Slot zero is spent; slot one is answering now. Stop here
                // rather than spending it too.
                assert_eq!(table.retired(), 1);
                assert_eq!(set.generation(), SetId::FIRST_GENERATION, "a fresh slot starts at one");
                assert!(table.resolve(set, 0, 64).is_ok());
                table.unregister(set, &mut frame).unwrap();
                return;
            }
            table.unregister(set, &mut frame).unwrap();
        }
        panic!("slot one should have taken over when slot zero was used up");
    }

    #[test]
    fn a_peer_restart_retires_every_set_and_re_registration_gets_a_new_generation() {
        let (mut table, mut frame) = table();
        let a = table.register(1, 256, 4, &mut frame).unwrap();
        let b = table.register(2, 128, 2, &mut frame).unwrap();

        assert_eq!(table.retire_all(&mut frame), 2);
        assert_eq!(frame.unmapped, 2, "every translation goes back");
        assert_eq!(table.live(), 0);

        let revoked = error::pack(error::AUTHORITY, error::authority::REVOKED);
        assert_eq!(table.resolve(a, 0, 4), Err((revoked, u64::from(a.bits()))));
        assert_eq!(table.resolve(b, 0, 4), Err((revoked, u64::from(b.bits()))));

        let again = table.register(1, 256, 4, &mut frame).unwrap();
        assert_eq!(again.index(), a.index());
        assert_ne!(again.generation(), a.generation());
        assert!(table.resolve(again, 0, 4).is_ok());
    }

    #[test]
    fn an_index_past_the_set_and_a_length_past_the_buffer_are_both_refused() {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let (mut table, mut frame) = table();
        let set = table.register(0, 256, 4, &mut frame).unwrap();

        assert_eq!(table.resolve(set, 4, 8), Err((bad, 4)), "one past the set");
        assert_eq!(table.resolve(set, u32::MAX, 8), Err((bad, u64::from(u32::MAX))));
        assert_eq!(table.resolve(set, 0, 65), Err((bad, 65)), "one byte past the buffer");
        assert!(table.resolve(set, 0, 64).is_ok(), "the whole buffer is a prefix of itself");
    }

    #[test]
    fn a_buffer_the_device_already_holds_is_refused_until_it_is_given_back() {
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let (mut table, mut frame) = table();
        let set = table.register(0, 256, 4, &mut frame).unwrap();

        assert!(table.resolve(set, 1, 64).is_ok());
        assert_eq!(table.resolve(set, 1, 64), Err((bad, 1)), "already occupied");
        assert!(table.resolve(set, 2, 64).is_ok(), "and its neighbour is unaffected");

        assert_eq!(table.release(set, 3), Err((bad, 3)), "a buffer that was never out");
        table.release(set, 1).expect("the device is done with it");
        assert!(table.resolve(set, 1, 64).is_ok(), "and round it goes again");
    }

    #[test]
    fn a_transport_is_refused_at_setup_and_not_at_first_use() {
        // RFC 0011 style, one layer up from `negotiate`: a service that cannot
        // read the entries it will be sent must not accept the channel.
        let walk = Reaches { base: 0, len: 0 };
        let none = Negotiated { version: f_abi::ABI_VERSION, features: 0 };
        assert_eq!(
            SharedVirtual::bind(none, &walk).map(|_| ()),
            Err(error::pack(error::PEER, error::peer::FEATURE_REQUIRED))
        );

        let svm =
            Negotiated { version: f_abi::ABI_VERSION, features: feature::SHARED_VIRTUAL_MEMORY };
        assert!(SharedVirtual::bind(svm, &walk).is_ok());

        // Registration needs nothing, so it binds on either kind of channel.
        let mut first = Table::<8>::new();
        assert!(Registered::bind(none, &mut first).is_ok());
        let mut second = Table::<8>::new();
        assert!(Registered::bind(svm, &mut second).is_ok());
    }

    /// An IOMMU that reaches one contiguous region and nothing else.
    struct Reaches {
        base: u64,
        len: u32,
    }

    impl PageWalk for Reaches {
        fn reaches(&self, address: u64, len: u32) -> bool {
            let end = self.base + u64::from(self.len);
            address >= self.base
                && address.checked_add(u64::from(len)).is_some_and(|last| last <= end)
        }
    }

    #[test]
    fn each_transport_refuses_the_other_path_s_name() {
        let mut table = Table::<8>::new();
        let mut frame = Pinned::at(0x1000);
        let set = table.register(0, 256, 4, &mut frame).unwrap();
        let none = Negotiated { version: f_abi::ABI_VERSION, features: 0 };
        let mut fixed = Registered::bind(none, &mut table).unwrap();

        assert_eq!(
            fixed.resolve(Name::Virtual { address: 0x1000 }, 8),
            Err((
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
                feature::SHARED_VIRTUAL_MEMORY
            ))
        );

        let walk = Reaches { base: 0x1000, len: 256 };
        let svm =
            Negotiated { version: f_abi::ABI_VERSION, features: feature::SHARED_VIRTUAL_MEMORY };
        let mut virt = SharedVirtual::bind(svm, &walk).unwrap();
        assert_eq!(
            virt.resolve(Name::Registered { set, index: 0 }, 8),
            Err((
                error::pack(error::AUTHORITY, error::authority::NO_SUCH_CAP),
                u64::from(set.bits())
            ))
        );
    }

    #[test]
    fn the_virtual_path_cannot_refuse_a_double_submission_and_says_so() {
        // Not a defect here. RFC 0024: on the virtual path the service has no
        // registration to keep an in-flight bit against, so a client that
        // bypasses the ownership types and submits one address twice is refused
        // by nothing — it tears its own memory. The asymmetry is asserted so
        // that it is a stated property rather than a gap somebody discovers,
        // and so that the day an IOMMU offers a way to see it, this test fails.
        let walk = Reaches { base: 0x2000, len: 128 };
        let svm =
            Negotiated { version: f_abi::ABI_VERSION, features: feature::SHARED_VIRTUAL_MEMORY };
        let mut virt = SharedVirtual::bind(svm, &walk).unwrap();

        let name = Name::Virtual { address: 0x2000 };
        assert!(virt.resolve(name, 64).is_ok());
        assert!(virt.resolve(name, 64).is_ok(), "and nothing here can tell");

        // What both paths do refuse is a name the device cannot reach at all.
        assert!(virt.resolve(Name::Virtual { address: 0x2000 }, 129).is_err());
        assert!(virt.resolve(Name::Virtual { address: 0x1FFF }, 8).is_err());
    }

    #[test]
    fn registration_is_an_operation_and_answers_over_the_wire() {
        let mut table = Table::<8>::new();
        let mut frame = Pinned::at(0x4000);

        let asked = registration(11, 3, 256, 4);
        let answer = table.execute(&asked, &mut frame, 7);
        assert_eq!(answer.user_data, 11, "the token comes back");
        assert_eq!(answer.timestamp, 7, "stamped with the clock it was given");
        let set = SetId::from_completion(&answer).expect("an id was issued");
        assert!(table.resolve(set, 0, 64).is_ok());

        let retire = unregistration(12, set);
        let answer = table.execute(&retire, &mut frame, 8);
        assert!(!answer.is_error());
        assert_eq!(answer.user_data, 12);
        assert_eq!(
            SetId::from_completion(&answer),
            Err((error::AUTHORITY, error::authority::NO_SUCH_CAP)),
            "an unregistration issues no id, and its completion does not pretend to"
        );

        // A malformed entry is refused with the code its own field earns, and
        // the refusal reaches the client as a completion rather than as a
        // teardown: the channel is healthy, one entry is not.
        let mut malformed = registration(13, 0, 256, 4);
        malformed.offset = 9;
        let answer = table.execute(&malformed, &mut frame, 9);
        assert_eq!(answer.error(), Some((error::ARGUMENT, error::argument::RESERVED_NOT_ZERO)));
        assert_eq!(answer.ext, 9, "a refusal names the offending field");

        // And an opcode this table does not answer on.
        let mut foreign = Sqe::ZERO;
        foreign.opcode = 1;
        let answer = table.execute(&foreign, &mut frame, 10);
        assert_eq!(answer.error(), Some((error::ARGUMENT, error::argument::UNKNOWN_OPCODE)));
    }

    #[test]
    fn an_entry_with_a_bad_envelope_registers_nothing() {
        // R04 where a driver will actually call it. `execute` runs *instead of*
        // a service's own executor for these two opcodes, so if the envelope
        // were only checked there, a peer could set a reserved word and an
        // undefined flag on a registration and be handed a set id anyway.
        let mut table = Table::<8>::new();
        let mut frame = Pinned::at(0x4000);

        let mut reserved = registration(1, 0, 256, 4);
        reserved._reserved = 0xDEAD_BEEF;
        let answer = table.execute(&reserved, &mut frame, 0);
        assert_eq!(answer.error(), Some((error::ARGUMENT, error::argument::RESERVED_NOT_ZERO)));
        assert_eq!(answer.ext, 0xDEAD_BEEF);

        let mut flagged = registration(2, 0, 256, 4);
        flagged.flags |= 1 << 7;
        let answer = table.execute(&flagged, &mut frame, 0);
        assert_eq!(answer.error(), Some((error::ARGUMENT, error::argument::UNKNOWN_FLAG)));
        assert_eq!(answer.ext, 1 << 7);

        assert_eq!(table.live(), 0, "neither entry registered anything");
        assert_eq!(frame.mapped, 0, "and neither asked the frame for a translation");
    }

    #[test]
    fn a_completion_carries_an_id_only_from_a_table_that_holds_the_set() {
        // `issued` takes `&self` because the table is the witness: an id it
        // does not hold earns the refusal `slot_of` gives, so a client reading
        // `SetId::from_completion` gets an error rather than a naming.
        let mut table = Table::<8>::new();
        let mut frame = Pinned::at(0x4000);
        let set = table.register(0, 256, 4, &mut frame).unwrap();

        let answer = table.issued(5, set, 1);
        assert_eq!(SetId::from_completion(&answer), Ok(set));

        // An id nobody issued, and an id this table has retired.
        let invented = SetId::new(7, 3);
        assert_eq!(
            table.issued(5, invented, 1).error(),
            Some((error::AUTHORITY, error::authority::NO_SUCH_CAP))
        );
        table.unregister(set, &mut frame).unwrap();
        assert_eq!(
            table.issued(5, set, 1).error(),
            Some((error::AUTHORITY, error::authority::REVOKED))
        );
    }
}
