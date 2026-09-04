// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The display driver: the **third** device driver in this system that lives
//! outside the frame, and the first one that is not a pipe.
//!
//! # Why this crate exists, which is not the same as what it does
//!
//! `user/virtio-blk` proved that a driver can live outside the frame.
//! `user/virtio-net` proved that the *shape* that answer arrived in was a shape
//! and not a coincidence — RFC 0051 counts what carried over and what did not.
//! Both of those devices move **opaque bytes**: a block request is a length and
//! an offset, a frame is a length, and neither driver ever has an opinion about
//! what is in the buffer it names.
//!
//! A display controller is a different kind of device. It takes **structured
//! commands** — create a resource, give it pages, copy into it, point a scanout
//! at it, push it to the screen — answers every one of them with a typed
//! response, and owns something neither of the other two has: a **scanout**,
//! which is a standing output that outlives the request that produced it. E1-B04
//! is the third sample and the useful part of it is what that difference cost.
//!
//! [`docs/rfc/0054`] is the answer in full. In one paragraph: nothing in `ring/`
//! or `abi/` had to change, and the frame's device discovery had to change in
//! exactly one place — a display controller has no *transitional* PCI device id,
//! because it was defined after the modern transport, and `virtio::route` was
//! written assuming every virtio device has one. Four of RFC 0051's five
//! receive-direction differences do not apply here, which says they were about
//! receiving rather than about being a second driver. And the fifth one applies
//! to something new: on this device the interval in which the device owns a
//! client's buffer is bounded by a **pair of commands** rather than by a chain,
//! which is a shape neither of the other two drivers could have found.
//!
//! # The three things this crate is built to make true
//!
//! **A driver with no `unsafe` can drive real hardware — and a *third* one can,
//! for a device of a different kind, without widening the frame.** Registers are
//! reached through [`f_ring::device::Window`] and the virtqueue through
//! [`f_ring::device::Region`], both of which are safe accessors over addresses
//! the frame mapped in answer to capabilities this component holds. Not one line
//! was added to `ring/src/device.rs` for this driver either.
//!
//! **The pixels never pass through this component.** A request names a
//! registered buffer set and an index; the service resolves it to a
//! [`Reach`](f_ring::registry::Reach), which is an address and a length and
//! deliberately not a slice; the address goes into a display command and the
//! device reads the client's memory directly.
//! [`driver::Counters::copies`] is the number that says so and
//! [`driver::Driver::provoke_copy`] is what makes that zero a measurement rather
//! than an absence. It is the *easiest* of the three zeroes to hold and
//! [`driver`]'s module comment says why rather than claiming otherwise.
//!
//! **A driver cannot address memory outside its grant, and here that matters for
//! a new reason.** Every address this crate can put in front of the device comes
//! from either [`Region::device_at`](f_ring::device::Region::device_at) — its own
//! grant — or a `Reach` the frame answered a registration with. There is no third
//! source, and the device's own IOMMU domain is what refuses one anyway.
//! [`driver::Driver::provoke_escape`] is what asks it, and what an unrefused
//! escape produces here is not a corruption to be found later in somebody's
//! buffer: it is a page of another component's memory **rendered to a screen**,
//! which is outside the machine and outside every mechanism in this system.
//!
//! # What this driver is not, listed rather than discovered
//!
//! One scanout, one pixel format, whole frames only, no cursor, no
//! `RESOURCE_UNREF`, no `GET_DISPLAY_INFO`, no 3D, no blob resources, no EDID,
//! and no interrupt. [`driver`]'s module comment lists what each absence costs
//! and `crate::transport`'s lists what each unnegotiated feature costs. There is
//! no window system here and there is not going to be: whoever draws a frame
//! chooses what is in it, and on this boot that is the client.
//!
//! # How anybody knows it worked
//!
//! **This is the part that has no precedent in the other two drivers.** A block
//! driver's evidence is bytes in the client's buffer; a network driver's is a
//! reply in the client's buffer and a fault record. Both are inside the machine,
//! and a component's own counters can be checked against them.
//!
//! A scanout is not inside the machine. The 2D protocol has no command that
//! reads a resource back, so nothing in this system — not the client, not the
//! frame, not this driver — can observe what is on the screen. Every counter in
//! [`driver::Counters`] is a statement about commands the display *accepted*,
//! and a display that accepted every command and drew nothing would move all of
//! them. So `cargo xtask gpu` captures the framebuffer from **outside** the
//! emulator and compares it with the bytes the client owns, and the boot holds
//! itself still while that happens. RFC 0054 argues why a driver's own report
//! cannot stand in for it, and why the harness's capture is the only honest
//! reading of E1-B04's exit criterion.
//!
//! # What runs where, stated rather than implied
//!
//! This crate is the driver **and it is scheduled**. [`component::start`] runs at
//! ring 3 on a core the frame allocated it, adopts its control ring and the ring
//! it serves its client on in safe code — `f_ring::adopt`, RFC 0037 — drives real
//! registers through mappings the frame made in answer to what its manifest
//! declares, and ends on a stop notice or on a bound it was told.
//! `kernel/src/gpu.rs` is the supervisor's half and nothing else, and
//! `cargo xtask lint-datapath` refuses a line under `kernel/` that names
//! `Driver::`.
//!
//! What is still owed is one sentence, and it is the other two drivers' sentence
//! unchanged: this instance is *scheduled* and not *spawned into a place*.
//! `CHAOS_GAP` in xtask carries exactly that difference and a third driver in the
//! same position widens nothing.

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
// same gate `user/init`, `user/store`, `user/virtio-blk` and `user/virtio-net`
// have, with the same reversal: an AArch64 frame.
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
/// **What is deliberately not here is a variant per display response.** The
/// display answers every command with a number of its own — `ERR_UNSPEC`,
/// `ERR_OUT_OF_MEMORY`, `ERR_INVALID_RESOURCE_ID` and the rest — and those reach
/// the client unchanged, packed into the same `DEVICE` domain with the device's
/// own code. `driver::cmd::RESP_FIRST` is what keeps the two spaces apart and a
/// test in `driver` asserts it. A driver that translated a display's refusals
/// into an enum of its own would be a driver deciding, on its client's behalf,
/// which of the display's distinctions matter.
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
    /// **Fatal on purpose**, and for a third reason.
    /// `kernel/src/arch/x86_64/dma.rs` records what it cost to discover that a
    /// device without this bit is architecturally outside the remapping unit:
    /// every isolation test passes, for the wrong reason. On a block device the
    /// consequence of that is a device reading memory nobody granted it into a
    /// client's buffer; on a network device it is a device writing wherever a
    /// driver's arithmetic said. On a **display** it is neither, and it is
    /// worse: what the device reads it puts on a screen, so an unrefused
    /// transfer is a page of somebody's memory shown to whoever is looking at
    /// the machine. That is the first path out of this system that does not
    /// cross a ring, a channel or a capability. R04 says refuse.
    NoPlatformAddressing,
    /// The device refused the feature set this driver offered, which is the one
    /// veto RFC 0011's shape gives a peer made of silicon.
    FeaturesRefused,
    /// The device reports no control queue, or one too small for a
    /// two-descriptor chain.
    NoQueue,
    /// The device gave back a chain reporting fewer bytes written than the
    /// response header every display command is answered with.
    ///
    /// Its own variant rather than a bare code at the arithmetic that finds it,
    /// and that is the point of this enum: the `DEVICE` space is shared with the
    /// display's own response numbers, so a refusal packed by hand somewhere in
    /// this crate is a number nothing checks against the rest.
    ShortUsed,
    /// The device published a used element naming a chain this driver never
    /// posted.
    ///
    /// Its own variant rather than folded into [`Trouble::NotResponding`],
    /// because it is a device *steering* the driver rather than a device failing
    /// to answer.
    Device,
    /// The device did not answer a command inside the bound this driver spins
    /// for.
    ///
    /// Distinct from [`Trouble::Device`] on purpose: *the device never answered*
    /// and *the device answered about a chain that does not exist* are different
    /// failures, and a client told the same code for both cannot retry one and
    /// give up on the other.
    ///
    /// Unlike the network driver's equivalent this one is unambiguous. Every
    /// command in the display protocol is a request the device owes an answer
    /// to, so a bound that fires here is a broken device rather than a quiet
    /// link — which is why the bound is a constant in this crate and not a
    /// number the frame has to tell the component.
    NotAnswered,
}

impl Trouble {
    /// The refusal a client reads.
    ///
    /// `DEVICE` for everything the hardware decided, which is RFC 0010's domain
    /// for a hardware failure, with the detail being *which*.
    /// `ARGUMENT/BAD_ADDRESS` for the one that is this component's own
    /// arithmetic, because a caller that named the wrong place can name a
    /// different one.
    ///
    /// Every code here is a single digit, and that is load-bearing rather than
    /// tidy: the display's own response numbers start at
    /// `driver::cmd::RESP_FIRST` and are passed through into this same domain
    /// unchanged, so the two spaces have to be disjoint. `driver`'s tests assert
    /// it.
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
            Self::NotAnswered => f_abi::error::pack(f_abi::error::DEVICE, 7),
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
            Self::Layout => "a granted region is not the shape the queue needs",
            Self::Register(_) => "an accessor refused an offset outside a granted window",
            Self::NotResponding => "the device did not come out of reset",
            Self::NoPlatformAddressing => {
                "the device does not offer platform addressing, so what it reads would bypass \
                 the remapping unit and reach a screen"
            }
            Self::FeaturesRefused => "the device refused the features this driver offered",
            Self::NoQueue => "the device reports no usable control queue",
            Self::ShortUsed => "the device reported a command answer shorter than a header",
            Self::Device => "the device gave back a chain this driver never posted",
            Self::NotAnswered => "the device did not answer a command inside the driver's bound",
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
            Trouble::ShortUsed,
            Trouble::Device,
            Trouble::NotAnswered,
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
        // is the half of R07 a single `DEVICE` domain would lose.
        let hardware = [
            Trouble::NotResponding,
            Trouble::NoPlatformAddressing,
            Trouble::FeaturesRefused,
            Trouble::NoQueue,
            Trouble::ShortUsed,
            Trouble::Device,
            Trouble::NotAnswered,
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
    fn the_three_drivers_do_not_share_a_routing_magic() {
        // All three components are mapped at the same board address by the one
        // driver shape the frame builds, so the magic is the only thing that
        // says which board a component is reading. A build that routed the wrong
        // image into the wrong supervisor has to find a refusal there rather
        // than a page whose fields mean something else.
        //
        // Asserted here rather than in `routing`, because the values being
        // compared against are *numbers* and not the other crates' constants:
        // this crate does not depend on `f-virtio-blk` or `f-virtio-net` and
        // must not start. `kernel/src/gpu.rs` is where the definitions are
        // linked together and where a compile-time assertion can name all
        // three.
        assert_ne!(routing::MAGIC, 0x626C_6B5F_726F_7574, "virtio-blk's routing magic");
        assert_ne!(routing::MAGIC, 0x6E65_745F_726F_7574, "virtio-net's routing magic");
    }
}
