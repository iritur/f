// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The block driver: the first device driver in this system that lives outside
//! the frame.
//!
//! # What this crate is, in one paragraph
//!
//! A component. It has a manifest — `user/virtio-blk/manifest.toml`, written
//! before it on purpose — declaring the `private` speculation domain, four
//! register frames, sixty-four kibibytes of untyped memory for its virtqueues,
//! an interrupt, a powerbox endpoint, and one server ring speaking `blk`
//! version 1 whose payload is `registered`. It is compiled into a component
//! file by `cargo xtask component` and named by one hash over its record and
//! its image. And it contains **no `unsafe`**: the crate inherits
//! `unsafe_code = "forbid"` from the workspace, `cargo xtask lint-unsafe` is
//! the backstop, and RFC 0033 is the argument for how a component with that
//! property can touch a device at all.
//!
//! # The three things this crate is built to make true
//!
//! **A driver with no `unsafe` can drive real hardware.** Registers are reached
//! through [`f_ring::device::Window`] and virtqueues through
//! [`f_ring::device::Region`], both of which are safe accessors over addresses
//! the frame mapped in answer to capabilities this component holds. The
//! obligation is discharged once, on the frame's side of the boundary, exactly
//! as `f_abi::door::call` and `f_abi::state::Reader` already discharge theirs.
//! RFC 0033.
//!
//! **The bytes of a read or a write never pass through this component.** A
//! request names a registered buffer set and an index; the service resolves it
//! to a [`Reach`](f_ring::registry::Reach), which is an address and a length and
//! deliberately not a slice; the address goes into a descriptor and the device
//! transfers into the client's memory directly. [`driver::Counters::copies`] is
//! the number that says so, published through the state tree, and
//! [`driver::Driver::provoke_copy`] is what makes that zero a measurement rather
//! than an absence.
//!
//! **A driver cannot address memory outside its grant.** Every address this
//! crate can put in front of the device comes from either
//! [`Region::device_at`](f_ring::device::Region::device_at) — its own grant — or
//! a `Reach` the frame answered a registration with. There is no third source,
//! and the device's own IOMMU domain is what refuses one anyway: E1-B01 proved
//! that at the device with the frame's own adversary, and `cargo xtask blk`
//! proves it at the component, which is the clause E1-B01's exit could not
//! observe on its own.
//!
//! # What runs where today, stated rather than implied
//!
//! This crate is the driver **and it is scheduled**. [`component::start`] runs
//! at ring 3 on a core the frame allocated it, adopts its control ring and the
//! ring it serves its client on in safe code — `f_ring::adopt`, RFC 0037 —
//! drives real registers through mappings the frame made in answer to what its
//! manifest declares, and ends on a stop notice. `kernel/src/blk.rs` is the
//! supervisor's half and nothing else; the grep RFC 0033 wrote as its own
//! reversal condition — *see which crate calls `Driver::execute`* — now finds
//! only this one.
//!
//! Two sentences that used to be here are worth keeping as scars, because both
//! were true and neither is any more:
//!
//! - *Nothing schedules a component.* E1-B08 landed the mechanism and RFC 0047
//!   pointed it at a driver: more than one page of text, a device's registers
//!   mapped uncached into a component's address space, and its queue memory
//!   mapped whole.
//! - *A component cannot drive a ring, because adopting a mapped channel is
//!   `unsafe`.* RFC 0037 answered that, and deliberately did not answer it the
//!   way RFC 0033 answered the device half — a channel is shared with a peer
//!   that may be hostile and a device window is not, so the two arguments are
//!   different arguments.
//!
//! What has **not** changed is that there is one body of driver code. The
//! frame links this crate for its manifest-facing constants and for
//! [`routing`], and calls none of it.
//!
//! What is still owed, and it is one sentence: this instance is *scheduled* and
//! not *spawned into a place*. `kernel/src/component.rs` builds a place for
//! this manifest on every boot and never hands its occupant a core, because the
//! supervisor that would is the ring-3 supervisor E1-B05 owes. `CHAOS_GAP` in
//! xtask carries exactly that difference and nothing wider.

#![no_std]

pub mod driver;
pub mod queue;
// Outside the `image` gate, because it is the one module both sides read: the
// frame writes the page this describes and the component reads it, and a set of
// offsets that existed in only one of the two builds would be a layout with two
// definitions. RFC 0047.
pub mod routing;
pub mod transport;

// The component half is x86-64's, and only because the door is. Nothing in
// `component.rs` is architecture-specific; the one instruction underneath it
// is, and `f_abi::door::call` is compiled only where there is a frame to call.
// The same gate `user/init` and `user/store` have, with the same reversal: an
// AArch64 frame.
//
// The second gate is the `image` feature, and it is off in exactly one place:
// the frame, which links this crate as a library because nothing schedules a
// component yet. A `#[panic_handler]` is a lang item and there may be one per
// linked artefact, so the module carrying this component's would otherwise
// collide with the frame's own. `user/virtio-blk/Cargo.toml` states the
// reversal, which is E1-B08.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod component;

/// Why the driver could not do what it was asked.
///
/// Every variant is either the device disagreeing with the specification or
/// this component's own arithmetic being wrong, and each one earns a distinct
/// [`f_abi::error`] pair through [`Trouble::packed`] — R07: a caller that
/// cannot tell why it was refused cannot handle a refusal as ordinary control
/// flow.
///
/// There is no *out of memory* variant and there will not be one: this
/// component allocates nothing. Everything it is made of is routed at spawn.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Trouble {
    /// A window or a region is not the shape the thing being laid out in it
    /// needs: a queue that does not fit, a size that is not a power of two, a
    /// descriptor index past the ring.
    Layout,
    /// An accessor refused, carrying its own packed reason — which is always
    /// `ARGUMENT/BAD_ADDRESS`, because that is the only thing
    /// [`f_ring::device`] refuses.
    Register(i32),
    /// The device did not come out of reset.
    NotResponding,
    /// The device does not offer `VIRTIO_F_ACCESS_PLATFORM`, so its transfers
    /// would bypass the platform's address translation entirely.
    ///
    /// **Fatal on purpose.** `kernel/src/arch/x86_64/dma.rs` records what it
    /// cost to discover that a device without this bit is architecturally
    /// outside the remapping unit: every isolation test passes, for the wrong
    /// reason. A driver that ran anyway would have no isolation and no way to
    /// know, so R04 says refuse.
    NoPlatformAddressing,
    /// The device refused the feature set this driver offered, which is the one
    /// veto RFC 0011's shape gives a peer made of silicon.
    FeaturesRefused,
    /// The device reports no queue zero, or one too small for a
    /// three-descriptor chain.
    NoQueue,
}

impl Trouble {
    /// The refusal a client reads.
    ///
    /// `DEVICE` for everything the hardware decided, which is RFC 0010's domain
    /// for a hardware failure, with the detail being *which* — a client that
    /// was told only `DEVICE` could not tell a device that never answered from
    /// one that refused a feature set. `ARGUMENT/BAD_ADDRESS` for the two that
    /// are this component's own arithmetic, because a caller that named the
    /// wrong place can name a different one.
    #[must_use]
    pub const fn packed(self) -> i32 {
        match self {
            Self::Layout => {
                f_abi::error::pack(f_abi::error::ARGUMENT, f_abi::error::argument::BAD_ADDRESS)
            }
            Self::Register(packed) => packed,
            Self::NotResponding => f_abi::error::pack(f_abi::error::DEVICE, 1),
            Self::NoPlatformAddressing => f_abi::error::pack(f_abi::error::DEVICE, 2),
            Self::FeaturesRefused => f_abi::error::pack(f_abi::error::DEVICE, 3),
            Self::NoQueue => f_abi::error::pack(f_abi::error::DEVICE, 4),
        }
    }

    /// A sentence for a boot log.
    ///
    /// The frame prints these while it is the thing driving this component; a
    /// scheduled driver has no serial port and its client reads
    /// [`Trouble::packed`] instead. Both exist because they answer different
    /// readers, and the day the second is the only one, this method goes.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Layout => "a granted region is not the shape the queue needs",
            Self::Register(_) => "an accessor refused an offset outside a granted window",
            Self::NotResponding => "the device did not come out of reset",
            Self::NoPlatformAddressing => {
                "the device does not offer platform addressing, so it would bypass the \
                 remapping unit"
            }
            Self::FeaturesRefused => "the device refused the features this driver offered",
            Self::NoQueue => "the device reports no usable queue zero",
        }
    }
}

impl From<i32> for Trouble {
    /// Every refusal [`f_ring::device`] produces is an accessor refusing an
    /// offset, so the conversion is total and lossless — which is what lets the
    /// transport and the queue use `?` on an accessor without a `map_err` at
    /// every line.
    fn from(packed: i32) -> Self {
        Self::Register(packed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_trouble_names_a_domain_a_client_can_act_on() {
        // R07. A refusal that named no domain, or that named the same one for
        // two different failures, is a refusal a client has to guess at.
        let troubles = [
            Trouble::Layout,
            Trouble::Register(f_abi::error::pack(
                f_abi::error::ARGUMENT,
                f_abi::error::argument::BAD_ADDRESS,
            )),
            Trouble::NotResponding,
            Trouble::NoPlatformAddressing,
            Trouble::FeaturesRefused,
            Trouble::NoQueue,
        ];
        for trouble in troubles {
            let packed = trouble.packed();
            let (domain, _) = f_abi::error::unpack(packed).expect("a refusal is negative");
            assert!(
                domain == f_abi::error::DEVICE || domain == f_abi::error::ARGUMENT,
                "a driver refuses on its own arithmetic or on the hardware, and nothing else"
            );
            assert!(!trouble.message().is_empty());
        }

        // The four hardware failures are distinguishable from each other, which
        // is the half of R07 that a single `DEVICE` domain would lose.
        let hardware = [
            Trouble::NotResponding,
            Trouble::NoPlatformAddressing,
            Trouble::FeaturesRefused,
            Trouble::NoQueue,
        ];
        for (index, one) in hardware.iter().enumerate() {
            for other in hardware.iter().skip(index + 1) {
                assert_ne!(one.packed(), other.packed());
            }
        }
    }

    #[test]
    fn an_accessor_refusal_reaches_a_client_unchanged() {
        // Passed through rather than translated, for the reason
        // `kernel/src/iommu.rs` gives about the same boundary: a refusal this
        // component invented a code for is a refusal a client cannot act on.
        let bad = f_abi::error::pack(f_abi::error::ARGUMENT, f_abi::error::argument::BAD_ADDRESS);
        assert_eq!(Trouble::from(bad).packed(), bad);
    }
}
