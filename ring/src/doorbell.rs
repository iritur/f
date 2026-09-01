// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The doorbell: one function, three implementations, chosen once.
//!
//! # Why this is a type and not a call
//!
//! `docs/design/ring-scene-boot.html` section 03 asks for exactly this and says
//! why: *abstract the doorbell behind one function with three implementations —
//! user interrupt, kernel IPI, pure polling — selected at channel creation from
//! what the hardware actually reports. Then measure all three.* The measurement
//! is the point. The delta between them is one of the cleanest results this
//! system can produce, and it is only a result if the three paths are the same
//! path with one thing changed.
//!
//! So the suppression protocol lives here, once, and a [`Path`] chooses only
//! *how* the signal is delivered. A build with three suppression protocols
//! would be measuring three systems.
//!
//! # Why the counters are here and not in the mapping
//!
//! Because they are the sender's own facts. Doorbells per operation is a number
//! about what this end decided to do, and the obvious place to put it — the
//! four reserved words in [`ChannelHeader`](f_abi::ChannelHeader) — is memory
//! the peer writes. Evidence of delivery that a peer can forge is not evidence,
//! and it would also have cost an ABI version to add a field that never needed
//! to cross the boundary at all.
//!
//! # What negotiation decides, and what it does not
//!
//! The user-interrupt path is behind `feature::USER_INTERRUPT_DOORBELL`, and
//! the machinery for that already existed: a peer that *requires* it and a
//! local side that does not offer it never gets a channel — RFC 0011 refuses at
//! `negotiate`, before any of this runs. What is left for [`Path::select`] is
//! the case where both sides *offer* it, which is a preference, and the case
//! where the local hardware turns out not to have it after all, which is a
//! fact. A feature bit is a statement about the protocol; it is not a statement
//! about the silicon, and conflating the two is how a channel gets negotiated
//! into an instruction that faults.

use f_abi::{Negotiated, feature};

/// How a doorbell is delivered on this channel.
///
/// Ordered by cost, cheapest last, which is also the order [`Path::select`]
/// prefers them in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Path {
    /// No doorbell. The consumer never sleeps, so nothing ever needs waking.
    ///
    /// Not a degraded mode: under load the suppression protocol drives every
    /// channel here, and a deadline-class consumer is meant to stay here
    /// permanently.
    Polling,
    /// A kernel inter-processor interrupt. One to two microseconds, and worse,
    /// a *variable* one to two microseconds.
    KernelIpi,
    /// A user-level interrupt. The sending core writes a target register and
    /// the receiving thread's handler runs with no kernel transition on either
    /// side — and the number that matters is not that it is faster but that it
    /// is bounded.
    UserInterrupt,
}

/// What the local machine can actually do, as distinct from what was agreed.
///
/// Two fields rather than one, because they fail differently. A machine with no
/// user interrupts has a slower doorbell; a machine that cannot interrupt
/// another core has no doorbell at all and its consumers must poll.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Hardware {
    /// The processor implements user-level interrupt delivery, and this build
    /// has set it up.
    pub user_interrupts: bool,
    /// One core can interrupt another.
    pub cross_core_interrupts: bool,
}

impl Path {
    /// Choose the path for a channel, from what was agreed and what exists.
    ///
    /// The order is deliberate and it is not "best available": a feature bit
    /// that both sides merely *offer* is a preference, and it is honoured only
    /// where the hardware backs it. A peer that could not proceed without user
    /// interrupts never reached here — `negotiate` refused the channel at
    /// setup, which is where a requirement belongs.
    #[must_use]
    pub fn select(agreed: Negotiated, hardware: Hardware) -> Self {
        let offered = agreed.features & feature::USER_INTERRUPT_DOORBELL != 0;
        if offered && hardware.user_interrupts {
            return Self::UserInterrupt;
        }
        if hardware.cross_core_interrupts {
            return Self::KernelIpi;
        }
        Self::Polling
    }
}

/// What actually rings.
///
/// One method, because the whole design argument is that the three paths differ
/// in cost and in nothing else. A trait rather than a function pointer so the
/// implementation can carry the target it rings — an APIC identifier, a
/// user-interrupt index — without this module knowing what either is.
pub trait Ringer {
    /// Signal the consumer. Called only when it has asked to be signalled.
    fn ring(&mut self);
}

/// A doorbell that will not ring: the [`Path::Polling`] implementation.
///
/// Not a stub. It is what a channel whose consumer never sleeps actually uses,
/// and having it be a real implementation rather than an `Option` is what lets
/// the suppression test drive all three paths through one body.
#[derive(Clone, Copy, Debug, Default)]
pub struct Silent;

impl Ringer for Silent {
    fn ring(&mut self) {}
}

/// The doorbell for one channel, and the two counts that make it measurable.
pub struct Bell<R> {
    path: Path,
    ringer: R,
    rings: u64,
    operations: u64,
}

impl<R: Ringer> Bell<R> {
    /// Bind a doorbell to a path.
    ///
    /// # Errors
    ///
    /// [`Path::UserInterrupt`] on a build that has not established the
    /// hardware supports it. Refused rather than silently downgraded: a channel
    /// that negotiated a feature and then quietly did something else is two
    /// peers with different beliefs about what just happened, which is the
    /// failure R04 exists to prevent one layer down.
    pub fn new(path: Path, hardware: Hardware, ringer: R) -> Result<Self, Path> {
        if path == Path::UserInterrupt && !hardware.user_interrupts {
            return Err(Path::UserInterrupt);
        }
        Ok(Self { path, ringer, rings: 0, operations: 0 })
    }

    /// Account one submission, and ring if the producer said to.
    ///
    /// `wanted` is what [`Producer::submit`](crate::Producer::submit) and
    /// [`Batch::publish`](crate::Batch::publish) return: the consumer's own
    /// statement that it is about to sleep, read after the fence RFC 0020 put
    /// between the publish and the flag.
    ///
    /// A batch is **one** operation for this count and at most one ring, which
    /// is the whole reason batching exists and the reason the number is worth
    /// publishing: doorbells per operation is what suppression is for, and a
    /// count that charged a batch per entry would report suppression working
    /// when it was only batching.
    pub fn submitted(&mut self, wanted: bool) {
        self.operations += 1;
        if wanted {
            self.rings += 1;
            self.ringer.ring();
        }
    }

    /// The path this channel chose.
    #[must_use]
    pub fn path(&self) -> Path {
        self.path
    }

    /// Doorbells sent. Unit: doorbells.
    #[must_use]
    pub fn rings(&self) -> u64 {
        self.rings
    }

    /// Operations accounted. Unit: operations, where a published batch is one.
    #[must_use]
    pub fn operations(&self) -> u64 {
        self.operations
    }

    /// Doorbells per thousand operations.
    ///
    /// Per thousand rather than a ratio because this crate is `no_std` and has
    /// no floating point, and because the number the design cares about is
    /// *zero under load* — an integer that reaches zero says that more clearly
    /// than a rounded fraction does.
    /// Unit: doorbells per thousand operations.
    #[must_use]
    pub fn per_thousand(&self) -> u64 {
        self.rings.saturating_mul(1000).checked_div(self.operations).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Consumer, Producer};
    use f_abi::Sqe;

    struct Counting(u64);

    impl Ringer for Counting {
        fn ring(&mut self) {
            self.0 += 1;
        }
    }

    fn agreed(features: u64) -> Negotiated {
        Negotiated { version: f_abi::ABI_VERSION, features }
    }

    #[test]
    fn a_feature_both_sides_offer_is_honoured_only_where_the_hardware_backs_it() {
        let both = agreed(feature::USER_INTERRUPT_DOORBELL);
        let has = Hardware { user_interrupts: true, cross_core_interrupts: true };
        let hasnt = Hardware { user_interrupts: false, cross_core_interrupts: true };

        assert_eq!(Path::select(both, has), Path::UserInterrupt);
        // The case this machine is in, and the reason the two are separate
        // questions: the bit says what the protocol permits and the hardware
        // says what the instruction does.
        assert_eq!(Path::select(both, hasnt), Path::KernelIpi);
        assert_eq!(Path::select(agreed(0), has), Path::KernelIpi);
        assert_eq!(Path::select(agreed(0), Hardware::default()), Path::Polling);
    }

    #[test]
    fn a_path_the_hardware_cannot_take_is_refused_and_never_downgraded() {
        let hasnt = Hardware { user_interrupts: false, cross_core_interrupts: true };
        assert_eq!(Bell::new(Path::UserInterrupt, hasnt, Silent).err(), Some(Path::UserInterrupt));
        assert!(Bell::new(Path::KernelIpi, hasnt, Silent).is_ok());
    }

    /// One body, every path. The exit criterion for `E0-B15` is that *both
    /// paths pass the same suppression test*, and the only way to be sure of
    /// that is for there to be one test that takes the path as an argument —
    /// two tests written from one description drift the first time somebody
    /// edits one of them.
    fn suppression_holds_on(path: Path, hardware: Hardware) {
        let backing = crate::tests::Backing::<8>::new();
        let producer = Producer::new(backing.chan()).expect("a power-of-two ring");
        let consumer = Consumer::new(backing.chan()).expect("a power-of-two ring");
        let mut bell = Bell::new(path, hardware, Counting(0)).expect("a path this build can take");

        // A draining consumer is never rung, however many entries arrive. This
        // is the property under load, and it is the one that has to hold on
        // every path identically.
        consumer.disarm_wakeup();
        for _ in 0..4 {
            let wanted = producer.submit(Sqe::ZERO).expect("a healthy ring");
            bell.submitted(wanted);
            consumer.pop().expect("a healthy ring");
        }
        assert_eq!(bell.rings(), 0, "{path:?}: a draining consumer was rung");
        assert_eq!(bell.per_thousand(), 0, "{path:?}: suppression did not reach zero");

        // A sleeping one is rung exactly once per submission it did not see.
        consumer.arm_wakeup();
        let wanted = producer.submit(Sqe::ZERO).expect("a healthy ring");
        bell.submitted(wanted);
        assert_eq!(bell.rings(), 1, "{path:?}: a sleeping consumer was not rung");

        // And the ringer was actually called, which is the half an accounting
        // bug would pass: a `Bell` that counted without ringing would satisfy
        // every assertion above.
        //
        // `Counting` on all three paths, deliberately. What is being asserted
        // is that the protocol is identical and that a path changes only what
        // `ring` *does* — `Path::Polling`'s real implementation is `Silent`,
        // and the assertion below is that nothing else about the polling path
        // is different. A test that gave Polling a silent ringer here would be
        // asserting the protocol differs, which is what this exists to deny.
        assert_eq!(bell.ringer.0, 1, "{path:?}: the ringer was not actually called");
        assert_eq!(bell.operations(), 5);
        assert_eq!(bell.per_thousand(), 200);
    }

    #[test]
    fn the_polling_path_is_silent_because_its_ringer_is() {
        // The other half of the sentence above. Nothing in `Bell` tests the
        // path before ringing; what makes polling free is that its ringer does
        // nothing, which is one branch fewer on the submission path than a
        // check would have been.
        let hardware = Hardware::default();
        let mut bell = Bell::new(Path::Polling, hardware, Silent).expect("polling needs nothing");
        bell.submitted(true);
        assert_eq!(bell.rings(), 1, "the count is of what the protocol decided");
        assert_eq!(bell.path(), Path::Polling);
    }

    #[test]
    fn every_path_passes_the_same_suppression_test() {
        suppression_holds_on(Path::Polling, Hardware::default());
        suppression_holds_on(
            Path::KernelIpi,
            Hardware { user_interrupts: false, cross_core_interrupts: true },
        );
        suppression_holds_on(
            Path::UserInterrupt,
            Hardware { user_interrupts: true, cross_core_interrupts: true },
        );
    }

    #[test]
    fn a_batch_is_one_operation_and_at_most_one_doorbell() {
        // The distinction the published number rests on. Charging a batch per
        // entry would make doorbells-per-operation fall as batch size rose and
        // report that as suppression working, when what was working was
        // batching. Two mechanisms, one number, and only one of them is what
        // the number is about.
        let backing = crate::tests::Backing::<8>::new();
        let mut producer = Producer::new(backing.chan()).expect("a power-of-two ring");
        let consumer = Consumer::new(backing.chan()).expect("a power-of-two ring");
        let mut bell = Bell::new(
            Path::KernelIpi,
            Hardware { user_interrupts: false, cross_core_interrupts: true },
            Counting(0),
        )
        .expect("a path this build can take");

        consumer.arm_wakeup();
        let mut batch = producer.batch();
        for _ in 0..4 {
            batch.push(Sqe::ZERO).expect("room for four");
        }
        let wanted = batch.publish().expect("a healthy ring");
        bell.submitted(wanted);

        assert_eq!(bell.operations(), 1, "four entries in one batch is one operation");
        assert_eq!(bell.rings(), 1, "one publish is at most one doorbell");
        assert_eq!(bell.ringer.0, 1);
    }
}
