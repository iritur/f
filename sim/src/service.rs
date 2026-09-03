// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The service's side of a buffer name, driven through the real types.
//!
//! # Why there is no simulated registration
//!
//! RFC 0034 is the decision this file is the service half of. RFC 0024 is the
//! one underneath it, and it puts buffer ownership in two halves:
//! `f_ring::buffers` is the client's, checked by the compiler, and
//! `f_ring::registry` is the service's,
//! checked at runtime because the far end of a ring is bound by nothing but its
//! own checks. A simulator that wrote a *third* half would be checking that its
//! own copy agrees with itself — which is the failure RFC 0032 rejected shape
//! (b) to avoid, arrived at from the other side. So every peer in this crate
//! holds a real [`Table`], answers a real [`Cqe`], and gets the generation
//! arithmetic, the `lent` bitmap and the speculation mask for free.
//!
//! What that buys, concretely, is that the model refuses the things the system
//! refuses: a set id no table issued, an index past the set, a length past the
//! buffer, and **a buffer the device already holds** — the double submission,
//! which is the one an ordinary harness cannot see and a seeded reordering
//! finds.
//!
//! # The frame under it is a model, and it is the smallest one that refuses
//!
//! [`Grants`] is `f_ring::registry::Domains`, which is the IOMMU seam
//! (`E1-B01`). Here it is a bump allocator over an address space nothing
//! dereferences: a registration takes a range, an unregistration gives it back,
//! and a domain with no room refuses with `RESOURCE`/`QUOTA_EXHAUSTED`. Two
//! things make that worth having rather than a stub returning zero.
//!
//! It **refuses**, so the client-visible half of RFC 0032's *allocation failure*
//! fault class already has a path — the component asked for memory and was told
//! no — and `E1-P02` inherits a place to aim rather than a branch to add.
//!
//! It **decodes**, so a device model can ask whether a descriptor's address is
//! one this domain translates. A device that accepted any address would be a
//! device outside the protection `dma.rs` exists to demonstrate, and a model of
//! one would pass for the wrong reason — the same failure `dma.rs` records
//! about the legacy transport, one layer up.

use f_abi::buf::{Name, SetId};
use f_abi::{Cqe, Sqe, error};
use f_ring::registry::{Domains, Reach, Refusal, Table};

/// Registrations one peer's table holds.
///
/// Eight, and a power of two because [`Table`] requires one so its slot index is
/// masked rather than clamped. Eight is more than any scenario here needs and
/// small enough that a scenario can exhaust it on purpose, which is the only
/// reason to pick a number at all.
pub const SLOTS: usize = 8;

/// Where the modelled IOMMU starts handing out device addresses. Unit: bytes,
/// in the device's address space.
///
/// Far from the queue and control regions the device models place themselves at,
/// so that an address decode which confused the two would answer wrongly rather
/// than plausibly. A model whose wrong answers look right is a model that hides
/// the bug it was built to find.
pub const GRANT_BASE: u64 = 0x4000_0000;

/// How far apart two grants are placed. Unit: bytes.
///
/// One mebibyte, which is larger than any set a scenario registers, so a range
/// walking off the end of its grant lands in nothing rather than in the next
/// one. The alternative — packing grants — would make an overrun *reach* memory
/// the device was legitimately given, which is precisely the corruption the
/// IOMMU exists to prevent and precisely what a model must not quietly permit.
pub const GRANT_STRIDE: u64 = 0x0010_0000;

/// One translation the modelled IOMMU holds.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Translation {
    /// Which capability the component named. Unit: capability-table slots.
    cap: u32,
    /// Where the device addresses it. Unit: bytes, device space.
    address: u64,
    /// How much of it. Unit: bytes.
    len: u32,
}

/// The frame's IOMMU, as much of it as a protocol model needs.
///
/// `E1-B01` is the real one; this is the interface it implements, with a bump
/// allocator behind it. See the module documentation for why a stub would not
/// do.
#[derive(Clone, Debug)]
pub struct Grants {
    live: Vec<Translation>,
    /// How many translations this domain will hold at once. Unit: translations.
    room: u32,
    /// How many grants have been made, ever. Unit: translations — the bump, and
    /// deliberately never reused: an address that named one set and then names
    /// another is the failure a generation exists to prevent, one layer down.
    made: u64,
    /// The next translation this domain is asked for is refused.
    ///
    /// `E1-P02`'s *allocation failure*, injected here rather than answered
    /// beside the table, so that the refusal a client reads is the one
    /// `f_ring::registry::Table::register` builds when a domain declines. A
    /// fabricated completion would be a model of a refusal rather than the
    /// refusal, and the property worth asserting — that a refused registration
    /// leaves no slot and no generation spent — is the real table's rather than
    /// this file's. `fault.rs` is where the class is argued.
    starved: bool,
}

impl Grants {
    /// A domain with room for `room` translations.
    #[must_use]
    pub fn new(room: u32) -> Self {
        Self { live: Vec::new(), room: room.max(1), made: 0, starved: false }
    }

    /// Refuse the next translation this domain is asked for.
    ///
    /// One-shot, and consumed by the next [`Domains::map`] rather than cleared
    /// on a timer or at the end of an operation: the class is *this allocation
    /// failed*, and a flag that outlived one call would be a domain that had
    /// been turned off, which is a different and much blunter thing.
    ///
    /// Armed and disarmed around **one** registration by the caller, because the
    /// call it is aimed at may never reach the domain at all:
    /// `f_ring::registry::Table::register` refuses a malformed geometry, an
    /// out-of-range buffer count and a full slot table before it asks `map` for
    /// anything. A flag left armed after one of those would refuse the *next*
    /// operation's translation instead — a strike written into the trace against
    /// one token taking effect on another, which is exactly the attribution
    /// `E1-P03` reads out of a failing run. [`Grants::relent`] is the other half
    /// and `Device::submit` is the caller that owns the pair.
    pub const fn starve(&mut self) {
        self.starved = true;
    }

    /// Disarm a [`Grants::starve`] that was never consumed.
    ///
    /// Idempotent, and a no-op on the ordinary path where the registration did
    /// reach the domain and spent the flag there.
    pub const fn relent(&mut self) {
        self.starved = false;
    }

    /// Does the device reach `len` bytes at `at`?
    ///
    /// The model's stand-in for the remapping unit's answer. A device model asks
    /// this of every descriptor address it did not place itself, and a `false`
    /// is the refusal that stands in for the fault `dma.rs` provokes.
    #[must_use]
    pub fn reaches(&self, at: u64, len: u32) -> bool {
        self.live.iter().any(|t| {
            let end = t.address.saturating_add(u64::from(t.len));
            at >= t.address && at.saturating_add(u64::from(len)) <= end
        })
    }

    /// Translations currently held. Unit: translations.
    #[must_use]
    pub fn live(&self) -> usize {
        self.live.len()
    }
}

impl Domains for Grants {
    fn map(&mut self, cap: u32, len: u32) -> Result<u64, Refusal> {
        if core::mem::replace(&mut self.starved, false) {
            // The injected refusal, and it is the *same* refusal a full domain
            // gives: same code, same detail, same path back through the table.
            // A distinct code would let a client tell an injected failure from a
            // real one, which would make every assertion about the response an
            // assertion about the harness.
            return Err((
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                u64::from(self.room),
            ));
        }
        if u32::try_from(self.live.len()).unwrap_or(u32::MAX) >= self.room {
            return Err((
                error::pack(error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
                u64::from(self.room),
            ));
        }
        if len == 0 || u64::from(len) > GRANT_STRIDE {
            // A grant larger than the stride would overlap the next one, which
            // would make an overrun reach memory the device was given. Refused
            // rather than placed further out, because a domain whose geometry
            // depends on what it was asked for is a domain nothing can reason
            // about. `ARGUMENT/BAD_ADDRESS`, the same code the registry gives a
            // geometry that is not a set.
            return Err((
                error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS),
                u64::from(len),
            ));
        }
        let address = GRANT_BASE.wrapping_add(self.made.wrapping_mul(GRANT_STRIDE));
        self.made = self.made.wrapping_add(1);
        self.live.push(Translation { cap, address, len });
        Ok(address)
    }

    fn unmap(&mut self, cap: u32, address: u64, len: u32) {
        // Answers nothing and cannot refuse, which is the trait's rule and its
        // reason: teardown that can fail is teardown a peer can decline. An
        // entry that is not there is dropped silently *here* and refused by the
        // table above, which is where the bookkeeping error is visible.
        self.live.retain(|t| !(t.cap == cap && t.address == address && t.len == len));
    }
}

/// A peer's registration state: the table, the domain under it, and the two
/// operations every peer in this crate performs on them.
///
/// Shared by the modelled devices and by the native peer, so that *the
/// substitution changes the device and not the protocol*. If each peer carried
/// its own registration code, the substitution test below would be comparing two
/// implementations rather than one client against two services.
/// Not `Clone` and not `Debug`: `f_ring::registry::Table` is neither, and it is
/// right not to be. A registration table that could be copied would be two
/// tables issuing one set of identifiers, and a printable one would put a
/// component's whole grant list in a log line.
pub struct Service {
    table: Table<SLOTS>,
    grants: Grants,
}

impl Service {
    /// A service with an empty table over a domain holding `room`
    /// translations.
    #[must_use]
    pub fn new(room: u32) -> Self {
        Self { table: Table::new(), grants: Grants::new(room) }
    }

    /// The modelled IOMMU, for a device model that has to decode an address.
    #[must_use]
    pub const fn grants(&self) -> &Grants {
        &self.grants
    }

    /// Refuse the next translation this peer's domain is asked for.
    ///
    /// The peer's door to [`Grants::starve`], so that a device model injects
    /// `E1-P02`'s allocation failure without reaching into the domain itself —
    /// the same reason [`Service::register`] exists rather than the peers
    /// driving `Table` directly.
    pub const fn starve(&mut self) {
        self.grants.starve();
    }

    /// Disarm a [`Service::starve`] the registration never reached.
    ///
    /// The peer's door to [`Grants::relent`]. Called unconditionally after the
    /// registration this starve was armed for, so that the one-shot is one-shot
    /// for *that* registration rather than for the next `map` that happens
    /// along — `Grants::starve` states the failure this closes.
    pub const fn relent(&mut self) {
        self.grants.relent();
    }

    /// Registrations this peer holds. Unit: buffer sets.
    #[must_use]
    pub fn registered(&self) -> usize {
        self.table.live()
    }

    /// Execute a registration or an unregistration and answer the completion it
    /// earns.
    ///
    /// The real `Table::execute`, so the id in the completion is one this table
    /// issued and `f_ring::buffers::Fixed::from_completion` accepts it — which
    /// is the whole of what `E1-B10` added to that type and the reason the
    /// client cannot mint a naming out of the air.
    pub fn register(&mut self, entry: &Sqe, now: u64) -> Cqe {
        self.table.execute(entry, &mut self.grants, now)
    }

    /// Resolve the buffer one entry names, and record that the device holds it.
    ///
    /// # Errors
    ///
    /// Whatever [`Name::read`] or `Table::resolve` refuses with, unchanged. A
    /// refusal the model invented a code for would be a refusal the client
    /// cannot act on, which is R07 said backwards.
    pub fn resolve(&mut self, entry: &Sqe) -> Result<(SetId, u32, Reach), Refusal> {
        // Zero features: this crate's clients bind `Fixed`, whose `REQUIRES` is
        // zero, so an entry naming a virtual address is refused here with
        // `ARGUMENT/FEATURE_NOT_NEGOTIATED`. That is the refusal a channel that
        // did not agree shared virtual memory gives, and it runs in this crate
        // because a client that used the wrong naming should hear it from the
        // service rather than from a model that shrugged.
        match Name::read(entry, 0)? {
            Name::Registered { set, index } => {
                let reach = self.table.resolve(set, index, entry.len)?;
                Ok((set, index, reach))
            }
            Name::Virtual { address } => Err((
                error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
                address,
            )),
        }
    }

    /// The device is finished with a buffer.
    ///
    /// # Errors
    ///
    /// `ARGUMENT/BAD_ADDRESS` for a buffer this table did not have out, which
    /// is *this peer* completing something twice rather than a client
    /// misbehaving — refused so that the second completion is visible instead
    /// of quietly making a live buffer look free.
    pub fn release(&mut self, set: SetId, index: u32) -> Result<(), Refusal> {
        self.table.release(set, index)
    }

    /// The client restarted, or this peer did: every set it holds is stale.
    ///
    /// Answers how many registrations were retired. Unit: buffer sets. The
    /// translations go with them, which is what makes
    /// [`InFlight::reclaim`](f_ring::buffers::InFlight::reclaim) sound: a
    /// transfer the device had started faults instead of landing in memory
    /// somebody is about to reuse.
    pub fn retire_all(&mut self) -> usize {
        self.table.retire_all(&mut self.grants)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use f_ring::registry::registration;

    fn set_of(cqe: &Cqe) -> SetId {
        SetId::from_completion(cqe).expect("a table's own answer to a registration")
    }

    #[test]
    fn a_registration_goes_through_the_real_table_and_the_real_domain() {
        let mut service = Service::new(4);
        let cqe = service.register(&registration(1, 0, 512, 4), 100);
        assert!(!cqe.is_error(), "a legal registration was refused: {:?}", cqe.error());
        assert_eq!(cqe.timestamp, 100, "the service did not stamp the completion");
        assert_eq!(service.registered(), 1);
        assert_eq!(service.grants().live(), 1);
    }

    #[test]
    fn a_domain_with_no_room_refuses_and_the_table_keeps_no_half_registration() {
        // The client-visible half of RFC 0032's *allocation failure* class, and
        // the reason `Grants` refuses rather than always answering an address.
        // `Table::register` asks the domain before it fills the slot, so a
        // refusal must leave nothing behind — checked here rather than assumed,
        // because a half-registration is a slot and a generation spent for
        // nothing.
        let mut service = Service::new(2);
        for token in 0..2 {
            assert!(!service.register(&registration(token, 0, 256, 2), 0).is_error());
        }
        let refused = service.register(&registration(2, 0, 256, 2), 0);
        assert_eq!(
            refused.error(),
            Some((error::RESOURCE, error::resource::QUOTA_EXHAUSTED)),
            "a full domain refused with something other than a quota"
        );
        assert_eq!(service.registered(), 2, "a refused registration filled a slot");
    }

    #[test]
    fn a_starve_the_registration_never_reached_does_not_refuse_the_next_one() {
        // `E1-P02`'s allocation failure is armed at the domain and the
        // registration is what consumes it — but `Table::register` refuses a
        // geometry that is not a set *before* it asks the domain for anything,
        // and a full slot table the same way. A flag left armed after one of
        // those would refuse the next operation's translation instead: a fault
        // written into the trace against one token and suffered by another,
        // which is precisely the attribution `E1-P03` reads out of a failing
        // run. `Device::submit` arms and disarms around one call and this is
        // what that pair is for.
        let mut service = Service::new(4);
        service.starve();
        let never_asked = service.register(&registration(0, 0, 512, 0), 0);
        assert!(never_asked.is_error(), "a registration with no buffers was accepted");
        assert_eq!(service.grants().live(), 0, "a refused registration held a translation");
        service.relent();

        let next = service.register(&registration(1, 0, 512, 4), 0);
        assert!(
            !next.is_error(),
            "a starve aimed at a registration the domain never saw refused the next one: {:?}",
            next.error()
        );
        assert_eq!(service.grants().live(), 1);
    }

    #[test]
    fn one_buffer_cannot_be_lent_twice() {
        // The refusal an ordinary harness never sees and a seeded reordering
        // finds. It is the `lent` bitmap in `f_ring::registry::Table`, which
        // this crate gets by driving the real type rather than modelling one.
        let mut service = Service::new(4);
        let set = set_of(&service.register(&registration(0, 0, 256, 4), 0));

        let mut entry = Sqe { len: 64, ..Sqe::ZERO };
        Name::Registered { set, index: 1 }.write(&mut entry);

        let (_, index, reach) = service.resolve(&entry).expect("a buffer the table holds");
        assert_eq!(index, 1);
        assert_eq!(reach.len, 64);
        assert_eq!(
            service.resolve(&entry).err().map(|(packed, _)| error::unpack(packed)),
            Some(Some((error::ARGUMENT, error::argument::BAD_ADDRESS))),
            "the same buffer was resolved twice"
        );

        service.release(set, index).expect("a buffer that was out");
        assert!(service.resolve(&entry).is_ok(), "a released buffer could not be lent again");
    }

    #[test]
    fn a_grant_is_reached_only_inside_itself() {
        // What a device model asks of every descriptor address it did not place.
        // The boundaries matter more than the middle: an off-by-one here would
        // be a model that lets a device touch the byte after its grant.
        let mut service = Service::new(4);
        let first = set_of(&service.register(&registration(0, 0, 256, 4), 0));
        let mut entry = Sqe { len: 64, ..Sqe::ZERO };
        Name::Registered { set: first, index: 0 }.write(&mut entry);
        let (_, _, reach) = service.resolve(&entry).expect("a buffer the table holds");

        assert!(service.grants().reaches(reach.address, 64));
        assert!(service.grants().reaches(reach.address, 256), "the whole registration is reached");
        assert!(!service.grants().reaches(reach.address, 257), "one byte past the grant");
        assert!(!service.grants().reaches(reach.address - 1, 1), "one byte before it");
        assert!(!service.grants().reaches(GRANT_BASE + GRANT_STRIDE, 1), "the next grant's slot");
    }

    #[test]
    fn retiring_takes_the_translations_with_the_registrations() {
        // The property `InFlight::reclaim` rests on: when a peer is gone its
        // sets are retired *and* the device's translations go, so a transfer it
        // had started faults rather than landing.
        let mut service = Service::new(4);
        for token in 0..3 {
            assert!(!service.register(&registration(token, 0, 256, 2), 0).is_error());
        }
        assert_eq!(service.grants().live(), 3);
        assert_eq!(service.retire_all(), 3);
        assert_eq!(service.registered(), 0);
        assert_eq!(service.grants().live(), 0, "a translation outlived its registration");
    }
}
