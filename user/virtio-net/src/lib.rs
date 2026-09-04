// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The network driver: the **second** device driver in this system that lives
//! outside the frame, and the reason there is a second one.
//!
//! # Why this crate exists, which is not the same as what it does
//!
//! `user/virtio-blk` proved that a driver can live outside the frame. One driver
//! cannot say whether the *shape* it was built into is a shape or a coincidence:
//! a component crate forbidding `unsafe`, a compiled manifest, a scheduled
//! ring-3 polling loop, registers reached through RFC 0033's safe [`Window`], a
//! device translation asked of the frame over control-ring opcodes (RFC 0047),
//! and buffers owned by one side at a time (RFC 0024/0028). E1-B03 is the second
//! sample, and the useful part of it is the list of places where following the
//! first one did not work.
//!
//! [`docs/rfc/0051`] is that list in full. In one paragraph: nothing in `ring/`,
//! `abi/` or the frame's device discovery had to change, which is a stronger
//! result than it sounds — `kernel/src/arch/x86_64/virtio.rs` was already
//! parameterised by device id and named this task as the caller that would use
//! it. What did not carry over is smaller and sharper than expected, and all of
//! it is on the **receive** side: an executor signature that cannot say
//! *accepted, answer to follow*; a used ring whose element has to be read for
//! its head as well as its length; a wait with no answer owed and therefore a
//! bound that has to be told rather than derived; and one gap in a client's
//! types that the block driver could not have found.
//!
//! # The three things this crate is built to make true
//!
//! **A driver with no `unsafe` can drive real hardware — and a *second* one can,
//! without widening the frame.** Registers are reached through
//! [`f_ring::device::Window`] and virtqueues through
//! [`f_ring::device::Region`], both of which are safe accessors over addresses
//! the frame mapped in answer to capabilities this component holds. Not one line
//! was added to `ring/src/device.rs` for this driver, which is the evidence RFC
//! 0033 asked for and could not supply from one example.
//!
//! **The bytes of a packet never pass through this component, in either
//! direction.** A request names a registered buffer set and an index; the
//! service resolves it to a [`Reach`](f_ring::registry::Reach), which is an
//! address and a length and deliberately not a slice; the address goes into a
//! descriptor and the device transfers into the client's memory directly.
//! [`driver::Counters::copies`] is the number that says so and
//! [`driver::Driver::provoke_copy`] is what makes that zero a measurement rather
//! than an absence. The *receive* half of that claim is the harder one: this
//! component is the only thing between a device and a client's buffer, and the
//! obvious implementation reads the frame to find out how long it is. This one
//! takes the length off the used ring instead.
//!
//! **A driver cannot address memory outside its grant, including on the
//! direction where the device writes.** Every address this crate can put in
//! front of the device comes from either
//! [`Region::device_at`](f_ring::device::Region::device_at) — its own grant — or
//! a `Reach` the frame answered a registration with. There is no third source,
//! and the device's own IOMMU domain is what refuses one anyway.
//! [`driver::Driver::provoke_escape`] is what asks it, and it asks on a
//! **receive** descriptor: what an unrefused escape produces there is a device
//! *writing* into memory this component was never granted, at a moment nothing
//! in this system chose, for as long as the buffer stays posted.
//!
//! # What this driver is not, listed rather than discovered
//!
//! One queue pair and no control queue. No `VIRTIO_NET_F_MAC`, no
//! `MRG_RXBUF`, no checksum or segmentation offload, no multiqueue, no
//! filtering, no link-state notification, no interrupt. `crate::transport`'s
//! module comment lists what each absence costs, and the manifest's `notify`
//! need names the task that ends the last one. There is no network stack here
//! and there is not going to be: whoever forms a frame chooses its addresses,
//! and on this boot that is the client.
//!
//! # What runs where, stated rather than implied
//!
//! This crate is the driver **and it is scheduled**. [`component::start`] runs at
//! ring 3 on a core the frame allocated it, adopts its control ring and the ring
//! it serves its client on in safe code — `f_ring::adopt`, RFC 0037 — drives real
//! registers through mappings the frame made in answer to what its manifest
//! declares, and ends on a stop notice or on a bound it was told.
//! `kernel/src/net.rs` is the supervisor's half and nothing else, and
//! `cargo xtask lint-datapath` refuses a line under `kernel/` that names
//! `Driver::`.
//!
//! What is still owed is one sentence, and it is the block driver's sentence
//! unchanged: this instance is *scheduled* and not *spawned into a place*.
//! `CHAOS_GAP` in xtask carries exactly that difference and a second driver in
//! the same position widens nothing.

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
// `component.rs` is architecture-specific; the one instruction underneath it is,
// and `f_abi::door::call` is compiled only where there is a frame to call. The
// same gate `user/init`, `user/store` and `user/virtio-blk` have, with the same
// reversal: an AArch64 frame.
//
// The second gate is the `image` feature, and it is off in exactly one place:
// the frame, which links this crate for `routing`'s offsets and
// `driver::Counters`' shape. A `#[panic_handler]` is a lang item and there may
// be one per linked artefact, so the module carrying this component's would
// otherwise collide with the frame's own.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod component;

/// Why the driver could not do what it was asked.
///
/// Every variant is either the device disagreeing with the specification or this
/// component's own arithmetic being wrong, and each one earns a distinct
/// [`f_abi::error`] pair through [`Trouble::packed`] — R07: a caller that cannot
/// tell why it was refused cannot handle a refusal as ordinary control flow.
///
/// There is no *out of memory* variant and there will not be one: this component
/// allocates nothing. Everything it is made of is routed at spawn.
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
    /// **Fatal on purpose**, and more so here than on the block driver.
    /// `kernel/src/arch/x86_64/dma.rs` records what it cost to discover that a
    /// device without this bit is architecturally outside the remapping unit:
    /// every isolation test passes, for the wrong reason. On a network device
    /// the consequence is not only an untested protection — it is a bus master
    /// writing wherever a driver's arithmetic said, whenever a peer sends a
    /// packet, with no request outstanding and nothing timing it. R04 says
    /// refuse.
    NoPlatformAddressing,
    /// The device refused the feature set this driver offered, which is the one
    /// veto RFC 0011's shape gives a peer made of silicon.
    FeaturesRefused,
    /// The device reports fewer than two queues, or one too small for a
    /// two-descriptor chain.
    NoQueue,
    /// The device gave back a receive chain reporting fewer bytes written than
    /// the header it was required to fill in.
    ///
    /// Its own variant rather than a bare code at the arithmetic that finds it,
    /// and that is the point of this enum: the `DEVICE` space is small and every
    /// number in it has to be distinguishable from every other, so a refusal
    /// packed by hand somewhere in this crate is a number nothing checks against
    /// the rest. R07, applied to the crate's own codes rather than only to a
    /// client's reading of them.
    ShortUsed,
    /// The device did not take a frame off the transmit queue inside the bound
    /// this driver spins for.
    ///
    /// Distinct from [`Trouble::Device`] on purpose: *the device never answered*
    /// and *the device answered about a chain that does not exist* are different
    /// failures, and a client told the same code for both cannot retry one and
    /// give up on the other.
    NotTaken,
    /// The device published a used element naming a chain this driver never
    /// posted.
    ///
    /// Its own variant rather than folded into [`Trouble::NotResponding`],
    /// because it is a device *steering* the driver rather than a device failing
    /// to answer — and a driver that followed it would release a client's buffer
    /// the device is still writing into. There is no equivalent on the block
    /// driver: with one chain outstanding there is nothing for a head to
    /// choose between.
    Device,
}

impl Trouble {
    /// The refusal a client reads.
    ///
    /// `DEVICE` for everything the hardware decided, which is RFC 0010's domain
    /// for a hardware failure, with the detail being *which* — a client that was
    /// told only `DEVICE` could not tell a device that never answered from one
    /// that refused a feature set. `ARGUMENT/BAD_ADDRESS` for the one that is
    /// this component's own arithmetic, because a caller that named the wrong
    /// place can name a different one.
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
            Self::ShortUsed => f_abi::error::pack(f_abi::error::DEVICE, 5),
            Self::Device => f_abi::error::pack(f_abi::error::DEVICE, 6),
            Self::NotTaken => f_abi::error::pack(f_abi::error::DEVICE, 7),
        }
    }

    /// A sentence for a boot log.
    ///
    /// The frame prints these; a scheduled driver has no serial port and its
    /// client reads [`Trouble::packed`] instead. Both exist because they answer
    /// different readers, and the day the second is the only one, this method
    /// goes.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Layout => "a granted region is not the shape the queues need",
            Self::Register(_) => "an accessor refused an offset outside a granted window",
            Self::NotResponding => "the device did not come out of reset",
            Self::NoPlatformAddressing => {
                "the device does not offer platform addressing, so it would bypass the \
                 remapping unit"
            }
            Self::FeaturesRefused => "the device refused the features this driver offered",
            Self::NoQueue => "the device reports fewer than two usable queues",
            Self::ShortUsed => "the device reported a receive shorter than the header it fills",
            Self::Device => "the device gave back a chain this driver never posted",
            Self::NotTaken => "the device did not take the frame inside the driver's bound",
        }
    }
}

impl From<i32> for Trouble {
    /// Every refusal [`f_ring::device`] produces is an accessor refusing an
    /// offset, so the conversion is total and lossless — which is what lets the
    /// transport and the queues use `?` on an accessor without a `map_err` at
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
            Trouble::ShortUsed,
            Trouble::Device,
            Trouble::NotTaken,
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

        // The seven hardware failures are distinguishable from each other, which
        // is the half of R07 a single `DEVICE` domain would lose. Seven and not
        // five: the two added last are the ones review found packed by hand at
        // the arithmetic that produced them — a used length too short to hold a
        // header, and a transmit the device never took — which is how two
        // different failures come to share code zero.
        let hardware = [
            Trouble::NotResponding,
            Trouble::NoPlatformAddressing,
            Trouble::FeaturesRefused,
            Trouble::NoQueue,
            Trouble::ShortUsed,
            Trouble::Device,
            Trouble::NotTaken,
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

    #[test]
    fn the_two_drivers_do_not_share_a_routing_magic() {
        // Both components are mapped at the same board address by the one driver
        // shape the frame builds, so the magic is the only thing that says which
        // board a component is reading. A build that routed the wrong image into
        // the wrong supervisor has to find a refusal there rather than a page
        // whose fields mean something else.
        //
        // Asserted here rather than in `routing`, because the value being
        // compared against is a *number* and not the other crate's constant:
        // this crate does not depend on `f-virtio-blk` and must not start.
        // `kernel/src/net.rs` is where the two definitions are linked together
        // and where a compile-time assertion can name both.
        assert_ne!(routing::MAGIC, 0x626C_6B5F_726F_7574, "virtio-blk's routing magic");
    }
}
