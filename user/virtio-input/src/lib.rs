// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The input driver: the **fourth** device driver in this system that lives
//! outside the frame, and the first one that is on the input path.
//!
//! # What this is, said before anything else because the answer is easy to
//! misread
//!
//! **A virtio-input device under an emulator.** Not a USB HID stack, not an
//! i8042 controller, not a touchpad with a firmware protocol, and — the one that
//! matters for every number this path publishes — **not a device that stamps its
//! own events**. Hardware-timestamped native input is `E5-B06`. This line is
//! `E3-B04d`, and what it delivers is a driver *component* in the shape the
//! three E1 drivers already share, bound to the device QEMU offers, whose events
//! reach a consumer as `f_abi::input` entries.
//!
//! The distinction is not pedantry and it is the reason this paragraph is the
//! first one. `docs/TECHNICAL-DEBT.md` exists because a component that implies
//! more than it does costs the next reader a round; a driver named *input* that
//! let a reader believe the stamp on its entries came off a device would make
//! every latency number downstream of it a measurement of something nobody
//! chose. [`clock`] is where what the stamp actually is is written down, in
//! full, with its cost.
//!
//! # Why this crate exists, which is not the same as what it does
//!
//! Three drivers — `user/virtio-blk`, `user/virtio-net`, `user/virtio-gpu` —
//! established a shape: a component crate forbidding `unsafe`, a compiled
//! manifest, a scheduled ring-3 loop, registers reached through RFC 0033's safe
//! [`Window`](f_ring::device::Window), device translations asked of the frame
//! over control-ring opcodes (RFC 0047), and a routing page the frame writes and
//! the component reads. RFC 0071 then merged what was byte-identical between
//! their three supervisors into `kernel/src/supervisor.rs`.
//!
//! This is the first driver written **after** that merge, which makes it the
//! first evidence that the merge produced a shared half rather than a
//! coincidence three files deep. `kernel/src/input.rs` is this driver's
//! supervisor and it is short, because `Registers`, `Supervising`, `Declared`,
//! `declared` and `order_for` are taken rather than copied. A reviewer checking
//! the exit sentence's *rather than a fourth copy of it* is asked to read that
//! file and count what is in it.
//!
//! # The three things this crate is built to make true
//!
//! **`f_input::stamp::at_interrupt` has a caller that is not a test.** Until this
//! crate, it had none, and no crate in the workspace depended on `f-input` — so
//! *one time source in the whole input path* was a rule guarding a path with no
//! traffic on it. RFC 0099 narrowed the claim to what was measurable and named
//! this task as what would widen it again. [`clock::Interrupt::stamp`] is the
//! call, there is exactly one of it, and `cargo xtask lint-stamp` now counts
//! calls as well as definitions — so deleting it is a red build rather than a
//! quiet return to a vacuous green.
//!
//! **A driver with no `unsafe` can drive a device that only ever writes.** The
//! block, network and display drivers all hand the device memory it reads.
//! virtio-input's event queue is the other direction end to end: every descriptor
//! this driver posts is device-writable, the device fills it whenever the user
//! does something, and nothing is owed an answer. [`queue`] is where that shows
//! up — a queue that is refilled rather than drained — and [`driver`] is where
//! *a device wrote into this buffer* becomes *the user moved the mouse*.
//!
//! **The bytes of an input event do pass through this component, and that is
//! declared rather than hidden.** `user/virtio-input/manifest.toml` says
//! `payload = "inline"` and there is no `copies` counter in this crate claiming a
//! zero, because the zero would be false: a `virtio_input_event` is eight bytes
//! of `(type, code, value)` in an evdev vocabulary, and an `f_abi::input::Event`
//! is a different record in a different vocabulary. Something has to translate,
//! and the translator is the stage that holds the device. `cargo xtask
//! lint-datapath` has no row for this crate for exactly that reason; the
//! alternative — a row asserting a zero this driver cannot hold — would be the
//! check going green about a claim nobody made.
//!
//! # What this driver is not, listed rather than discovered
//!
//! One queue: the event queue. No status queue, so no LED or force-feedback
//! writes back to the device — [`queue::index::STATUS`] states what that costs.
//! No reading of the device configuration space, so no device name, no axis
//! ranges and no capability bitmaps — [`transport`] states the cost, which is
//! sharper here than on the display driver. No `EV_ABS`, so no tablet and no
//! touchscreen, and therefore no `f_abi::input::TouchPoint` and no
//! `f_abi::input::StylusPoint` on the wire from this driver; [`driver::Decoder`]
//! names both and says why neither is forged from a relative device. No
//! interrupt: the `notify` capability is declared and unused, exactly as it is on
//! the other three, and `E1-B09` is what turns it into a wait.
//!
//! # What runs where, stated rather than implied
//!
//! This crate is the driver **and it is scheduled**. [`component::start`] runs at
//! ring 3 on a core the frame allocated it, adopts its control ring and the ring
//! it submits on in safe code — `f_ring::adopt`, RFC 0037 — drives real registers
//! through mappings the frame made in answer to what its manifest declares, and
//! ends on a stop notice or on a bound it was told.
//!
//! It is the first driver in this tree that is a ring **client** rather than a
//! server. The other three are asked for something and answer; nobody asks for
//! an input event. So there is no executor here, no admission arithmetic and no
//! completion to post: there is a device that produces, and this component
//! submits what it produced to a peer that drains. `user/panel` is the other
//! client component and the shape is deliberately its shape rather than a
//! driver's.
//!
//! What is still owed is the block driver's sentence unchanged: this instance is
//! *scheduled* and not *spawned into a place*. `CHAOS_GAP` in xtask carries
//! exactly that difference and a fourth driver in the same position widens
//! nothing.

#![no_std]

pub mod clock;
pub mod driver;
// The input path's own prediction for `E3-B04e`'s seam, published and never
// sent. Outside the `image` gate because the frame reads the shape of what it
// publishes, as it reads `routing`. RFC 0134.
pub mod forecast;
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
// same gate every component crate in this tree has, with the same reversal: an
// AArch64 frame.
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
    /// **Fatal on purpose**, and the argument is the network driver's rather
    /// than the block driver's, because this device is in the same position:
    /// nothing is outstanding, nothing is owed, and the device writes when the
    /// *user* acts. A bus master without this bit is one writing wherever this
    /// driver's arithmetic said, at a moment nothing in this system chose, for
    /// as long as a buffer stays posted. `kernel/src/arch/x86_64/dma.rs` records
    /// what it cost to learn that such a device is architecturally outside the
    /// remapping unit — every isolation test passes, for the wrong reason. R04
    /// says refuse.
    NoPlatformAddressing,
    /// The device refused the feature set this driver offered, which is the one
    /// veto RFC 0011's shape gives a peer made of silicon.
    FeaturesRefused,
    /// The device reports no queues, or one too small to hold the events this
    /// driver posts buffers for.
    NoQueue,
    /// The device gave back an event chain reporting fewer bytes written than
    /// one `virtio_input_event` occupies.
    ///
    /// Its own variant rather than a bare code at the arithmetic that finds it,
    /// and that is the point of this enum: the `DEVICE` space is small and every
    /// number in it has to be distinguishable from every other, so a refusal
    /// packed by hand somewhere in this crate is a number nothing checks against
    /// the rest. R07, applied to the crate's own codes rather than only to a
    /// client's reading of them.
    ShortUsed,
    /// The device published a used element naming a chain this driver never
    /// posted.
    ///
    /// A device *steering* the driver rather than failing to answer, and a
    /// driver that followed it would decode eight bytes it never gave the device
    /// and submit the result as something the user did. That is worse here than
    /// on the network driver, where the same variant means a client's buffer is
    /// released early: an input event is a statement about a person's intent,
    /// and one the device did not make is one this system invented.
    Device,
    /// The peer this driver submits to has stopped speaking, or its ring has no
    /// room and the event would have to be dropped silently.
    ///
    /// Its own variant, and it is the one the other three drivers have no
    /// counterpart for: they *answer*, so a full ring is a completion they
    /// cannot post and a client that will notice. Nobody is waiting for an input
    /// event, so a full ring is an event that never happened as far as every
    /// later stage is concerned. Naming it is what makes `Counters::dropped` a
    /// number rather than an absence.
    NoRoom,
}

impl Trouble {
    /// The refusal a peer reads.
    ///
    /// `DEVICE` for everything the hardware decided, which is RFC 0010's domain
    /// for a hardware failure, with the detail being *which* — a peer told only
    /// `DEVICE` could not tell a device that never answered from one that
    /// refused a feature set. `ARGUMENT/BAD_ADDRESS` for the one that is this
    /// component's own arithmetic, because a caller that named the wrong place
    /// can name a different one, and `PEER/GONE` for the ring.
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
            Self::NoRoom => f_abi::error::pack(f_abi::error::PEER, f_abi::error::peer::GONE),
        }
    }

    /// A sentence for a boot log.
    ///
    /// The frame prints these; a scheduled driver has no serial port. Both this
    /// and [`Trouble::packed`] exist because they answer different readers, and
    /// the day one of them is the only reader, the other method goes.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Layout => "a granted region is not the shape the event queue needs",
            Self::Register(_) => "an accessor refused an offset outside a granted window",
            Self::NotResponding => "the device did not come out of reset",
            Self::NoPlatformAddressing => {
                "the device does not offer platform addressing, so it would bypass the \
                 remapping unit"
            }
            Self::FeaturesRefused => "the device refused the features this driver offered",
            Self::NoQueue => "the device reports no event queue this driver can use",
            Self::ShortUsed => "the device reported an event shorter than one evdev record",
            Self::Device => "the device gave back a chain this driver never posted",
            Self::NoRoom => "the peer this driver submits to is not taking entries",
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
        // two different failures, is a refusal a peer has to guess at.
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
            Trouble::NoRoom,
        ];
        for trouble in troubles {
            let packed = trouble.packed();
            let (domain, _) = f_abi::error::unpack(packed).expect("a refusal is negative");
            assert!(
                domain == f_abi::error::DEVICE
                    || domain == f_abi::error::ARGUMENT
                    || domain == f_abi::error::PEER,
                "a driver refuses on its own arithmetic, on the hardware, or on its peer"
            );
            assert!(!trouble.message().is_empty());
        }

        // The six hardware failures are distinguishable from each other, which
        // is the half of R07 a single `DEVICE` domain would lose.
        let hardware = [
            Trouble::NotResponding,
            Trouble::NoPlatformAddressing,
            Trouble::FeaturesRefused,
            Trouble::NoQueue,
            Trouble::ShortUsed,
            Trouble::Device,
        ];
        for (index, one) in hardware.iter().enumerate() {
            for other in hardware.iter().skip(index + 1) {
                assert_ne!(one.packed(), other.packed());
            }
        }
    }

    #[test]
    fn an_accessor_refusal_reaches_a_peer_unchanged() {
        // Passed through rather than translated, for the reason
        // `kernel/src/iommu.rs` gives about the same boundary: a refusal this
        // component invented a code for is a refusal a peer cannot act on.
        let bad = f_abi::error::pack(f_abi::error::ARGUMENT, f_abi::error::argument::BAD_ADDRESS);
        assert_eq!(Trouble::from(bad).packed(), bad);
    }

    #[test]
    fn the_four_drivers_do_not_share_a_routing_magic() {
        // Every component is mapped at the same board address by the one driver
        // shape the frame builds, so the magic is the only thing that says which
        // board a component is reading. A build that routed the wrong image into
        // the wrong supervisor has to find a refusal there rather than a page
        // whose fields mean something else.
        //
        // Asserted here rather than in `routing`, because the values being
        // compared against are *numbers* and not the other crates' constants:
        // this crate does not depend on the other three drivers and must not
        // start. `kernel/src/input.rs` is where the definitions are linked
        // together and where a compile-time assertion can name them all.
        assert_ne!(routing::MAGIC, 0x626C_6B5F_726F_7574, "virtio-blk's routing magic");
        assert_ne!(routing::MAGIC, 0x6E65_745F_726F_7574, "virtio-net's routing magic");
        assert_ne!(routing::MAGIC, 0x6770_755F_726F_7574, "virtio-gpu's routing magic");
    }
}
