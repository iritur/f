// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A channel a component may drive without writing `unsafe`: adopted once,
//! bound for the length of one call, and disbelieved for everything else.
//!
//! # The wall this is on the far side of
//!
//! `E0-B13` recorded it, `E1-B05` recorded it again, and RFC 0033 narrowed it
//! by exactly one half. A component inherits `unsafe_code = "forbid"`, so it
//! cannot dereference anything; [`Mapping::adopt`](crate::Mapping::adopt) is
//! `unsafe`; therefore a supervisor could not drive its own control ring and
//! RFC 0008's restart policy stayed in the frame. RFC 0033 answered the
//! *device* half — a granted register window and a granted DMA region become
//! safe accessors — and deliberately refused to answer this one, because
//! **a device window has one writer on this side and a channel is shared with
//! a peer that may be hostile.** That is still true. What follows is not
//! `Window`'s argument used a second time; it is a different argument, and
//! RFC 0037 is where it is made in full.
//!
//! # What is actually unsafe about adopting a channel
//!
//! Not the validation. [`Mapping::adopt`](crate::Mapping::adopt) already copies
//! the header out with one volatile read, negotiates against it, and refuses a
//! [`Layout`] whose offsets are not the ones this build computes — all of that
//! in safe code over this stack's own 64 bytes. A component could run every one
//! of those checks itself and be no less correct.
//!
//! Two obligations are left, and they are different in kind:
//!
//! 1. **The region is mapped.** A contract, and the same one
//!    [`Region::at`](crate::Region::at) and
//!    [`f_abi::state::Reader::at`] already keep: `base` names `len` bytes the
//!    frame mapped for this component in answer to a capability it holds. It
//!    is not sound by Rust's rules and saying so is the point — a component
//!    that invents an address takes a page fault at ring 3, which is the
//!    defined machine outcome `cargo xtask user` is seven boots of.
//! 2. **Nothing else holds a reference into the range.** This is the one a
//!    window does not have, and it is the reason this module exists rather
//!    than a `Mapping::at`. A [`Mapping`](crate::Mapping) hands out
//!    `&Cursor`, `&[AtomicU32]` and `&[UnsafeCell<Sqe>]` — real references,
//!    borrowed from `&self`, which Rust requires to point at live memory for
//!    their whole lifetime. A component holding one across the moment its
//!    supervisor revokes the channel holds a reference into unmapped memory,
//!    and that is undefined behaviour rather than a fault. A `Window` hands
//!    out nothing: every access is a fresh volatile operation, so a revoked
//!    window is a page fault at the next touch.
//!
//! # The decision: a channel is adopted for a call, not for a lifetime
//!
//! [`Adopted`] is four words — a base, the [`Layout`] the header was believed
//! at once, the [`Negotiated`] set, and the peer's epoch. It holds no
//! reference into the shared region and hands none out. Every operation builds
//! a [`Mapping`](crate::Mapping) over the stored layout, uses it, and drops it
//! before returning, so **no borrow of the peer's memory outlives one call**.
//! That is what makes obligation 2 discharged by construction rather than
//! promised, and it is what reduces obligation 1 to exactly the contract
//! RFC 0033 already accepted for a window: between two calls there is nothing
//! to dangle, and inside one there is a page fault if the mapping went.
//!
//! # Believing once, and re-checking always
//!
//! The layout is read from the peer's header **once**, at [`Adopted::at`], and
//! never again. That is the answer to the hostile peer and it is the opposite
//! of what per-call re-adoption would do: a peer that rewrote the header
//! between two calls could otherwise move the entry array under a component
//! that is midway through a drain. What is re-checked on every access is
//! everything a peer can still move — the cursors, the slot numbers in the
//! index ring, the occupancy — and that is [`Producer`](crate::Producer),
//! [`Consumer`](crate::Consumer), [`Poster`](crate::Poster) and
//! [`Collector`](crate::Collector) doing what they already did. Binding is the
//! point past which the arithmetic is known to describe the bytes; it is not
//! the point past which the bytes are known to be friendly, and this type
//! changes neither half of that sentence.
//!
//! # Two roles, because one end is not both
//!
//! [`Client`] submits and reaps; [`Server`] drains and answers. The split is
//! the single-producer single-consumer discipline the whole protocol rests on,
//! made a type rather than a paragraph — the same reason
//! [`Service`](crate::Service) holds one side of a channel and not both.
//! A component that genuinely is both ends of one region — a runtime whose
//! executor drains its own task queue — adopts it twice and says so, which is
//! exactly what [`Mapping`](crate::Mapping)'s own safety note already permits:
//! *two ends sharing a region is the intended use, not a violation of it*.

use f_abi::layout::Layout;
use f_abi::{Cqe, Negotiated, Sqe, error};

use crate::{Collector, Consumer, Mapping, Poster, Producer, RingError};

/// A channel bound once, holding no reference into it.
///
/// `Copy`, and deliberately: it is four words describing a region the frame
/// mapped, it owns nothing, and a component that had to thread one `&mut`
/// through its polling loop would end up with a borrow graph rather than a
/// runtime. The same reasoning [`Window`](crate::Window) records.
#[derive(Clone, Copy, Debug)]
pub struct Adopted {
    /// Where this component sees the region. Unit: bytes, in this component's
    /// address space.
    base: u64,
    /// How long it is. Carried because the arena's length is taken from the
    /// mapping and never from the header. Unit: bytes.
    len: u32,
    /// The layout the header was believed at, once.
    layout: Layout,
    /// What the two sides agreed to speak.
    agreed: Negotiated,
    /// The peer's epoch at the moment of binding. Unit: restarts of the writing
    /// peer.
    epoch: u32,
}

impl Adopted {
    /// Adopt a channel the frame mapped for this component.
    ///
    /// # Errors
    ///
    /// Everything [`Mapping::adopt`](crate::Mapping::adopt) refuses, plus
    /// `ARGUMENT/BAD_ADDRESS` for an address no mapping can be stated against —
    /// zero, a zero length, or a base this machine cannot hold in a pointer.
    /// Never a panic: every field behind this is a value a peer wrote, so
    /// refusing is the ordinary path.
    ///
    /// **Not among them: a `len` larger than what the frame granted.** It is
    /// the caller's claim about its own address space and there is nothing here
    /// to reconcile it against, so a component that over-states it gets a
    /// [`Layout`] bounding bytes it does not own and a page fault at the first
    /// access past the grant — not a refusal. That is the same contract
    /// [`Window::at`](crate::Window::at) keeps under RFC 0033 and it is stated
    /// here, on the call that owes it, rather than only in the module comment:
    /// the obligation a caller is most likely to get wrong belongs where the
    /// caller is looking.
    ///
    /// # Why this is safe to call
    ///
    /// The module comment is the argument and RFC 0037 is the argument in full.
    /// The short version: the two obligations
    /// [`Mapping::adopt`](crate::Mapping::adopt) carries are *the region is
    /// mapped*, which is a contract the frame keeps and whose failure mode is
    /// a page fault, and *nothing else references the range*, which is
    /// discharged structurally because the [`Mapping`](crate::Mapping) built
    /// here is dropped before this function returns and no accessor on this
    /// type outlives its own call.
    ///
    /// The contract, stated so a caller can fail to keep it deliberately:
    /// `base` names `len` bytes the frame mapped for this component in answer
    /// to a capability it holds. There is no check for this and there cannot
    /// be one — a component that could tell whether an address was granted
    /// would be a component with a page walk.
    pub fn at(base: u64, len: u32, offers: u64, requires: u64) -> Result<Self, i32> {
        let pointer = addressable(base, len)?;
        // SAFETY: the contract above supplies the first obligation — `base`
        // names `len` mapped bytes — and the second is discharged by this
        // function's shape rather than by a promise: `bound` is the only
        // `Mapping` over these bytes that exists here, every reference it hands
        // out is borrowed from it, and it is dropped at the end of this
        // statement. Nothing derived from it escapes.
        let bound = unsafe { Mapping::adopt(pointer, len, offers, requires) }?;
        Ok(Self {
            base,
            len,
            layout: bound.layout(),
            agreed: bound.negotiated(),
            epoch: bound.epoch(),
        })
    }

    /// What the two sides agreed to speak.
    #[must_use]
    pub const fn negotiated(&self) -> Negotiated {
        self.agreed
    }

    /// The layout this channel was bound at, and the one it keeps.
    #[must_use]
    pub const fn layout(&self) -> Layout {
        self.layout
    }

    /// The peer's epoch at the moment of binding.
    ///
    /// Carried rather than re-read, for [`Mapping::epoch`](crate::Mapping::epoch)'s
    /// reason: a channel is bound to one epoch, and a peer that restarts
    /// produces a different channel whose outstanding tokens are all stale.
    /// Unit: restarts of the writing peer.
    #[must_use]
    pub const fn epoch(&self) -> u32 {
        self.epoch
    }

    /// How many bytes the region holds.
    ///
    /// Carried because the arena's length is taken from the mapping and never
    /// from the header — the one number a peer does not get to choose.
    /// Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> u32 {
        self.len
    }

    /// Is the region empty? It never is: [`Adopted::at`] refuses a zero length.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// This end submits and reaps.
    #[must_use]
    pub const fn client(self) -> Client {
        Client(self)
    }

    /// This end drains and answers.
    #[must_use]
    pub const fn server(self) -> Server {
        Server(self)
    }

    /// Build the mapping for the length of one call.
    ///
    /// Private, and it is the whole mechanism: the value it returns borrows the
    /// shared region, so it may never be handed to a caller. Every method below
    /// takes one, uses it, and lets it go.
    fn bind(&self) -> Mapping {
        // SAFETY: the layout, negotiation and epoch are what `at` computed for
        // these very bytes through `Mapping::adopt`, which is `bound`'s
        // obligation; `base` and `len` are unchanged since. The region being
        // mapped is `at`'s contract, restated here because it is the one thing
        // this type cannot check. Nothing else holds a reference into the range:
        // the only `Mapping` that ever exists over it is this one, and it dies
        // with the call that made it.
        unsafe { Mapping::bound(self.base as *mut u8, self.layout, self.agreed, self.epoch) }
    }
}

/// The submitting end of a channel: it writes entries and reaps completions.
///
/// One `Adopted` may answer both this and [`Server`], and a component that
/// takes both for one region is claiming to be both ends of it. That is legal
/// and it is what a runtime's own task queue is; it is not what a component
/// does with a channel a *peer* holds the far end of.
#[derive(Clone, Copy, Debug)]
pub struct Client(Adopted);

impl Client {
    /// The channel underneath.
    #[must_use]
    pub const fn channel(&self) -> Adopted {
        self.0
    }

    /// Publish one submission.
    ///
    /// Answers whether the consumer asked to be woken, exactly as
    /// [`Producer::submit`](crate::Producer::submit) does.
    ///
    /// # Errors
    ///
    /// [`RingError`], every variant of which is a condition a peer can cause.
    pub fn submit(&self, entry: Sqe) -> Result<bool, RingError> {
        let mapping = self.0.bind();
        let producer = Producer::new(mapping.channel()).ok_or(RingError::Corrupt)?;
        producer.submit(entry)
    }

    /// Entries this end has queued and the far end has not taken.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] for a peer cursor that is impossible.
    pub fn queued(&self) -> Result<u32, RingError> {
        let mapping = self.0.bind();
        let producer = Producer::new(mapping.channel()).ok_or(RingError::Corrupt)?;
        producer.occupancy()
    }

    /// Take one completion, or `None` when there is none.
    ///
    /// **This is the polling point.** Every event a component receives is a
    /// completion entry drained here — R05, and RFC 0008 — and there is no
    /// second path in. A notice is a completion carrying
    /// [`f_abi::cflags::NOTICE`], which [`f_abi::control::is_notice`] is the
    /// one place that question is asked.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] for a service cursor that is impossible.
    pub fn take(&self) -> Result<Option<Cqe>, RingError> {
        let mapping = self.0.bind();
        let collector = Collector::new(mapping.completions()).ok_or(RingError::Corrupt)?;
        collector.take()
    }
}

/// The serving end of a channel: it drains submissions and posts completions.
#[derive(Clone, Copy, Debug)]
pub struct Server(Adopted);

impl Server {
    /// The channel underneath.
    #[must_use]
    pub const fn channel(&self) -> Adopted {
        self.0
    }

    /// Take one submission, or `None` when there is none.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] for a producer cursor or a slot number that is
    /// impossible. Both are bounds-checked on every entry, because both are
    /// values a peer wrote.
    pub fn pop(&self) -> Result<Option<Sqe>, RingError> {
        let mapping = self.0.bind();
        let consumer = Consumer::new(mapping.channel()).ok_or(RingError::Corrupt)?;
        consumer.pop()
    }

    /// Room left to answer in.
    ///
    /// Asked *before* a submission is taken, not after: an entry popped and
    /// then not completed is a caller waiting forever for a reply that was
    /// dropped on the floor.
    ///
    /// # Errors
    ///
    /// [`RingError::Corrupt`] for a peer cursor that is impossible.
    pub fn free(&self) -> Result<u32, RingError> {
        let mapping = self.0.bind();
        let poster = Poster::new(mapping.completions()).ok_or(RingError::Corrupt)?;
        poster.free()
    }

    /// Answer one submission.
    ///
    /// # Errors
    ///
    /// [`RingError::Full`] when there is no room, which is why [`Self::free`]
    /// is asked first.
    pub fn post(&self, entry: Cqe) -> Result<(), RingError> {
        let mapping = self.0.bind();
        let poster = Poster::new(mapping.completions()).ok_or(RingError::Corrupt)?;
        poster.post(entry)
    }
}

/// Refuse a base address no channel can be stated against.
///
/// The alignment and the header's own room are
/// [`Mapping::adopt`](crate::Mapping::adopt)'s to refuse and it does; what is
/// here is the part that has to happen before a pointer exists at all.
fn addressable(base: u64, len: u32) -> Result<*mut u8, i32> {
    let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
    if base == 0 || len == 0 {
        return Err(bad);
    }
    if usize::try_from(base).is_err() {
        return Err(bad);
    }
    Ok(base as *mut u8)
}

#[cfg(test)]
mod tests {
    use f_abi::{ChannelHeader, cflags, control};

    use super::*;

    /// A region a host test owns, aligned the way a frame is.
    ///
    /// The frame's contract cannot be kept on a host — there is no frame — so a
    /// test keeps it by owning the bytes outright, which is the same exemption
    /// `f_ring::device`'s tests take and for the same reason.
    #[repr(C, align(4096))]
    struct Page([u8; 4096]);

    impl Page {
        const fn new() -> Self {
            Self([0; 4096])
        }

        fn base(&mut self) -> u64 {
            core::ptr::from_mut::<Self>(self).cast::<u8>() as u64
        }
    }

    /// Describe a channel into a page the way the frame does before it hands
    /// one over.
    fn described(page: &mut Page, entries: u32) -> u64 {
        let base = page.base();
        // SAFETY: `page` is 4096 zeroed bytes at a 4096-byte alignment, which is
        // stronger than the cache line `describe` needs, and nothing else holds
        // a pointer into it for the length of this call.
        let mapping = unsafe { Mapping::describe(base as *mut u8, 4096, entries, 0, 0, 0) };
        assert!(mapping.is_ok(), "the frame could not describe a channel it owns");
        base
    }

    /// The whole point: a component adopts, submits, drains, and holds nothing
    /// across a call.
    #[test]
    fn a_component_drives_both_ends_without_unsafe() {
        let mut page = Page::new();
        let base = described(&mut page, 8);

        let client = Adopted::at(base, 4096, 0, 0).expect("adopt the client end").client();
        let server = Adopted::at(base, 4096, 0, 0).expect("adopt the server end").server();

        for index in 0..4u64 {
            let entry = Sqe { user_data: index, ..Sqe::ZERO };
            assert!(client.submit(entry).is_ok());
        }
        assert_eq!(client.queued(), Ok(4));

        let mut drained = 0;
        while let Ok(Some(entry)) = server.pop() {
            assert!(server.free().is_ok_and(|room| room > 0));
            assert!(server.post(crate::completion(entry.user_data, 0, 0)).is_ok());
            drained += 1;
        }
        assert_eq!(drained, 4);

        let mut reaped = 0;
        while let Ok(Some(cqe)) = client.take() {
            assert_eq!(cqe.user_data, reaped);
            reaped += 1;
        }
        assert_eq!(reaped, 4);
    }

    /// A notice is a completion entry and is drained at the same point as
    /// everything else. There is no second path in.
    #[test]
    fn a_notice_arrives_at_the_polling_point() {
        let mut page = Page::new();
        let base = described(&mut page, 4);

        let frame = Adopted::at(base, 4096, 0, 0).expect("the frame's end").server();
        let component = Adopted::at(base, 4096, 0, 0).expect("the component's end").client();

        let posted = control::entry(control::notice::RECLAIM, 0, 7, 0);
        assert!(frame.post(posted).is_ok());

        let taken = component.take().expect("no corruption").expect("one notice");
        assert!(control::is_notice(&taken));
        assert_eq!(taken.flags & cflags::NOTICE, cflags::NOTICE);
        assert_eq!(taken.result, control::notice::RECLAIM);
        assert_eq!(taken.ext, 7);
    }

    /// Fail closed. A header a peer scribbled is refused rather than believed,
    /// and the refusal names its domain.
    #[test]
    fn a_scribbled_header_is_refused_and_not_believed() {
        let mut page = Page::new();
        let base = described(&mut page, 8);
        assert!(Adopted::at(base, 4096, 0, 0).is_ok());

        // The peer rewrites the first word of the header, which is what
        // `component::demonstrate` does to provoke a fault at boot.
        let header = core::ptr::from_mut::<Page>(&mut page).cast::<ChannelHeader>();
        // SAFETY: `page` is 4096 aligned bytes this test owns, which is stronger
        // than `ChannelHeader`'s 64, and no reference into it is live here.
        unsafe { (header.cast::<u64>()).write_volatile(!f_abi::CHANNEL_MAGIC) };

        let refused = Adopted::at(base, 4096, 0, 0).expect_err("a scribbled header must refuse");
        assert_eq!(
            error::unpack(refused),
            Some((error::ARGUMENT, error::argument::MALFORMED_HEADER))
        );
    }

    /// An address no mapping can be stated against is refused before a pointer
    /// exists, rather than dereferenced to find out.
    #[test]
    fn an_impossible_address_is_refused_before_it_is_used() {
        for (base, len) in [(0u64, 4096u32), (0x1000, 0)] {
            let refused = Adopted::at(base, len, 0, 0).expect_err("must refuse");
            assert_eq!(
                error::unpack(refused),
                Some((error::ARGUMENT, error::argument::BAD_ADDRESS))
            );
        }
    }

    /// A channel whose peer requires a feature this end does not offer is
    /// refused at adoption, which is the one refusal a control ring depends on:
    /// a control ring whose peer cannot speak notices is not a control ring.
    #[test]
    fn a_feature_the_far_end_cannot_speak_refuses() {
        let mut page = Page::new();
        let base = page.base();
        // SAFETY: 4096 zeroed bytes this test owns, aligned past what `describe`
        // needs, with no other pointer into them live.
        let described = unsafe {
            Mapping::describe(
                base as *mut u8,
                4096,
                8,
                0,
                f_abi::feature::CONTROL_EVENTS,
                f_abi::feature::CONTROL_EVENTS,
            )
        };
        assert!(described.is_ok());

        let refused = Adopted::at(base, 4096, 0, 0).expect_err("must refuse");
        assert_eq!(error::unpack(refused), Some((error::PEER, error::peer::FEATURE_REQUIRED)));
        assert!(Adopted::at(base, 4096, f_abi::feature::CONTROL_EVENTS, 0).is_ok());
    }
}
