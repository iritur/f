// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The device's side of the boundary: memory the caller registered, and a tally
//! of every byte that went anywhere else.
//!
//! # Why a zero here is a count and not a construction
//!
//! `E2-B08`'s exit is *copies per read is zero, **counted rather than
//! asserted***, and `intent/0006-state/spec.md` says why the word matters: RFC
//! 0024's typestate is an argument by construction, and an argument by
//! construction publishes the same zero whether the property holds or the
//! mechanism behind it was deleted. So the number is a tally, it is taken twice
//! on opposite sides of the boundary, and the two readings are required to
//! agree — claim 0012's discipline, because a zero one component published is a
//! zero that component can produce by writing it.
//!
//! The two readings, named:
//!
//! - **The device model's**, here. [`Landed::unregistered_bytes`] moves inside
//!   [`stage`], which is the only function in this crate that moves a byte, and
//!   is reached only when the destination is memory nobody registered.
//! - **The driver's**, in [`crate::read`]. [`crate::read::Counters::staged_bytes`]
//!   moves when the *submission* named no registered buffer, which is a
//!   different observation of the same event: one is what the request said, the
//!   other is where the bytes went. A driver that named a registered set and
//!   then staged the bytes anyway moves one and not the other, and
//!   [`crate::read::ReadPath::readings_agree`] is what notices.
//!
//! And a counter nothing can move is not a counter, which is the sentence
//! `kernel/src/mem.rs` makes with `provoke_remote` and `user/virtio-blk` makes
//! one crate over: [`Landing::provoke_copy`] moves both on purpose, so a zero
//! recorded beside a non-zero is evidence rather than a default.
//!
//! # A destination is registered or it is not, and this module does not decide
//!
//! [`Table`] decides — `f_ring::registry`, which is E1-B10's service-side
//! registration and the memory a real driver would check a real submission
//! against. Nothing here re-implements it. What this module adds is the one
//! thing a host model has that a device does not: the region really exists as
//! bytes, so an address the table answers with can be turned back into the
//! slice the device writes into. A [`Reach`] is deliberately not a slice
//! (`ring/src/registry.rs` says why), and [`Landing::lend`] is the single place
//! in this crate where that conversion happens.
//!
//! # What is deliberately not here
//!
//! **A second way to reach the region.** [`Landing::lend`] hands out a `&mut
//! [u8]` only for a range the table resolved, and [`Landing::buffer`] hands out
//! a shared slice for a caller reading its own bytes back. There is no accessor
//! that takes a bare offset, because such an accessor is exactly how a
//! zero-copy count stops being about the caller's buffer.

use f_abi::buf::SetId;
use f_abi::store::refusal;
use f_ring::registry::{Domains, Refusal, Table};

/// Registration slots this component holds for one client.
///
/// Four, and a power of two because `f_ring::registry::Table` requires one —
/// the slot index is masked rather than clamped, RFC 0005. Four rather than
/// `f_virtio_blk::driver::SETS`'s sixteen because a reader holds one set per
/// geometry it reads at and this crate has one geometry; a client that wants
/// more is refused `RESOURCE/QUOTA_EXHAUSTED` rather than this side deciding
/// how much memory to commit on its behalf.
pub const SETS: usize = 4;

/// Where the modelled device addresses the client's region from.
///
/// Non-zero on purpose. Both of RFC 0024's paths refuse a null buffer address,
/// so a model whose region began at device address zero would make the refusal
/// unreachable and the check untestable.
/// Unit: bytes, in the device's address space.
const DEVICE_BASE: u64 = 0x1000_0000;

/// What the device model saw, in bytes.
///
/// The primary is [`Landed::unregistered_bytes`] and the other two are its
/// denominators: a zero beside a zero says nothing, which is the same sentence
/// `f_virtio_blk::driver::Counters` makes about `bytes` beside `copies`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Landed {
    /// Bytes the device wrote straight into a buffer the caller registered.
    ///
    /// The denominator. A read path that transferred nothing would publish
    /// `unregistered_bytes = 0` too, and this is what tells the two apart.
    /// Unit: bytes.
    pub registered_bytes: u64,
    /// Bytes that moved through a buffer that is not the caller's registered
    /// one.
    ///
    /// **Required to be zero on the read path**, and it is a tally rather than
    /// a property: [`stage`] is the only thing that moves it, so a build in
    /// which the read path had grown a staging step moves it. The module
    /// comment names the second reading it has to agree with.
    /// Unit: bytes.
    pub unregistered_bytes: u64,
    /// Transfers the device completed into a registered buffer.
    ///
    /// Unit: count of transfers. Present because *copies per read* is a ratio
    /// and a ratio needs a count of reads that is taken here rather than
    /// asserted by whoever divides.
    pub transfers: u64,
}

/// The client's memory as the device reaches it, and the table that says which
/// of it the device may reach.
///
/// Named for what it is: the place a transfer *lands*. It holds the region and
/// the registrations together because that pairing is the whole property —
/// a region with no table is memory anyone may write, and a table with no
/// region is a check with nothing under it.
pub struct Landing<'m> {
    /// The client's memory. Never handed out whole.
    region: &'m mut [u8],
    /// The registrations, in this component's own private memory and never in
    /// the shared region — RFC 0028, and `ring/src/registry.rs` gives the
    /// argument: a generation a peer can write is a peer that can un-revoke its
    /// own retired set.
    table: Table<SETS>,
    /// One buffer's width, recorded at the registration that fixed it rather
    /// than divided out again wherever it is wanted: the division happened once
    /// when the set was registered, and a second division is a second answer.
    /// Unit: bytes.
    stride: usize,
    /// What this device model saw.
    landed: Landed,
}

impl<'m> Landing<'m> {
    /// Register the whole of `region` as one set of `buffers` equal buffers, and
    /// answer the set's name.
    ///
    /// The registration goes through `f_ring::registry::Table::register`
    /// unchanged, which is the point: the geometry refusals a real client would
    /// meet — a region that does not divide evenly, more buffers than
    /// `BUFFERS_MAX`, no free slot — are that function's and are not restated
    /// here where they could drift.
    ///
    /// # Errors
    ///
    /// The refusal code `Table::register` gives, unpacked to the `i32` half:
    /// `ARGUMENT`/`BAD_ADDRESS` for a geometry that is not a set,
    /// `RESOURCE`/`QUOTA_EXHAUSTED` for more buffers than the table can hold.
    /// [`refusal::MALFORMED`] for a region larger than a `u32` of bytes, which
    /// is a registration no wire entry could name.
    pub fn open(region: &'m mut [u8], buffers: u32) -> Result<(Self, SetId), i32> {
        let len = u32::try_from(region.len()).map_err(|_| refusal::MALFORMED)?;
        let mut domains = Flat { next: DEVICE_BASE };
        let mut table = Table::<SETS>::new();
        // Capability one: this model has no capability space, and the number is
        // passed through to `Domains::map` untouched rather than invented at
        // the call site, so that the day a real capability arrives it arrives
        // in one place.
        let set = table.register(1, len, buffers, &mut domains).map_err(|(code, _)| code)?;
        // The geometry the registration just fixed. `register` has already
        // refused a region that does not divide, so this division is exact.
        let stride = region.len() / buffers as usize;
        Ok((Self { region, table, stride, landed: Landed::default() }, set))
    }

    /// What this device model saw.
    #[must_use]
    pub const fn landed(&self) -> &Landed {
        &self.landed
    }

    /// How many bytes one buffer of the set holds.
    ///
    /// Unit: bytes. A read whose record does not fit in one of these is refused
    /// [`refusal::SHORT_BUFFER`] before a block is read, rather than after.
    #[must_use]
    pub const fn stride(&self) -> usize {
        self.stride
    }

    /// Resolve one buffer through the table and hand the device exactly the
    /// bytes it may write.
    ///
    /// This is the one place a [`Reach`](f_ring::registry::Reach) — an address
    /// and a length, deliberately not a slice — becomes memory in this model.
    /// The tally is taken *here* and not by the caller, because a caller that
    /// counted its own transfers would be the component publishing its own
    /// zero.
    ///
    /// The buffer is marked lent until [`Landing::release`], which is the
    /// table's own double-submission check and not this crate's: a second
    /// `lend` of a buffer the device already holds is refused.
    ///
    /// # Errors
    ///
    /// `AUTHORITY`/`NO_SUCH_CAP` or `AUTHORITY`/`REVOKED` for a set id this
    /// table never issued or has retired; `ARGUMENT`/`BAD_ADDRESS` for an index
    /// past the set, a length past the buffer, or a buffer already lent —
    /// every one of them `Table::resolve`'s, unchanged. [`refusal::ADDRESS`]
    /// for an address the table answered that this model's region does not
    /// cover, which is a translation and a region that have stopped agreeing
    /// and is checked rather than assumed.
    pub fn lend(&mut self, set: SetId, index: u32, len: usize) -> Result<&mut [u8], i32> {
        let wanted = u32::try_from(len).map_err(|_| refusal::SHORT_BUFFER)?;
        let reach = self.table.resolve(set, index, wanted).map_err(|(code, _)| code)?;
        let at = usize::try_from(reach.address.wrapping_sub(DEVICE_BASE))
            .map_err(|_| refusal::ADDRESS)?;
        let end = at.checked_add(len).ok_or(refusal::ADDRESS)?;
        let window = self.region.get_mut(at..end).ok_or(refusal::ADDRESS)?;
        self.landed.registered_bytes = self.landed.registered_bytes.saturating_add(len as u64);
        self.landed.transfers = self.landed.transfers.saturating_add(1);
        Ok(window)
    }

    /// The device is finished with the buffer.
    ///
    /// # Errors
    ///
    /// `Table::release`'s, unchanged: a buffer this table did not have out is
    /// this side completing something twice, and it is refused so that the
    /// second completion is visible rather than quietly making a live buffer
    /// look free.
    pub fn release(&mut self, set: SetId, index: u32) -> Result<(), i32> {
        self.table.release(set, index).map_err(|(code, _)| code)
    }

    /// One buffer of the set, for the caller reading back its own bytes.
    ///
    /// Shared and not exclusive, and by index rather than by offset: a caller
    /// that could name a bare offset into the region could name somebody
    /// else's buffer, and the whole of what a registration is for is that it
    /// cannot.
    #[must_use]
    pub fn buffer(&self, index: u32) -> Option<&[u8]> {
        self.contents(index, 1)
    }

    /// A run of `buffers` consecutive buffers, as one slice.
    ///
    /// Contiguous by construction and not by luck: a set is registered over one
    /// range and its buffers are that range divided, so buffer `n` ends where
    /// buffer `n + 1` begins. That is what lets a record spread across several
    /// blocks be read by the caller without reassembling anything — the
    /// alternative would be a gather step, and a gather step over content is
    /// the copy this crate exists not to make.
    ///
    /// `None` when the run would leave the set, which is the check
    /// [`crate::read::ReadPath::read`] makes *before* it reads the second block
    /// rather than after.
    #[must_use]
    pub fn contents(&self, first: u32, buffers: u32) -> Option<&[u8]> {
        let at = usize::try_from(first).ok()?.checked_mul(self.stride)?;
        let len = usize::try_from(buffers).ok()?.checked_mul(self.stride)?;
        self.region.get(at..at.checked_add(len)?)
    }

    /// Move bytes into the caller's buffer that did **not** land there, and
    /// count them on this side of the boundary.
    ///
    /// # Why a read path with no copy in it has a function that copies
    ///
    /// Because a counter nothing can move is indistinguishable from a counter
    /// that does not work, and `copies_per_read = 0` published by a build whose
    /// tally had been deleted reads exactly like `copies_per_read = 0`
    /// published by a build that is zero-copy. So this is the page cache,
    /// modelled: bytes that landed in memory the component owns, moved on into
    /// the caller's buffer afterwards. It is the shape the exit's *no page cache
    /// second copy* is denying, and running it is how the denial becomes
    /// evidence.
    ///
    /// It moves **this** side's tally and nothing else. The driver's tally is
    /// moved by the driver, at its own submission, in
    /// [`crate::read::ReadPath::provoke_second_copy`] — two statements, two
    /// crates' worth of separate bookkeeping, one event. That separation is the
    /// whole reason the two readings agreeing is a check rather than a
    /// restatement of one number.
    ///
    /// # Errors
    ///
    /// [`refusal::ADDRESS`] for a buffer index the set does not have or a
    /// source longer than one buffer.
    pub fn provoke_copy(&mut self, from: &[u8], index: u32) -> Result<(), i32> {
        if from.len() > self.stride {
            return Err(refusal::ADDRESS);
        }
        let at = usize::try_from(index).ok().and_then(|i| i.checked_mul(self.stride));
        let at = at.ok_or(refusal::ADDRESS)?;
        let end = at.checked_add(from.len()).ok_or(refusal::ADDRESS)?;
        let into = self.region.get_mut(at..end).ok_or(refusal::ADDRESS)?;
        // Two disjoint fields of one struct, which the borrow checker allows
        // here and would not allow across a method call — and that is the
        // reason `stage` takes the tally as an argument rather than reaching
        // for it, exactly as `f_virtio_blk::driver::stage` does one crate over.
        stage(from, into, &mut self.landed.unregistered_bytes);
        Ok(())
    }
}

/// Move `from` into `into`, adding the bytes to `tally`.
///
/// **The only function in this crate that moves a byte**, and the tally is an
/// argument rather than a field so that *which* counter moved says which caller
/// ran. Nothing on the read path passes it a tally, because nothing on the read
/// path calls it at all: `cargo xtask lint-datapath` requires this to be defined
/// once and called once, from [`Landing::provoke_copy`], so a reader who wants
/// to disagree with *copies per read is zero* should start by searching this
/// crate for calls to it — and that search stays a search with one result
/// rather than being re-established by whoever next reads the file.
///
/// It cannot fail: the caller has already checked the two slices are the same
/// length, and a copy that returned a `Result` would put the check in two
/// places.
fn stage(from: &[u8], into: &mut [u8], tally: &mut u64) {
    into.copy_from_slice(from);
    *tally = tally.saturating_add(from.len() as u64);
}

/// The frame's IOMMU, modelled: successive translations from [`DEVICE_BASE`].
///
/// A model and not a stub. What a real `Domains` does is hand a device an
/// address for memory a capability names, and the only property this crate
/// depends on is that the address is stable and that unmapping it takes the
/// reach away — which is why `unmap` here is a no-op with a sentence rather
/// than a `todo!()`: nothing in this crate retires a set, and a teardown that
/// pretended to do work would be a second thing to keep true.
struct Flat {
    /// The next address to hand out. Unit: bytes, in the device's address space.
    next: u64,
}

impl Domains for Flat {
    fn map(&mut self, _cap: u32, len: u32) -> Result<u64, Refusal> {
        let address = self.next;
        self.next = self.next.saturating_add(u64::from(len));
        Ok(address)
    }

    fn unmap(&mut self, _cap: u32, _address: u64, _len: u32) {
        // Nothing to take away: this model's region outlives every set made
        // over it, and a model that pretended otherwise would be modelling a
        // teardown no test in this crate performs.
    }
}

#[cfg(test)]
mod tests {
    use super::{Landing, SETS, stage};
    use alloc::vec;
    use f_abi::buf::SetId;
    use f_abi::store::refusal;

    #[test]
    fn the_tally_moves_when_it_is_passed_and_not_when_it_is_not() {
        // The test that makes a zero worth reading, and it is `stage`'s own:
        // both directions, because a tally that only ever goes up is a tally
        // that cannot distinguish a call from a deletion.
        let from = [7u8; 16];
        let mut into = [0u8; 16];
        let mut moved = 0u64;
        let untouched = 0u64;

        stage(&from, &mut into, &mut moved);
        assert_eq!(into, from, "the bytes arrived");
        assert_eq!(moved, 16, "and the tally that was passed moved");
        assert_eq!(untouched, 0, "and the tally that was not passed did not");
    }

    #[test]
    fn a_lend_is_the_registration_and_not_an_offset() {
        let mut region = vec![0u8; 256];
        let (mut landing, set) = Landing::open(&mut region, 4).expect("a region that divides");

        // Inside the set: the window is the buffer the index names, and the
        // device tally moved by exactly what was lent.
        let window = landing.lend(set, 1, 64).expect("buffer one, whole");
        window[0] = 0xAB;
        assert_eq!(landing.landed().registered_bytes, 64);
        assert_eq!(landing.landed().unregistered_bytes, 0);
        landing.release(set, 1).expect("the device is finished with it");
        assert_eq!(landing.stride(), 64, "the geometry the registration fixed");
        assert_eq!(landing.buffer(1).expect("buffer one")[0], 0xAB);
        assert_eq!(landing.buffer(0).expect("buffer zero")[0], 0, "and nothing else moved");

        // Past the set, and a length past one buffer. Both are the table's
        // refusals rather than this crate's arithmetic, which is the point.
        assert!(landing.lend(set, 4, 64).is_err(), "an index past the set");
        assert!(landing.lend(set, 0, 65).is_err(), "a length past one buffer");
        // A name nobody issued.
        assert!(landing.lend(SetId::NULL, 0, 64).is_err(), "a set id that names nothing");
    }

    #[test]
    fn a_buffer_the_device_already_holds_is_refused_a_second_time() {
        let mut region = vec![0u8; 128];
        let (mut landing, set) = Landing::open(&mut region, 2).expect("a region that divides");
        let _ = landing.lend(set, 0, 64).expect("the first lend");
        assert!(landing.lend(set, 0, 64).is_err(), "the second, while the device holds it");
        landing.release(set, 0).expect("released");
        assert!(landing.lend(set, 0, 64).is_ok(), "and available again");
    }

    #[test]
    fn the_provocation_moves_this_sides_reading_and_delivers_the_bytes() {
        let mut region = vec![0u8; 128];
        let (mut landing, _set) = Landing::open(&mut region, 2).expect("a region that divides");
        let cached = [0x5Au8; 64];

        assert_eq!(landing.landed().unregistered_bytes, 0, "before");
        landing.provoke_copy(&cached, 1).expect("buffer one");
        assert_eq!(landing.landed().unregistered_bytes, 64, "the device model's reading moved");
        assert_eq!(landing.buffer(1).expect("buffer one"), &cached[..]);
        assert_eq!(landing.buffer(0).expect("buffer zero")[0], 0, "and only buffer one");
        // Past the set, and a source wider than one buffer: the two refusals a
        // provocation can earn, so that a run which meant to move bytes and
        // silently moved none is not mistaken for a zero-copy read.
        assert!(landing.provoke_copy(&cached, 2).is_err(), "a buffer the set does not have");
        assert!(landing.provoke_copy(&[0u8; 65], 0).is_err(), "a source wider than one buffer");
    }

    #[test]
    fn a_run_of_buffers_is_one_slice_and_stops_at_the_set() {
        let mut region = vec![0u8; 128];
        let (mut landing, set) = Landing::open(&mut region, 2).expect("a region that divides");
        let window = landing.lend(set, 1, 64).expect("buffer one");
        window[0] = 0xCD;
        landing.release(set, 1).expect("released");

        let run = landing.contents(0, 2).expect("both buffers");
        assert_eq!(run.len(), 128);
        assert_eq!(run[64], 0xCD, "buffer one begins where buffer zero ends");
        assert!(landing.contents(0, 3).is_none(), "a run that would leave the set");
        assert!(landing.contents(2, 1).is_none(), "a buffer past the set");
    }

    #[test]
    fn a_region_that_is_not_a_set_is_refused_at_registration() {
        let mut region = vec![0u8; 100];
        // A hundred bytes do not divide into three buffers, and the refusal is
        // `Table::register`'s rather than a check written here twice.
        assert!(Landing::open(&mut region, 3).is_err());
        let mut empty: [u8; 0] = [];
        assert!(Landing::open(&mut empty, 1).is_err(), "a region with no bytes is not a set");
        // The table's own capacity is the other direction, and it is a `const`
        // block rather than a runtime assertion because it is a fact about the
        // build: `Table` requires a power-of-two slot count so that a slot
        // index is masked rather than clamped, RFC 0005, and a `SETS` that
        // stopped being one would stop compiling here rather than in a message
        // about masking.
        const { assert!(SETS.is_power_of_two()) }
    }

    #[test]
    fn a_short_buffer_is_told_apart_from_a_bad_address() {
        let mut region = vec![0u8; 64];
        let (mut landing, set) = Landing::open(&mut region, 1).expect("one buffer");
        let huge = usize::try_from(u64::from(u32::MAX) + 1).expect("a host with 64-bit usize");
        assert_eq!(landing.lend(set, 0, huge), Err(refusal::SHORT_BUFFER));
    }
}
