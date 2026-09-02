// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Buffer ownership: a buffer is held by one side at a time, and the compiler
//! is what says which.
//!
//! # The bug this module exists to make unwritable
//!
//! Every completion-based I/O interface has the same worst failure: the caller
//! hands the device a buffer, loses interest — drops a future, returns early,
//! reuses the memory — and the device writes into whatever lives there now.
//! Reference counting hides it. `ring-scene-boot` section 04 says the type
//! system can eliminate it instead, and this is that: an [`Idle`] buffer is
//! **moved** into a submission and comes back only from the completion that
//! answers it. While it is [`InFlight`] there is no method that yields its
//! bytes, so there is nothing to write through.
//!
//! Four misuses are compile errors, one per fixture below. Writing an in-flight
//! buffer has no method to call. Naming memory the set was not bound over has
//! no type to pass, because an [`Idle`] cannot be built and the only ones that
//! exist are pieces of the set's own region. Carving a set twice is a second
//! mutable borrow of a set the first carve borrowed for the life of its
//! buffers, so one name is minted once. Submitting a buffer twice is a use
//! after move. `docs/rfc/0024-a-buffer-is-owned-by-one-side.md` names these,
//! and also the misuses that stay runtime refusals and which [`f_abi::error`]
//! domain each earns — because a type can only bind the code that uses it, and
//! the service at the far end of a ring is bound by nothing but its own checks.
//!
//! What the types here do **not** establish is that the service ever issued the
//! [`SetId`] the set was bound with: at E1 nothing issues one, so
//! [`BufferSet::bind`] takes it on trust and the service's registration table
//! is what refuses an id it never gave out. `E1-B10` is where ids start coming
//! from a registration, and it is the task that closes that gap.
//!
//! # Two paths, one set of rules
//!
//! How the buffer is *named* on the wire is a type parameter, [`Naming`], with
//! two implementations. [`Fixed`] is the registered path: the set was
//! registered with the service once, and the entry carries a set id and an
//! index. [`Virtual`] is the shared-virtual-memory path: the device walks the
//! submitter's page tables through the IOMMU, so the entry carries the
//! buffer's own address and nothing was registered. The second is behind
//! [`feature::SHARED_VIRTUAL_MEMORY`], and [`BufferSet::bind`] refuses it on a
//! channel that did not negotiate the bit.
//!
//! Everything else is shared. The same [`Idle`] and [`InFlight`] types, the
//! same move on submission and return on completion, and the same test body —
//! `ownership_holds_on` in this module's tests, which takes the naming as an
//! argument for the reason `doorbell` gives about its suppression test: two
//! tests written from one description drift the first time somebody edits one
//! of them. `E1-B10` measures the two paths; this is what makes the measurement
//! a comparison of one thing.
//!
//! # What a lifetime buys, and what it does not
//!
//! A set owns the region it was bound over, for `'m`, and [`BufferSet::carve`]
//! borrows the set for the same `'m` — which is why it can only be called once
//! and why the region cannot be reused while any buffer carved from it is
//! alive. So the only memory a set can name is the memory it was bound over,
//! and a buffer cannot outlive the set that names it. What the borrow cannot do
//! is stop an [`InFlight`] being *dropped*, because this language has no linear
//! types. A dropped `InFlight` is the one misuse left, and it is answered by a
//! drop bomb: the component that dropped a buffer the device still holds
//! panics, and under this workspace's `panic = "abort"` that is the component
//! ending — at which point RFC 0008 revokes its buffer sets and tears down its
//! IOMMU domain, so the transfer faults rather than lands. The frame is the
//! graveyard section 04 asks for. The RFC says what would turn this into a
//! cancellation instead.
//!
//! # The legal path
//!
//! ```
//! use f_abi::buf::SetId;
//! use f_abi::{Negotiated, Sqe, ABI_VERSION};
//! use f_ring::buffers::{BufferSet, Fixed, Submitter};
//! use f_ring::RingError;
//!
//! // Stands in for a `Producer` or a `Batch`, both of which implement
//! // `Submitter`; the ownership rules do not care what is on the far end.
//! struct Wire;
//! impl Submitter for Wire {
//!     fn submit(&mut self, _: Sqe) -> Result<bool, RingError> { Ok(false) }
//! }
//!
//! let agreed = Negotiated { version: ABI_VERSION, features: 0 };
//! let mut region = [0u8; 128];
//! let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed, &mut region).unwrap();
//! let [mut a, _b] = set.carve::<2>().unwrap();
//!
//! a.bytes_mut()[0] = 0x5A;                 // idle: ours to write
//! let mut entry = Sqe::ZERO;
//! entry.user_data = 7;
//! entry.len = 64;
//! let (lent, _doorbell) = a.submit(&mut Wire, entry).unwrap();
//!
//! // ... the service works, and eventually answers with our token ...
//! let cqe = f_ring::completion(7, 64, 0);
//! let a = lent.complete(&cqe).unwrap();    // ours again
//! assert_eq!(a.bytes()[0], 0x5A);
//! ```
//!
//! # The four fixtures
//!
//! Writing an in-flight buffer. There is no `bytes_mut` on [`InFlight`], or
//! `bytes`, or anything else that reaches the memory:
//!
//! ```compile_fail,E0599
//! # use f_abi::buf::SetId;
//! # use f_abi::{Negotiated, Sqe, ABI_VERSION};
//! # use f_ring::buffers::{BufferSet, Fixed, Submitter};
//! # use f_ring::RingError;
//! # struct Wire;
//! # impl Submitter for Wire {
//! #     fn submit(&mut self, _: Sqe) -> Result<bool, RingError> { Ok(false) }
//! # }
//! # let agreed = Negotiated { version: ABI_VERSION, features: 0 };
//! # let mut region = [0u8; 128];
//! # let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed, &mut region).unwrap();
//! let [a, _b] = set.carve::<2>().unwrap();
//! let (mut lent, _) = a.submit(&mut Wire, Sqe::ZERO).unwrap();
//! lent.bytes_mut()[0] = 1;                 // the device holds this buffer
//! ```
//!
//! Naming memory the set was not bound over. An [`Idle`] cannot be built by
//! hand, so a slice that did not come out of [`BufferSet::carve`] — and so was
//! not part of the region the set covers — has no way into a submission:
//!
//! ```compile_fail,E0451
//! # use f_abi::buf::SetId;
//! # use f_abi::{Negotiated, ABI_VERSION};
//! # use f_ring::buffers::{BufferSet, Fixed, Idle};
//! # let agreed = Negotiated { version: ABI_VERSION, features: 0 };
//! # let mut region = [0u8; 128];
//! # let set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed, &mut region).unwrap();
//! let mut somewhere = [0u8; 64];
//! let forged = Idle { naming: set.naming(), bytes: &mut somewhere[..], index: 0 };
//! ```
//!
//! Carving a set twice, which would mint two buffers with one name and put the
//! same registered buffer in flight twice. The first carve borrows the set for
//! as long as its buffers live, which is as long as the set:
//!
//! ```compile_fail,E0499
//! # use f_abi::buf::SetId;
//! # use f_abi::{Negotiated, ABI_VERSION};
//! # use f_ring::buffers::{BufferSet, Fixed};
//! # let agreed = Negotiated { version: ABI_VERSION, features: 0 };
//! # let mut region = [0u8; 128];
//! # let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed, &mut region).unwrap();
//! let [a, _b] = set.carve::<2>().unwrap();
//! let [c, _d] = set.carve::<2>().unwrap();   // `a` and `c` would both be buffer 0
//! ```
//!
//! Submitting the same buffer twice. The first submission moved it:
//!
//! ```compile_fail,E0382
//! # use f_abi::buf::SetId;
//! # use f_abi::{Negotiated, Sqe, ABI_VERSION};
//! # use f_ring::buffers::{BufferSet, Fixed, Submitter};
//! # use f_ring::RingError;
//! # struct Wire;
//! # impl Submitter for Wire {
//! #     fn submit(&mut self, _: Sqe) -> Result<bool, RingError> { Ok(false) }
//! # }
//! # let agreed = Negotiated { version: ABI_VERSION, features: 0 };
//! # let mut region = [0u8; 128];
//! # let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed, &mut region).unwrap();
//! let [a, _b] = set.carve::<2>().unwrap();
//! let (first, _) = a.submit(&mut Wire, Sqe::ZERO).unwrap();
//! let (second, _) = a.submit(&mut Wire, Sqe::ZERO).unwrap();
//! ```

use f_abi::buf::{Name, SetId};
use f_abi::{Cqe, Negotiated, Sqe, cflags, error, feature};

use crate::{Batch, Producer, RingError};

/// How a buffer is named on the wire.
///
/// The only thing the two paths differ in. Everything about *who holds the
/// buffer* is the same on both, which is the property `E1-B10`'s comparison
/// rests on and the reason this is a trait rather than two modules.
pub trait Naming {
    /// Feature bits the channel must have negotiated for this naming to be
    /// legal on it. Checked once, at [`BufferSet::bind`]; an entry that used
    /// a naming the channel did not agree to would be refused by the service
    /// with `ARGUMENT`/`FEATURE_NOT_NEGOTIATED`, and refusing it here is the
    /// same refusal a round trip earlier.
    const REQUIRES: u64;

    /// The name the entry for buffer `index`, whose bytes are `bytes`, should
    /// carry.
    fn name(&self, index: u32, bytes: &[u8]) -> Name;
}

/// The registered path: the set was registered with the service, and an entry
/// names a buffer by set id and index. No address crosses the boundary.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Fixed(pub SetId);

impl Naming for Fixed {
    const REQUIRES: u64 = 0;

    fn name(&self, index: u32, _bytes: &[u8]) -> Name {
        Name::Registered { set: self.0, index }
    }
}

/// The shared-virtual-memory path: the device sees the submitter's own
/// addresses through the IOMMU, so an entry names a buffer by where it is.
///
/// Nothing was registered, and the [`BufferSet`] this names for is a ledger of
/// who holds what rather than a record of a registration — which is exactly
/// the part section 04 says must not disappear when registration does. An
/// address on the wire is also the one thing on this path a seeded run has to
/// be careful about: it is deterministic only if the allocation behind it is,
/// and a trace that contains entries will show the difference.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Virtual;

impl Naming for Virtual {
    const REQUIRES: u64 = feature::SHARED_VIRTUAL_MEMORY;

    fn name(&self, _index: u32, bytes: &[u8]) -> Name {
        Name::Virtual { address: bytes.as_ptr() as usize as u64 }
    }
}

/// Something a named entry can be handed to.
///
/// [`Producer`] and [`Batch`] both are, and the ownership types take either,
/// so a buffer can go out in a batch without a second `submit` that would have
/// to repeat the argument above. A test can stand a recorder in here and
/// exercise the ownership rules with no ring at all, which is what the module
/// documentation's fixtures do.
pub trait Submitter {
    /// Hand over one entry. The answer is what [`Producer::submit`] answers:
    /// whether the consumer asked to be rung. A [`Batch`] answers `false`,
    /// because its doorbell decision arrives at [`Batch::publish`].
    ///
    /// # Errors
    ///
    /// Whatever the ring refuses with. [`RingError::Full`] is the ordinary
    /// one, and it hands the buffer back — see [`Idle::submit`].
    fn submit(&mut self, entry: Sqe) -> Result<bool, RingError>;
}

impl Submitter for Producer<'_> {
    fn submit(&mut self, entry: Sqe) -> Result<bool, RingError> {
        Producer::submit(self, entry)
    }
}

impl Submitter for Batch<'_, '_> {
    fn submit(&mut self, entry: Sqe) -> Result<bool, RingError> {
        self.push(entry)?;
        Ok(false)
    }
}

/// Why a submission was handed back with its buffer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Refused {
    /// The ring refused it. `Full` is the one a caller retries.
    Ring(RingError),
    /// The caller got the entry wrong, and this side noticed before the wire
    /// did. The service would have refused the same entry with
    /// `ARGUMENT`/`BAD_ADDRESS`; catching it here saves the round trip and
    /// names the mistake in the caller's own terms.
    Misuse(Misuse),
}

/// A mistake on this side of the wire, refused before it reaches it.
///
/// These are the ownership rules a type cannot carry — lengths, chiefly — and
/// so they are checked. Every variant is a caller's bug and none of them is a
/// peer's, which is why they are a Rust enum and not a packed error: nothing
/// here goes into a completion.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Misuse {
    /// The entry asked for more bytes than the buffer has. An operation names
    /// a whole buffer or a prefix of it; a sub-range is a smaller buffer.
    Length {
        /// What the entry's `len` asked for. Unit: bytes.
        asked: u32,
        /// What the buffer holds. Unit: bytes.
        have: usize,
    },
    /// The region does not divide into that many equal, non-empty buffers.
    Geometry {
        /// The region's length. Unit: bytes.
        region: usize,
        /// The buffer count asked for. Unit: buffers.
        buffers: usize,
    },
}

/// One buffer set on one channel: the region, the naming, and nothing else.
///
/// The set holds the memory it names. That is what makes "a buffer this set
/// names is memory this set covers" a statement about types rather than about
/// discipline: [`BufferSet::carve`] is the only source of an [`Idle`], and it
/// divides the region the set was bound over. It borrows the set for as long as
/// those buffers live, so it can be called once — two carves would be two
/// buffers with one name, and on the registered path that is the double
/// submission the service would have to catch on the wire.
///
/// What the set does *not* establish is that the [`SetId`] it was bound with
/// names a registration the service ever made. Nothing issues one until
/// `E1-B10`; see [`BufferSet::bind`].
#[derive(Debug)]
pub struct BufferSet<'m, N: Naming> {
    naming: N,
    region: &'m mut [u8],
}

impl<'m, N: Naming> BufferSet<'m, N> {
    /// Bind a naming to `region` on a channel that negotiated `agreed`.
    ///
    /// The region is the whole of what this set can ever name. On the
    /// registered path it is the memory whose registration answered with the
    /// [`SetId`] in `naming` — which this call takes on trust, because at E1
    /// nothing issues one: the registration entry and the table that would
    /// refuse an invented id are `E1-B10`'s, and until they exist an id a
    /// client made up reaches the service and is refused there
    /// (`AUTHORITY`/`NO_SUCH_CAP`) rather than here.
    ///
    /// # Errors
    ///
    /// A packed [`error`] result: `PEER`/[`FEATURE_REQUIRED`](error::peer::FEATURE_REQUIRED)
    /// when the naming needs a feature the channel did not agree — the same
    /// refusal [`ChannelHeader::negotiate`](f_abi::ChannelHeader::negotiate)
    /// gives a peer that requires what the other side does not offer, because
    /// it is the same situation one layer up. Refused rather than downgraded:
    /// a set that asked for shared virtual memory and quietly got registration
    /// would be two peers with different beliefs about what an entry names.
    pub fn bind(naming: N, agreed: Negotiated, region: &'m mut [u8]) -> Result<Self, i32> {
        if agreed.features & N::REQUIRES != N::REQUIRES {
            return Err(error::pack(error::PEER, error::peer::FEATURE_REQUIRED));
        }
        Ok(Self { naming, region })
    }

    /// The naming this set puts on the wire.
    pub const fn naming(&self) -> &N {
        &self.naming
    }

    /// Bytes the set covers. Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.region.len()
    }

    /// Never — [`BufferSet::bind`] would have to have been given an empty
    /// region, and [`BufferSet::carve`] refuses to divide one — but the pair is
    /// conventional and the lint asks for it.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.region.is_empty()
    }

    /// Divide the set's region into `B` equal buffers, all idle.
    ///
    /// The one way an [`Idle`] comes into existence, and the reason a buffer
    /// cannot name memory outside its set. Borrows the set for as long as any
    /// of the buffers lives, which is what makes a second carve a compile
    /// error rather than a second name for buffer zero.
    ///
    /// # Errors
    ///
    /// [`Misuse::Geometry`] when the region does not divide into `B` equal
    /// buffers of at least one byte, or `B` does not fit the index field.
    pub fn carve<const B: usize>(&'m mut self) -> Result<[Idle<'m, N>; B], Misuse> {
        let geometry = Misuse::Geometry { region: self.region.len(), buffers: B };
        if B == 0 || u32::try_from(B).is_err() {
            return Err(geometry);
        }
        let size = self.region.len() / B;
        if size == 0 || !self.region.len().is_multiple_of(B) {
            return Err(geometry);
        }

        let naming = &self.naming;
        let mut chunks = self.region.chunks_exact_mut(size);
        Ok(core::array::from_fn(|index| {
            // Exactly `B` chunks: the length is a multiple of `size` and
            // `size * B` is the length, both established above. Not a check on
            // anything a peer wrote — the region and the count are the
            // caller's own — so an expectation rather than a refusal.
            let bytes = chunks.next().expect("a region divides into the buffers it was checked to");
            // Fits: `B` fits a `u32` and `index < B`.
            Idle { naming, bytes, index: index as u32 }
        }))
    }
}

/// A buffer this side holds. The only state in which its bytes can be reached.
///
/// Created by [`BufferSet::carve`] and by nothing else, out of the region its
/// set was bound over. Moved into [`Idle::submit`], and returned by
/// [`InFlight::complete`] when the service has answered.
#[must_use = "an idle buffer that is dropped is a slot in the set nobody can name again"]
pub struct Idle<'m, N: Naming> {
    naming: &'m N,
    bytes: &'m mut [u8],
    index: u32,
}

impl<N: Naming> core::fmt::Debug for Idle<'_, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Idle").field("index", &self.index).field("len", &self.bytes.len()).finish()
    }
}

impl<'m, N: Naming> Idle<'m, N> {
    /// Which buffer of the set. Unit: buffers, zero-based.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Bytes in the buffer. Unit: bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Never, by construction — [`BufferSet::carve`] refuses a zero-size
    /// buffer — but the pair is conventional and the lint asks for it.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Read the buffer.
    #[must_use]
    pub const fn bytes(&self) -> &[u8] {
        &*self.bytes
    }

    /// Write the buffer. Exists on this type and on no other in the module,
    /// which is the whole mechanism.
    #[must_use]
    pub const fn bytes_mut(&mut self) -> &mut [u8] {
        &mut *self.bytes
    }

    /// Hand the buffer to the service.
    ///
    /// The buffer's name is written over `entry`'s `buf_set`, `buf_index` and
    /// `FIXED_BUF` flag by this call — a caller cannot name a buffer by hand,
    /// which is what "a buffer names memory its set covers" means on this side.
    /// Everything else in the entry is the caller's.
    ///
    /// `entry.user_data` is the token the completion has to carry to get this
    /// buffer back, and it **must be unique among this channel's in-flight
    /// buffers**. That obligation is the caller's and neither the compiler nor
    /// the service can see it: two buffers lent on one token means the first
    /// completion returns whichever of them is asked first, which may be the
    /// one the device is still writing. RFC 0024 states it beside the drop
    /// bomb, as the second misuse that is neither a compile error nor a wire
    /// refusal; `two_buffers_on_one_token_return_the_wrong_one` is the fixture.
    ///
    /// On success the buffer is [`InFlight`] and its bytes are unreachable
    /// until [`InFlight::complete`]. The `bool` is the doorbell answer
    /// [`Submitter::submit`] gave.
    ///
    /// # Errors
    ///
    /// The refusal *and the buffer*, because a submission that did not happen
    /// did not move anything: a full ring is a retry, not a loss.
    /// [`Misuse::Length`] when `entry.len` exceeds the buffer.
    pub fn submit<S: Submitter>(
        self,
        lane: &mut S,
        mut entry: Sqe,
    ) -> Result<(InFlight<'m, N>, bool), (Refused, Self)> {
        if entry.len as usize > self.bytes.len() {
            let misuse = Misuse::Length { asked: entry.len, have: self.bytes.len() };
            return Err((Refused::Misuse(misuse), self));
        }

        self.naming.name(self.index, self.bytes).write(&mut entry);

        let index = self.index;
        match lane.submit(entry) {
            Ok(wanted) => {
                Ok((InFlight { token: entry.user_data, index, idle: Some(self) }, wanted))
            }
            Err(err) => Err((Refused::Ring(err), self)),
        }
    }
}

/// A buffer the service holds. Nothing here reaches its bytes.
///
/// Returned by [`Idle::submit`]; consumed by [`InFlight::complete`] when the
/// completion carrying its token arrives, or by [`InFlight::reclaim`] when
/// evidence arrives that it never will. Dropping one is the misuse the type
/// system cannot refuse, so the drop refuses it: see the module documentation.
#[must_use = "a buffer the device holds must be completed or reclaimed, never dropped"]
pub struct InFlight<'m, N: Naming> {
    /// `Some` from construction until one of the two consuming methods takes
    /// it. `None` only inside those methods, on the way out, which is what
    /// lets the drop bomb tell a consumed buffer from an abandoned one.
    idle: Option<Idle<'m, N>>,
    token: u64,
    /// Kept here rather than read out of `idle`, so that [`InFlight::index`]
    /// is total: it answers the same during the moment `idle` is empty, and
    /// `Debug` has no sentinel to print.
    index: u32,
}

impl<N: Naming> core::fmt::Debug for InFlight<'_, N> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("InFlight").field("index", &self.index).field("token", &self.token).finish()
    }
}

impl<'m, N: Naming> InFlight<'m, N> {
    /// The `user_data` the completion has to carry. Unit: none — the caller's
    /// own token, as it wrote it into the entry.
    #[must_use]
    pub const fn token(&self) -> u64 {
        self.token
    }

    /// Which buffer of the set. Unit: buffers, zero-based.
    #[must_use]
    pub const fn index(&self) -> u32 {
        self.index
    }

    /// Take the buffer back, if this completion is the one that returns it.
    ///
    /// A completion returns the buffer when it carries this buffer's token
    /// and is the last one that will: a refusal returns it, because the
    /// service is done with it; a cancellation returns it, for the same
    /// reason — RFC 0010 says cancellation is a flag and not an error, and
    /// nothing here treats it as either. What does *not* return it is a
    /// completion with [`cflags::MORE`], which promises another for the same
    /// token, or a completion for somebody else's token.
    ///
    /// The token is the whole of the test, so a token lent to two buffers at
    /// once returns the wrong one — see [`Idle::submit`] for whose obligation
    /// that is.
    ///
    /// # Errors
    ///
    /// The buffer, still in flight, when `cqe` does not return it. Not a
    /// mistake: a client reaping a completion ring sees every token, and
    /// asking each in-flight buffer "is this yours?" is how it finds the one.
    pub fn complete(mut self, cqe: &Cqe) -> Result<Idle<'m, N>, Self> {
        if cqe.user_data != self.token || cqe.flags & cflags::MORE != 0 {
            return Err(self);
        }
        // Taken out, so the drop that follows the return sees `None` and stays
        // quiet. The expectation cannot fire: the buffer is put in at
        // construction and taken out only here and in `reclaim`, both of which
        // consume `self`.
        Ok(self.idle.take().expect("an in-flight buffer holds its bytes until it is consumed"))
    }

    /// Take the buffer back without a completion, because none is coming.
    ///
    /// The evidence is a [`PeerGone`], which can only be made from a condition
    /// meaning the peer's outstanding tokens are all void. Sound because of
    /// what RFC 0008 makes the frame do when a component ends: revoke its
    /// buffer sets and tear down its IOMMU domain, so a transfer the dead peer
    /// had started faults instead of landing in memory this side is about to
    /// reuse. That is `E1-B01`'s guarantee, and this method is the place that
    /// depends on it.
    pub fn reclaim(mut self, _gone: PeerGone) -> Idle<'m, N> {
        self.idle.take().expect("an in-flight buffer holds its bytes until it is consumed")
    }
}

impl<N: Naming> Drop for InFlight<'_, N> {
    fn drop(&mut self) {
        // The misuse the borrow checker cannot see. A component that reaches
        // here has a bug that would otherwise become a device writing into
        // memory it no longer owns; ending the component is what makes that
        // write fault instead. Not a peer's fault, so not a refusal — nothing
        // a peer wrote can bring a program here.
        assert!(
            self.idle.is_none(),
            "a buffer was dropped while the device held it; complete it or reclaim it"
        );
    }
}

/// Evidence that a peer's outstanding completions will never arrive.
///
/// Constructed only from a condition that means it — today,
/// [`RingError::EpochChanged`], the ring's own observation that the peer
/// restarted. The peer-gone notice RFC 0008 posts on a control ring is the
/// other source, and arrives with `E1-B05`. A zero-sized type rather than a
/// `bool` argument so that the caller cannot assert it.
#[derive(Clone, Copy, Debug)]
pub struct PeerGone(());

impl PeerGone {
    /// The evidence a ring error constitutes, if any.
    ///
    /// Only [`RingError::EpochChanged`] is evidence. `Full` is a retry and
    /// `Corrupt` is a teardown this side chose, and neither says anything
    /// about whether a transfer already accepted will finish.
    #[must_use]
    pub const fn of(err: RingError) -> Option<Self> {
        match err {
            RingError::EpochChanged => Some(Self(())),
            RingError::Full | RingError::Corrupt => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::Backing;
    use crate::{Collector, Consumer, Poster, completion, refusal};
    use f_abi::{ABI_VERSION, flags};

    fn agreed(features: u64) -> Negotiated {
        Negotiated { version: ABI_VERSION, features }
    }

    fn entry(token: u64, len: u32) -> Sqe {
        let mut e = Sqe::ZERO;
        e.user_data = token;
        e.len = len;
        e
    }

    /// One body, both paths. `E1-B10`'s exit is that *both paths pass the same
    /// ownership tests*, and the only way to be sure of that is for there to
    /// be one test that takes the naming as a parameter.
    ///
    /// Drives a real ring: the entry goes through a [`Producer`], is popped by
    /// a [`Consumer`] that reads its name the way a service would, and the
    /// completion comes back through a [`Poster`] and a [`Collector`].
    fn ownership_holds_on<'m, N: Naming>(
        set: &'m mut BufferSet<'m, N>,
        features: u64,
        expect: impl Fn(&Name),
    ) {
        let backing = Backing::<8>::new();
        let mut producer = Producer::new(backing.chan()).expect("a power-of-two ring");
        let consumer = Consumer::new(backing.chan()).expect("a power-of-two ring");
        let poster = Poster::new(backing.cq()).expect("a power-of-two ring");
        let collector = Collector::new(backing.cq()).expect("a power-of-two ring");

        let [mut a, mut b] = set.carve::<2>().expect("256 bytes is two buffers");
        assert_eq!((a.index(), b.index()), (0, 1));
        assert_eq!((a.len(), b.len()), (128, 128));

        // Idle: both are ours to write.
        a.bytes_mut().fill(0xA5);
        b.bytes_mut().fill(0x5A);

        // Submitted: `a` is gone from this scope, and the entry on the ring
        // carries the naming this set was bound with — not what the caller
        // wrote, which was nothing.
        let (a, wanted) = a.submit(&mut producer, entry(11, 128)).expect("room on the ring");
        assert!(!wanted, "a consumer that never armed is never rung");
        assert_eq!(a.token(), 11);
        assert_eq!(a.index(), 0);

        let on_wire = consumer.pop().expect("healthy").expect("one entry");
        assert_eq!(on_wire.user_data, 11);
        assert_eq!(on_wire.len, 128);
        let name = Name::read(&on_wire, features).expect("a name this channel can read");
        expect(&name);

        // `b` is untouched by any of this: still idle, still writable.
        b.bytes_mut()[0] = 0xFF;

        // A completion for somebody else's token does not return the buffer,
        // and neither does one that promises more.
        let a = a.complete(&completion(12, 0, 0)).expect_err("not our token");
        let mut more = completion(11, 64, 0);
        more.flags = cflags::MORE;
        let a = a.complete(&more).expect_err("more to come");

        // The real one, through the completion ring.
        poster.post(completion(11, 128, 5)).expect("room to answer");
        let cqe = collector.take().expect("healthy").expect("one completion");
        let mut a = a.complete(&cqe).expect("our token, final");

        // Idle again: ours, and what we wrote is still there — the service in
        // this test touched nothing, which is what a `Recorder` would show.
        assert_eq!(a.bytes()[0], 0xA5);
        a.bytes_mut()[0] = 0;

        // And the buffer can go round again, which is what "returns" means.
        let (a, _) = a.submit(&mut producer, entry(13, 8)).expect("room on the ring");
        let _ = consumer.pop().expect("healthy").expect("the second entry");
        let a = a.complete(&completion(13, 8, 0)).expect("returned again");
        drop(a);
        drop(b);
    }

    #[test]
    fn both_paths_pass_the_same_ownership_test() {
        let id = SetId::new(4, 2);
        let mut registered = [0u8; 256];
        let mut fixed = BufferSet::bind(Fixed(id), agreed(0), &mut registered)
            .expect("registration needs no feature");
        ownership_holds_on(&mut fixed, 0, |name| {
            assert_eq!(*name, Name::Registered { set: id, index: 0 });
        });

        let svm = feature::SHARED_VIRTUAL_MEMORY;
        let mut shared = [0u8; 256];
        let mut virt =
            BufferSet::bind(Virtual, agreed(svm), &mut shared).expect("the feature was negotiated");
        ownership_holds_on(&mut virt, svm, |name| {
            // The address is the buffer's own. Its value is the allocator's
            // business and is not asserted; that it is non-zero and that the
            // reading is the virtual one is.
            assert!(matches!(name, Name::Virtual { address } if *address != 0));
        });
    }

    #[test]
    fn the_virtual_path_is_refused_where_it_was_not_negotiated() {
        // RFC 0011 style: the feature bit gates the path, and a set that asked
        // for it on a channel without it is refused rather than downgraded.
        let mut first = [0u8; 64];
        let refused =
            BufferSet::bind(Virtual, agreed(0), &mut first).map(|_| ()).expect_err("no feature");
        assert_eq!(error::unpack(refused), Some((error::PEER, error::peer::FEATURE_REQUIRED)));

        // Registration needs nothing, so it binds on either kind of channel.
        let mut second = [0u8; 64];
        assert!(BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut second).is_ok());
        let mut third = [0u8; 64];
        let both = agreed(feature::SHARED_VIRTUAL_MEMORY);
        assert!(BufferSet::bind(Fixed(SetId::new(0, 1)), both, &mut third).is_ok());
    }

    #[test]
    fn the_fixed_naming_sets_the_flag_and_the_virtual_naming_clears_it() {
        let mut region = [0u8; 64];
        let mut set = BufferSet::bind(Fixed(SetId::new(1, 1)), agreed(0), &mut region).unwrap();
        let [a] = set.carve::<1>().unwrap();
        let mut wire = Recorder(None);
        let mut e = entry(1, 64);
        e.flags = flags::LINK;
        let (a, _) = a.submit(&mut wire, e).unwrap();
        let sent = wire.0.expect("one entry recorded");
        assert_eq!(sent.flags, flags::LINK | flags::FIXED_BUF, "the caller's flags survive");
        assert_eq!(sent.buf_set, SetId::new(1, 1).bits());
        assert_eq!(sent.buf_index, 0);
        let _ = a.complete(&completion(1, 0, 0)).unwrap();

        let mut region = [0u8; 64];
        let where_it_is = region.as_ptr() as usize as u64;
        let both = agreed(feature::SHARED_VIRTUAL_MEMORY);
        let mut set = BufferSet::bind(Virtual, both, &mut region).unwrap();
        let [a] = set.carve::<1>().unwrap();
        let mut e = entry(2, 64);
        e.flags = flags::FIXED_BUF; // a caller cannot name a buffer by hand
        let (a, _) = a.submit(&mut wire, e).unwrap();
        let sent = wire.0.expect("one entry recorded");
        assert_eq!(sent.flags & flags::FIXED_BUF, 0, "the naming decides the flag, not the caller");
        let address = u64::from(sent.buf_set) | (u64::from(sent.buf_index) << 32);
        assert_eq!(address, where_it_is, "the virtual name is the buffer's own address");
        let _ = a.complete(&completion(2, 0, 0)).unwrap();
    }

    /// A submitter that keeps the last entry, and can be told to be full.
    struct Recorder(Option<Sqe>);

    impl Submitter for Recorder {
        fn submit(&mut self, entry: Sqe) -> Result<bool, RingError> {
            self.0 = Some(entry);
            Ok(false)
        }
    }

    struct FullRing;

    impl Submitter for FullRing {
        fn submit(&mut self, _: Sqe) -> Result<bool, RingError> {
            Err(RingError::Full)
        }
    }

    #[test]
    fn a_refused_submission_hands_the_buffer_back() {
        let mut region = [0u8; 64];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        let [a] = set.carve::<1>().unwrap();

        // A full ring is a retry: the buffer comes back idle, unchanged.
        let (why, mut a) = a.submit(&mut FullRing, entry(1, 64)).expect_err("the ring is full");
        assert_eq!(why, Refused::Ring(RingError::Full));
        a.bytes_mut()[0] = 1;

        // And so is a length the buffer cannot hold — caught here, before the
        // service would have refused it with BAD_ADDRESS.
        let (why, a) = a.submit(&mut Recorder(None), entry(1, 65)).expect_err("too long");
        assert_eq!(why, Refused::Misuse(Misuse::Length { asked: 65, have: 64 }));
        assert_eq!(a.bytes()[0], 1);
    }

    /// A hundred bytes, carved into `B`. A set is carved once, so the geometry
    /// cases each need their own; this is that set, and the buffers die inside.
    fn carve_a_hundred_bytes_into<const B: usize>() -> Result<(), Misuse> {
        let mut region = [0u8; 100];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        set.carve::<B>().map(|_| ())
    }

    #[test]
    fn a_region_that_does_not_divide_is_refused() {
        let geometry = |buffers| Err(Misuse::Geometry { region: 100, buffers });
        assert_eq!(carve_a_hundred_bytes_into::<3>(), geometry(3));
        assert_eq!(carve_a_hundred_bytes_into::<0>(), geometry(0));
        // More buffers than bytes would be zero-length buffers, which nothing
        // could name.
        assert_eq!(carve_a_hundred_bytes_into::<200>(), geometry(200));
        assert!(carve_a_hundred_bytes_into::<4>().is_ok());
    }

    #[test]
    fn a_refusal_and_a_cancellation_both_return_the_buffer() {
        let mut region = [0u8; 128];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        let [a, b] = set.carve::<2>().unwrap();
        let mut wire = Recorder(None);

        // The service refused: it is done with the buffer.
        let (a, _) = a.submit(&mut wire, entry(1, 64)).unwrap();
        let bad = error::pack(error::ARGUMENT, error::argument::BAD_ADDRESS);
        let a = a.complete(&refusal(1, bad, 0, 0)).expect("a refusal returns the buffer");
        drop(a);

        // The service cancelled: RFC 0010 says that is a flag and not an
        // error, and the buffer is returned either way.
        let (b, _) = b.submit(&mut wire, entry(2, 64)).unwrap();
        let mut cancelled = completion(2, 0, 0);
        cancelled.flags = cflags::CANCELLED;
        let b = b.complete(&cancelled).expect("a cancellation returns the buffer");
        drop(b);
    }

    #[test]
    fn two_buffers_on_one_token_return_the_wrong_one() {
        // The obligation `Idle::submit` states, as a fixture: a token is the
        // whole of what a completion is matched on, so two buffers lent under
        // one token are indistinguishable, and the first one asked takes the
        // answer. Nothing here is a bug in this module — it is the misuse RFC
        // 0024 says only the caller's own bookkeeping can see, and this test
        // exists so that a reader can see it too rather than discover it.
        let mut region = [0u8; 128];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        let [a, b] = set.carve::<2>().unwrap();
        let mut wire = Recorder(None);

        let (a, _) = a.submit(&mut wire, entry(9, 64)).unwrap();
        let (b, _) = b.submit(&mut wire, entry(9, 64)).unwrap();
        assert_eq!((a.token(), b.token()), (9, 9), "one token, two buffers in flight");

        // The service answers for buffer 1, and buffer 0 takes it: the client
        // now writes memory the device may still be writing.
        let answer = completion(9, 64, 0);
        let mut a = a.complete(&answer).expect("the token matches, and that is all it checks");
        assert_eq!(a.index(), 0, "the completion was buffer 1's and buffer 0 answered to it");
        a.bytes_mut()[0] = 1;

        // And `b` has no completion left to take, so the only way to put it
        // down is the reclaim path — which is what makes this a bug and not a
        // deadlock: the ledger is wrong, not stuck.
        let gone = PeerGone::of(RingError::EpochChanged).expect("the peer restarted");
        drop(b.reclaim(gone));
    }

    #[test]
    fn a_batch_is_a_submitter_and_the_buffers_go_out_together() {
        let mut region = [0u8; 128];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        let backing = Backing::<8>::new();
        let mut producer = Producer::new(backing.chan()).unwrap();
        let consumer = Consumer::new(backing.chan()).unwrap();
        let [a, b] = set.carve::<2>().unwrap();

        let mut batch = producer.batch();
        let (a, wanted_a) = a.submit(&mut batch, entry(1, 64)).unwrap();
        let (b, wanted_b) = b.submit(&mut batch, entry(2, 64)).unwrap();
        assert!(!wanted_a && !wanted_b, "a batch answers the doorbell at publish");
        assert!(consumer.pop().unwrap().is_none(), "nothing is visible before publish");
        batch.publish().unwrap();

        let first = consumer.pop().unwrap().unwrap();
        let second = consumer.pop().unwrap().unwrap();
        assert_eq!((first.buf_index, second.buf_index), (0, 1));

        let _ = a.complete(&completion(1, 0, 0)).unwrap();
        let _ = b.complete(&completion(2, 0, 0)).unwrap();
    }

    #[test]
    fn a_lost_peer_is_the_only_way_back_without_a_completion() {
        let mut region = [0u8; 64];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        let [a] = set.carve::<1>().unwrap();
        let (a, _) = a.submit(&mut Recorder(None), entry(1, 64)).unwrap();

        assert!(PeerGone::of(RingError::Full).is_none(), "full is a retry");
        assert!(PeerGone::of(RingError::Corrupt).is_none(), "corrupt says nothing about DMA");
        let gone = PeerGone::of(RingError::EpochChanged).expect("the peer restarted");
        let mut a = a.reclaim(gone);
        a.bytes_mut()[0] = 1;
    }

    #[test]
    #[should_panic(expected = "dropped while the device held it")]
    fn dropping_an_in_flight_buffer_is_refused_at_the_drop() {
        let mut region = [0u8; 64];
        let mut set = BufferSet::bind(Fixed(SetId::new(0, 1)), agreed(0), &mut region).unwrap();
        let [a] = set.carve::<1>().unwrap();
        let (a, _) = a.submit(&mut Recorder(None), entry(1, 64)).unwrap();
        drop(a);
    }
}
