// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Wire types that cross a trust boundary.
//!
//! This is the only crate whose memory layout is load-bearing against code we
//! do not control: an imported driver component, a WebAssembly component, or a
//! peer built by a different toolchain. Everything here is `#[repr(C)]`,
//! fixed-width, free of generics and free of `Drop`.
//!
//! Every change to this crate is an ABI change. Review it as one.
//!
//! See `docs/design/ring-scene-boot.html` section 05.

#![no_std]

/// The version this build speaks.
///
/// A peer is not required to report this exact value. Setup negotiates the
/// highest version both sides can speak, down to [`ABI_VERSION_MIN`]: see
/// [`ChannelHeader::negotiate`] and `docs/rfc/0011-peers-negotiate.md`.
pub const ABI_VERSION: u32 = 1;

/// The oldest version this build still speaks.
///
/// A promise about how far back the implementation actually goes, so raising it
/// is a decision about which peers get dropped — taken deliberately, in a
/// reviewable diff, never as a side effect of adding something.
pub const ABI_VERSION_MIN: u32 = 1;

/// Identifies a channel header so a corrupt or foreign mapping is detected
/// before anything in it is trusted.
pub const CHANNEL_MAGIC: u64 = 0x465f_4348_414e_0001;

/// Submission queue entry. Exactly one cache line.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct Sqe {
    /// Operation. The opcode space is per-service, not global.
    pub opcode: u8,
    /// See the `flags` module.
    pub flags: u8,
    /// Scheduling class and priority. See `docs/design/deadline-all-the-way-down.html`.
    pub class: u16,
    /// Index into the caller's capability table. A forged value fails the
    /// bounds check and kills the channel.
    pub cap: u32,
    /// Returned verbatim in the completion. Opaque to the service.
    pub user_data: u64,
    /// Absolute deadline, in **monotonic nanoseconds in this channel's epoch**:
    /// the clock `f_env::Instant` reports, which is the only clock in the system
    /// with ordering authority. [`NO_DEADLINE`] is zero.
    ///
    /// Lets a service order by deadline rather than by arrival, and carries a
    /// caller's deadline across a ring so that blocking on a service does not
    /// invert priority. The unit, the epoch and the zero are part of the ABI —
    /// see `docs/rfc/0009-three-clocks.md`, which exists because they were not.
    pub deadline: u64,
    /// Operation-specific position.
    pub offset: u64,
    /// Registered buffer set. On hardware with shared virtual memory this and
    /// `buf_index` collapse to a plain address in the submitter's own space.
    pub buf_set: u32,
    /// Index within the buffer set.
    pub buf_index: u32,
    /// Length in bytes.
    pub len: u32,
    /// Reserved. Must be zero.
    pub _reserved: u32,
    /// Operation-specific payload.
    pub ext: [u64; 2],
}

/// No deadline: the service may order this entry however it likes.
///
/// Distinct from a deadline already in the past, which is a late request of
/// whatever class it claimed and is treated as one.
pub const NO_DEADLINE: u64 = 0;

/// Completion queue entry. Two per cache line.
#[repr(C, align(32))]
#[derive(Clone, Copy, Debug)]
pub struct Cqe {
    /// Echoed from the submission.
    pub user_data: u64,
    /// Non-negative values are success, and their meaning is per-opcode — for a
    /// transfer it is the count actually transferred, which is how partial
    /// completion is *stated* rather than inferred from a short result.
    ///
    /// Negative values are structured errors rather than an errno: see the
    /// [`error`] module and [`Cqe::error`]. Cancellation is
    /// [`cflags::CANCELLED`] and is never an error code.
    pub result: i32,
    /// See the `cflags` module.
    pub flags: u32,
    /// When the service actually finished the work, on the same clock as
    /// [`Sqe::deadline`]. Eight bytes that give whole-system deadline
    /// accounting with no instrumentation build.
    pub timestamp: u64,
    /// Operation-specific payload.
    pub ext: u64,
}

/// Submission flags.
pub mod flags {
    /// The next entry does not start until this one completes successfully.
    pub const LINK: u8 = 1 << 0;
    /// Do not start until all prior entries have completed.
    pub const DRAIN: u8 = 1 << 1;
    /// `buf_set`/`buf_index` name a registered buffer rather than an address.
    pub const FIXED_BUF: u8 = 1 << 2;
    /// Produce no completion. Fire and forget.
    pub const NO_CQE: u8 = 1 << 3;
}

/// Completion flags.
pub mod cflags {
    /// More completions follow for the same `user_data`.
    pub const MORE: u32 = 1 << 0;
    /// The operation was cancelled rather than executed.
    pub const CANCELLED: u32 = 1 << 1;
}

/// The error space.
///
/// A negative [`Cqe::result`] is `-((domain << 16) | code)`, and [`Cqe::ext`]
/// carries a per-domain detail. The domain says *which kind of thing refused*,
/// which is the distinction a caller must act on and the one `errno` throws
/// away by giving every subsystem the same hundred and thirty integers.
///
/// The domain is the stable part and the code is the detailed part: a service
/// may add codes freely and may not add domains. Six is a number that fits in
/// the head, which is the property `errno` lost by growing.
///
/// See `docs/rfc/0010-structured-errors.md`.
pub mod error {
    /// The caller does not hold the capability, or holds it without this right.
    /// Detail: the offending capability index.
    pub const AUTHORITY: u8 = 1;
    /// A reservation was refused: the deadline could not be promised. Detail:
    /// the largest budget that would have been granted.
    pub const ADMISSION: u8 = 2;
    /// A quota, a budget or a device limit was reached. Detail: the limit.
    pub const RESOURCE: u8 = 3;
    /// The far side is gone, has restarted, or speaks a version this channel
    /// did not negotiate. Detail: the peer's channel epoch.
    pub const PEER: u8 = 4;
    /// The entry is malformed. Unknown opcode, unknown flag, non-zero reserved
    /// field: all refused, never ignored. Detail: the offending field.
    pub const ARGUMENT: u8 = 5;
    /// The hardware reported a failure. Detail: the device's own status.
    pub const DEVICE: u8 = 6;

    /// Codes within [`AUTHORITY`].
    pub mod authority {
        /// The index names no capability in the caller's table.
        pub const NO_SUCH_CAP: u16 = 1;
        /// The capability exists but does not carry the right this asks for.
        pub const RIGHT_NOT_HELD: u16 = 2;
        /// The capability was revoked. Distinguishable from never having held
        /// it, because the two need different handling.
        pub const REVOKED: u16 = 3;
    }

    /// Codes within [`ADMISSION`].
    pub mod admission {
        /// The schedulability test failed against existing reservations.
        pub const NOT_SCHEDULABLE: u16 = 1;
        /// No physical core is free under the whole-core rule.
        pub const NO_CORE: u16 = 2;
        /// Memory bandwidth or a cache partition could not be reserved.
        pub const NO_BANDWIDTH: u16 = 3;
    }

    /// Codes within [`RESOURCE`].
    pub mod resource {
        /// The component's own quota is exhausted. Local and deterministic:
        /// there is no global killer that picks somebody else.
        pub const QUOTA_EXHAUSTED: u16 = 1;
        /// The device cannot accept more outstanding work.
        pub const DEVICE_FULL: u16 = 2;
    }

    /// Codes within [`PEER`].
    pub mod peer {
        /// The peer restarted: the channel epoch moved, so every outstanding
        /// token is stale and must be discarded rather than matched.
        pub const EPOCH_CHANGED: u16 = 1;
        /// No common version. Detail carries the version this side offered.
        pub const VERSION_UNSUPPORTED: u16 = 2;
        /// A feature one side marked required is not offered by the other.
        /// Detail carries the missing bits.
        pub const FEATURE_REQUIRED: u16 = 3;
        /// The peer is gone and is not coming back.
        pub const GONE: u16 = 4;
    }

    /// Codes within [`ARGUMENT`].
    pub mod argument {
        /// The channel header failed structural validation.
        pub const MALFORMED_HEADER: u16 = 1;
        /// An opcode this service does not implement. Refused, not ignored.
        pub const UNKNOWN_OPCODE: u16 = 2;
        /// A flag bit this build does not know. Refused, not ignored: a bit
        /// that is silently dropped is two peers with different beliefs about
        /// what just happened.
        pub const UNKNOWN_FLAG: u16 = 3;
        /// A reserved field was not zero.
        pub const RESERVED_NOT_ZERO: u16 = 4;
    }

    /// Pack a domain and a code into a [`Cqe::result`].
    #[must_use]
    pub const fn pack(domain: u8, code: u16) -> i32 {
        -(((domain as i32) << 16) | code as i32)
    }

    /// Unpack a negative [`Cqe::result`]. `None` for success values.
    #[must_use]
    pub const fn unpack(result: i32) -> Option<(u8, u16)> {
        if result >= 0 {
            return None;
        }
        let raw = result.unsigned_abs();
        Some((((raw >> 16) & 0xFF) as u8, (raw & 0xFFFF) as u16))
    }
}

/// Scheduling classes. See the resource discipline document.
pub mod class {
    /// Deadline must be met. Admission-controlled; a reservation can be refused.
    pub const HARD: u16 = 0;
    /// Deadline honoured best-effort; missing it degrades quality.
    pub const SOFT: u16 = 1;
    /// Throughput matters, latency does not.
    pub const BATCH: u16 = 2;
    /// Runs only on resources nothing else wants.
    pub const IDLE: u16 = 3;
}

/// Consumer state flags, published by the consumer for the producer to read.
pub mod chan {
    /// The consumer is about to sleep and must be woken by a doorbell.
    /// Cleared while it is actively draining, which drives doorbell count to
    /// zero under load.
    pub const NEED_WAKEUP: u32 = 1 << 0;
}

/// Optional protocol behaviour, negotiated at setup.
///
/// A bit is *offered* by a peer that implements it and *required* by a peer that
/// cannot proceed without it. Additions are cheap and removals are not, so a
/// bitmap is the place this design will eventually show its age: prefer a
/// version bump over a fifth compatibility bit for one subsystem.
pub mod feature {
    /// Addresses may be passed in the submitter's own space rather than through
    /// a registered buffer set, because the device walks its page tables.
    pub const SHARED_VIRTUAL_MEMORY: u64 = 1 << 0;
    /// The doorbell is a user-level interrupt rather than a kernel call.
    pub const USER_INTERRUPT_DOORBELL: u64 = 1 << 1;
    /// The consumer honours the hard class: it will refuse a deadline it cannot
    /// meet rather than accept one it will miss.
    pub const ADMISSION_CONTROL: u64 = 1 << 2;
    /// This channel carries control events — peer death, revocation, pressure,
    /// core reclaim, shutdown, generation change.
    pub const CONTROL_EVENTS: u64 = 1 << 3;
}

/// What both sides agreed to speak.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Negotiated {
    /// The highest version both sides can speak.
    pub version: u32,
    /// The features both sides offered. A feature outside this set must not be
    /// used on this channel even where the local build implements it.
    pub features: u64,
}

/// Channel header. First cache line of a channel mapping.
///
/// Every field here is untrusted input: it is written by a peer that may have
/// crashed, restarted, or been compromised. Validate before use, always.
#[repr(C, align(64))]
#[derive(Clone, Copy, Debug)]
pub struct ChannelHeader {
    /// Must equal [`CHANNEL_MAGIC`].
    pub magic: u64,
    /// Features the writer implements and offers.
    pub features: u64,
    /// The subset of `features` the writer cannot proceed without.
    pub features_required: u64,
    /// The highest version the writer speaks.
    pub abi_version: u32,
    /// The oldest version the writer still speaks. Setup meets in the middle
    /// rather than demanding equality, which is what makes a component
    /// updatable independently of the frame.
    pub abi_version_min: u32,
    /// Entries in each ring. Always a power of two.
    pub ring_size: u32,
    /// Byte offset of the submission entry array from the start of the mapping.
    pub sqe_offset: u32,
    /// Byte offset of the completion ring.
    pub cqe_offset: u32,
    /// Incremented by a peer that restarts. A mismatch means every outstanding
    /// token is stale and must be discarded rather than matched.
    pub epoch: u32,
    /// Reserved. Must be zero. Where a future field lands — gated by a feature
    /// bit wherever that is possible, so that adding one is not a version bump.
    pub _reserved: [u32; 4],
}

// The layout guarantees this crate exists to make. A change that breaks one of
// these is an ABI break and fails the build here rather than at a peer.
const _: () = assert!(core::mem::size_of::<Sqe>() == 64);
const _: () = assert!(core::mem::align_of::<Sqe>() == 64);
const _: () = assert!(core::mem::size_of::<Cqe>() == 32);
const _: () = assert!(core::mem::size_of::<ChannelHeader>() == 64);

impl Sqe {
    /// A zeroed entry. `const` so a ring can be initialised without a loop.
    pub const ZERO: Self = Self {
        opcode: 0,
        flags: 0,
        class: class::BATCH,
        cap: 0,
        user_data: 0,
        deadline: 0,
        offset: 0,
        buf_set: 0,
        buf_index: 0,
        len: 0,
        _reserved: 0,
        ext: [0; 2],
    };
}

impl Cqe {
    /// Did the operation fail?
    ///
    /// Cancellation is not failure: a cancelled operation reports it in
    /// [`cflags::CANCELLED`] and leaves `result` alone.
    #[must_use]
    pub const fn is_error(&self) -> bool {
        self.result < 0
    }

    /// The domain and code of a failure, or `None` for success.
    #[must_use]
    pub const fn error(&self) -> Option<(u8, u16)> {
        error::unpack(self.result)
    }

    /// Was the operation cancelled rather than executed?
    #[must_use]
    pub const fn was_cancelled(&self) -> bool {
        self.flags & cflags::CANCELLED != 0
    }
}

impl ChannelHeader {
    /// Structural validation: is this a header at all?
    ///
    /// Returns `false` for anything a hostile or crashed peer could have
    /// written. The caller must tear the channel down rather than repair it,
    /// and must never panic on the result. Version and feature agreement is a
    /// separate question — see [`Self::negotiate`].
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.magic == CHANNEL_MAGIC
            && self.ring_size.is_power_of_two()
            && self.ring_size > 0
            && self.abi_version_min <= self.abi_version
            && self._reserved == [0; 4]
    }

    /// Agree a version and a feature set with the peer that wrote this header.
    ///
    /// `offers` and `requires` are the local side's: what this build implements,
    /// and the subset it cannot proceed without. The result is the intersection,
    /// at the highest version both sides can speak.
    ///
    /// # Errors
    ///
    /// A packed [`error`] result, so a refusal can be written straight into a
    /// completion. `ARGUMENT` for a header that is not one; `PEER` with
    /// [`error::peer::VERSION_UNSUPPORTED`] when the version ranges do not
    /// overlap, or [`error::peer::FEATURE_REQUIRED`] when either side requires
    /// something the other does not offer.
    pub fn negotiate(&self, offers: u64, requires: u64) -> Result<Negotiated, i32> {
        if !self.is_valid() {
            return Err(error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER));
        }

        // The highest version both sides speak, if the ranges overlap at all.
        let version = if self.abi_version < ABI_VERSION { self.abi_version } else { ABI_VERSION };
        if version < self.abi_version_min || version < ABI_VERSION_MIN {
            return Err(error::pack(error::PEER, error::peer::VERSION_UNSUPPORTED));
        }

        let common = self.features & offers;
        let peer_unmet = self.features_required & !common;
        let local_unmet = requires & !common;
        if peer_unmet != 0 || local_unmet != 0 {
            return Err(error::pack(error::PEER, error::peer::FEATURE_REQUIRED));
        }

        Ok(Negotiated { version, features: common })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A header from a peer identical to this build.
    fn peer(version: u32, min: u32, features: u64, required: u64) -> ChannelHeader {
        ChannelHeader {
            magic: CHANNEL_MAGIC,
            features,
            features_required: required,
            abi_version: version,
            abi_version_min: min,
            ring_size: 64,
            sqe_offset: 64,
            cqe_offset: 64 + 64 * 64,
            epoch: 1,
            _reserved: [0; 4],
        }
    }

    #[test]
    fn an_error_survives_the_round_trip() {
        for (domain, code) in [
            (error::AUTHORITY, error::authority::REVOKED),
            (error::ADMISSION, error::admission::NOT_SCHEDULABLE),
            (error::RESOURCE, error::resource::QUOTA_EXHAUSTED),
            (error::PEER, error::peer::EPOCH_CHANGED),
            (error::ARGUMENT, error::argument::UNKNOWN_FLAG),
            (error::DEVICE, 0xFFFF),
        ] {
            let packed = error::pack(domain, code);
            assert!(packed < 0, "an error must be a negative result");
            assert_eq!(error::unpack(packed), Some((domain, code)));
        }
    }

    #[test]
    fn success_is_not_an_error() {
        // Including the boundary: a zero-length transfer succeeded.
        for result in [0, 1, 4096, i32::MAX] {
            assert_eq!(error::unpack(result), None);
        }
    }

    #[test]
    fn cancellation_is_not_an_error() {
        // The rule from RFC 0010 that is easiest to break by accident: a
        // cancelled operation reports it in the flags and leaves `result` alone,
        // so a caller checking only `is_error` sees a success and a caller
        // checking only the flag sees a cancellation. Both are right.
        let cqe = Cqe {
            user_data: 7,
            result: 0,
            flags: cflags::CANCELLED,
            timestamp: 0,
            ext: 0,
        };
        assert!(!cqe.is_error());
        assert!(cqe.was_cancelled());
        assert_eq!(cqe.error(), None);
    }

    #[test]
    fn an_identical_peer_agrees() {
        let all = feature::SHARED_VIRTUAL_MEMORY | feature::CONTROL_EVENTS;
        let agreed = peer(ABI_VERSION, ABI_VERSION_MIN, all, 0).negotiate(all, 0).unwrap();
        assert_eq!(agreed.version, ABI_VERSION);
        assert_eq!(agreed.features, all);
    }

    #[test]
    fn a_newer_peer_meets_us_at_our_version() {
        // The case lockstep versioning made impossible: a peer built later than
        // this one, still able to speak what this one speaks.
        let agreed = peer(ABI_VERSION + 7, ABI_VERSION, 0, 0).negotiate(0, 0).unwrap();
        assert_eq!(agreed.version, ABI_VERSION);
    }

    #[test]
    fn a_peer_below_our_floor_is_refused() {
        let older = ABI_VERSION_MIN.saturating_sub(1);
        let header = peer(older, older, 0, 0);
        assert_eq!(
            header.negotiate(0, 0),
            Err(error::pack(error::PEER, error::peer::VERSION_UNSUPPORTED))
        );
    }

    #[test]
    fn a_peer_above_our_ceiling_is_refused() {
        // Its floor is higher than anything this build speaks.
        let header = peer(ABI_VERSION + 9, ABI_VERSION + 5, 0, 0);
        assert_eq!(
            header.negotiate(0, 0),
            Err(error::pack(error::PEER, error::peer::VERSION_UNSUPPORTED))
        );
    }

    #[test]
    fn a_feature_the_peer_requires_and_we_lack_is_refused() {
        let header = peer(
            ABI_VERSION,
            ABI_VERSION_MIN,
            feature::USER_INTERRUPT_DOORBELL,
            feature::USER_INTERRUPT_DOORBELL,
        );
        assert_eq!(
            header.negotiate(0, 0),
            Err(error::pack(error::PEER, error::peer::FEATURE_REQUIRED))
        );
    }

    #[test]
    fn a_feature_we_require_and_the_peer_lacks_is_refused() {
        // The same refusal in the other direction, which is a separate code
        // path and the one an implementation forgets.
        let header = peer(ABI_VERSION, ABI_VERSION_MIN, 0, 0);
        assert_eq!(
            header.negotiate(feature::ADMISSION_CONTROL, feature::ADMISSION_CONTROL),
            Err(error::pack(error::PEER, error::peer::FEATURE_REQUIRED))
        );
    }

    #[test]
    fn an_offered_feature_the_other_side_lacks_is_simply_absent() {
        // Offered but not required: agreement without it, rather than refusal.
        let header = peer(ABI_VERSION, ABI_VERSION_MIN, feature::CONTROL_EVENTS, 0);
        let agreed = header.negotiate(feature::SHARED_VIRTUAL_MEMORY, 0).unwrap();
        assert_eq!(agreed.features, 0);
    }

    #[test]
    fn a_malformed_header_is_an_argument_error() {
        let malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER);

        let mut bad_magic = peer(ABI_VERSION, ABI_VERSION_MIN, 0, 0);
        bad_magic.magic = 0;
        assert_eq!(bad_magic.negotiate(0, 0), Err(malformed));

        let mut bad_size = peer(ABI_VERSION, ABI_VERSION_MIN, 0, 0);
        bad_size.ring_size = 63;
        assert_eq!(bad_size.negotiate(0, 0), Err(malformed));

        let mut zero_size = peer(ABI_VERSION, ABI_VERSION_MIN, 0, 0);
        zero_size.ring_size = 0;
        assert_eq!(zero_size.negotiate(0, 0), Err(malformed));

        let mut dirty_reserved = peer(ABI_VERSION, ABI_VERSION_MIN, 0, 0);
        dirty_reserved._reserved = [0, 1, 0, 0];
        assert_eq!(dirty_reserved.negotiate(0, 0), Err(malformed));

        let mut inverted = peer(ABI_VERSION, ABI_VERSION_MIN, 0, 0);
        inverted.abi_version_min = inverted.abi_version + 1;
        assert_eq!(inverted.negotiate(0, 0), Err(malformed));
    }

    #[test]
    fn a_deadline_of_zero_means_none() {
        assert_eq!(Sqe::ZERO.deadline, NO_DEADLINE);
    }
}
