// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The labels the client and every peer share, and the one test that keeps them
//! inside the trace's columns.
//!
//! # Why the protocol has a module of its own
//!
//! Because it belongs to neither end. A client that owned the message kinds
//! would be a client the peers imported *from*, which reads as the peers being
//! written against one client rather than against a protocol — and component
//! substitution is exactly the claim that they are not. Putting the words here
//! means the modelled devices and the native peer are two implementations of one
//! thing, and the client cannot tell which it has.
//!
//! # Why the widths are checked in one place
//!
//! A trace line is fixed-width, so a label wider than [`crate::LABEL_WIDTH`]
//! shifts every field after it and makes two otherwise identical runs disagree
//! — and the value that shifts it is by definition the one nobody tested with.
//! [`tests::every_label_fits_the_trace_column`] holds the whole set at once,
//! which is what stops the check from being something each new label's author
//! has to remember.

/// What one actor says to another.
///
/// A message is the *occurrence* of something; the thing itself, when it is an
/// ABI entry, travels on [`crate::wire::Wire`]. See that module for why the two
/// are separate.
pub mod kind {
    /// Begin. Sent to each client at the start of the run.
    pub const START: &str = "start";
    /// Client to peer: there is a submission for you on the wire.
    pub const SUBMIT: &str = "submit";
    /// Peer to client: there is a completion for you on the wire.
    pub const CQE: &str = "cqe";
    /// Client to itself: the back-off after a refusal has elapsed.
    pub const RETRY: &str = "retry";
    /// Peer to client: I restarted, so every token you hold is stale.
    ///
    /// The one event that lets a client take back a buffer with no completion —
    /// `f_ring::buffers::PeerGone` is the evidence and RFC 0008 is why it is
    /// sound. A model that dropped completions without ever sending this would
    /// leave a client holding buffers it could never reclaim, which is a hang
    /// dressed as a quiet trace.
    pub const GONE: &str = "gone";

    /// Peer to itself: the device notices the doorbell.
    ///
    /// A separate event and not a continuation of [`SUBMIT`], because the gap
    /// between a driver publishing an available index and a device reading it
    /// is where the interesting orderings live — and a model that closed the
    /// gap would explore none of them.
    pub const POLL: &str = "poll";
    /// Peer to itself: one request's service time has elapsed.
    pub const SERVICE: &str = "service";
    /// Peer to itself: the driver half harvests the used ring.
    pub const REAP: &str = "reap";
}

/// What an actor writes into the trace.
pub mod wrote {
    /// A client asked for a buffer set.
    pub const REGISTER: &str = "register";
    /// A client bound its set and carved it. From here it can submit.
    pub const BOUND: &str = "bound";
    /// A client put an operation on the wire.
    pub const ISSUE: &str = "issue";
    /// A client saw one of its operations complete.
    pub const DONE: &str = "done";
    /// A client was refused and will try again.
    pub const REFUSED: &str = "refused";
    /// A client took a buffer back without a completion, because none is
    /// coming.
    pub const RECLAIM: &str = "reclaim";
    /// A client has nothing left to issue and nothing outstanding.
    pub const FINISHED: &str = "finished";
    /// The ring had no room, so the buffer came straight back.
    pub const FULL: &str = "full";

    /// A peer took a submission and put a chain on its queue.
    pub const QUEUED: &str = "queued";
    /// A peer refused a submission outright.
    ///
    /// Distinct from [`REFUSED`], which is what the *client* writes when it
    /// reads the refusal: two records for one event, one at each end, because a
    /// refusal that only the refuser wrote down is a refusal nothing proves the
    /// client acted on.
    pub const DENIED: &str = "denied";
    /// The device took a chain off the available ring.
    pub const TAKEN: &str = "taken";
    /// The device finished one and published a used entry.
    pub const SERVED: &str = "served";
    /// The device finished one and did **not** publish it.
    ///
    /// Written rather than silent, and that is the whole discipline: a
    /// simulator that quietly dropped work would produce a trace that
    /// reproduces perfectly and describes nothing.
    pub const DROPPED: &str = "dropped";
    /// The device published a completion and held the notification back, so the
    /// driver will see this one and the next together.
    pub const HELD: &str = "held";
    /// The peer restarted after losing work.
    pub const RESET: &str = "reset";

    /// The device was asked for something its protocol does not define.
    pub const UNSUPP: &str = "unsupp";
    /// The device refused the request on its own terms — a sector past the
    /// disk, a resource that was never created.
    pub const IOERR: &str = "ioerr";
    /// A descriptor named an address the device's domain does not translate.
    ///
    /// The model's stand-in for the fault `kernel/src/arch/x86_64/dma.rs`
    /// provokes on real silicon.
    pub const NOREACH: &str = "noreach";
    /// A request carrying a fence, which may not be reordered past another one.
    pub const FENCED: &str = "fenced";
    /// The link is down, so a transmitted frame went nowhere.
    pub const LINKDOWN: &str = "linkdown";
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::LABEL_WIDTH;
    use crate::dev::Protocol;

    /// Every label the protocol and the models between them ship.
    ///
    /// `actors.rs` has its own list for the two actors stage one shipped; this
    /// is the list for everything that speaks the ring protocol, and the two are
    /// separate because the two sets are separate. A label in neither list is a
    /// label nothing checks, which is why every module that mints one adds it
    /// here.
    const LABELS: &[&str] = &[
        kind::START,
        kind::SUBMIT,
        kind::CQE,
        kind::RETRY,
        kind::GONE,
        kind::POLL,
        kind::SERVICE,
        kind::REAP,
        wrote::REGISTER,
        wrote::BOUND,
        wrote::ISSUE,
        wrote::DONE,
        wrote::REFUSED,
        wrote::RECLAIM,
        wrote::FINISHED,
        wrote::FULL,
        wrote::QUEUED,
        wrote::DENIED,
        wrote::TAKEN,
        wrote::SERVED,
        wrote::DROPPED,
        wrote::HELD,
        wrote::RESET,
        wrote::UNSUPP,
        wrote::IOERR,
        wrote::NOREACH,
        wrote::FENCED,
        wrote::LINKDOWN,
        crate::client::App::NAME,
        crate::blk::Blk::NAME,
        crate::net::Net::NAME,
        crate::gpu::Gpu::NAME,
        crate::native::Native::NAME,
    ];

    #[test]
    fn every_label_fits_the_trace_column() {
        for label in LABELS {
            assert!(
                label.len() <= LABEL_WIDTH,
                "`{label}` is {} bytes and the column is {LABEL_WIDTH}",
                label.len()
            );
        }
    }

    #[test]
    fn no_two_labels_are_the_same_word() {
        // Two labels with one spelling is two events a reader of a trace cannot
        // tell apart, and a property a test asserts on one of them would pass
        // for the other. Cheap to check once, impossible to notice by eye.
        let mut sorted = LABELS.to_vec();
        sorted.sort_unstable();
        let before = sorted.len();
        sorted.dedup();
        assert_eq!(before, sorted.len(), "two labels share a spelling");
    }
}
