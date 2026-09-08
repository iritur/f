// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Starting a topology, and what a failure costs: its subtree and nothing else.
//!
//! # The sentence this module implements
//!
//! *A driver that fails to start leaves its subtree unstarted rather than
//! failing the boot.* Three claims, and each is a separate thing that could be
//! got wrong:
//!
//! - **its subtree** — the components that route a capability from it,
//!   transitively, and no others. `Assembly::dependents` is the definition and
//!   it is stated once;
//! - **unstarted** — [`State::Unstarted`], which carries *which* component's
//!   failure stopped it. A state that only said *not started* would leave the
//!   operator to work the cause out from a graph, which is exactly the work an
//!   assembler exists to have already done;
//! - **rather than failing the boot** — [`start`] returns a [`Report`] and
//!   never an error. There is no path out of this function that abandons the
//!   topology, and that is the whole design rather than an omission: a boot
//!   that dies because one card is missing is the failure mode RFC 0008's
//!   places exist to remove, one level up.
//!
//! # Why the trait, and why it is not a callback
//!
//! There is no supervisor component to submit `f_abi::control::op::SPAWN` yet —
//! `kernel/src/component.rs` says so at length and `cargo xtask lint-owed`
//! holds it. So what this crate owns is the *decision*: which components, in
//! which order, bound to which device. Performing a spawn is injected.
//!
//! That is the distinction `cargo xtask lint-callbacks` already draws for
//! `f_env::Env` — a trait the system implements and this code calls into is
//! dependency injection, and nothing a peer sends registers anything. R05 is
//! about an interface that lets a *peer* hand over code; this is not one.
//!
//! # Why a driver with no device is a failure and not a refusal
//!
//! Because a machine missing a card is an ordinary machine, and the answer that
//! serves the operator is a topology that came up with a hole in it and a state
//! saying which hole. Refusing the whole instantiation would make one absent
//! device unbootable — which is the sentence above, inverted.

use alloc::vec::Vec;

use f_abi::manifest::Record;

use crate::bind::{Address, wanted_and_absent};
use crate::topology::Assembly;

/// What happened to one component.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Nothing has been attempted yet. The state every member has after
    /// instantiation, so that a topology rendered before a start is a truthful
    /// document rather than one claiming everything failed.
    Untried,
    /// It started.
    Started,
    /// It was tried and did not start. Carries what the starter said.
    /// Unit of the payload: none — a packed `f_abi::error`, as the starter
    /// returned it.
    Failed(i32),
    /// A driver it needs did not start, so it was never tried. Carries the
    /// index of the component whose failure stopped it — the *nearest* one, not
    /// the root of the whole chain, because that is the edge an operator can
    /// act on.
    /// Unit of the payload: index into the topology's member list.
    Unstarted(u16),
    /// It declares a device and the bus does not have it, so there was nothing
    /// to start it against.
    ///
    /// Told apart from [`State::Failed`] deliberately: a driver that failed is
    /// a driver that is wrong, and a driver with no device is a *machine*
    /// without the card. Conflating them would send somebody to read a driver's
    /// code about a missing card.
    NoDevice,
}

impl State {
    /// Did this component start?
    #[must_use]
    pub const fn started(self) -> bool {
        matches!(self, Self::Started)
    }

    /// A word for a log.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Untried => "untried",
            Self::Started => "started",
            Self::Failed(_) => "failed",
            Self::Unstarted(_) => "unstarted",
            Self::NoDevice => "no-device",
        }
    }

    /// The byte this state is rendered as.
    ///
    /// Stated here rather than in `render.rs` so that the encoding of a state
    /// and the state live in one file: a renderer that assigned its own numbers
    /// would be a second opinion about what a topology is, which is the thing
    /// the byte-identity claim rests on there not being.
    /// Unit: none — an ordinal. Zero is not a state.
    #[must_use]
    pub const fn wire(self) -> u8 {
        match self {
            Self::Untried => 1,
            Self::Started => 2,
            Self::Failed(_) => 3,
            Self::Unstarted(_) => 4,
            Self::NoDevice => 5,
        }
    }
}

/// The thing that actually starts a component.
///
/// One method, taking what a spawn needs and returning what a spawn can refuse
/// with. The implementation is a supervisor; today, in this tree, it is a test
/// and the frame's own `kernel::component::spawn` is what a real one would
/// reach.
pub trait Start {
    /// Start one component, bound to the device it was matched with.
    ///
    /// # Errors
    ///
    /// A packed `f_abi::error`, which becomes [`State::Failed`]. What it means
    /// is the implementation's business: this crate does not interpret it, it
    /// records it and stops walking that subtree.
    fn start(&mut self, index: u16, record: &Record, at: Option<Address>) -> Result<(), i32>;
}

/// What a start pass did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// How many components started. Unit: components.
    pub started: usize,
    /// How many were tried and failed. Unit: components.
    pub failed: usize,
    /// How many declared a device the bus did not have. Unit: components.
    pub absent: usize,
    /// How many were never tried because something they need did not start.
    /// Unit: components.
    pub unstarted: usize,
}

impl Report {
    /// Did every component start?
    ///
    /// A question and never a gate: nothing in this crate refuses to continue
    /// because the answer is `false`, which is the point.
    #[must_use]
    pub const fn whole(&self) -> bool {
        self.failed == 0 && self.absent == 0 && self.unstarted == 0
    }
}

/// Start every component the topology names, in the order it computed.
///
/// Returns a [`Report`] and never an error, because there is no failure of one
/// component that is a failure of the boot. A component is tried when every
/// component it routes a capability from has started; when one has not, it is
/// marked [`State::Unstarted`] with that component's index and the walk goes on
/// to the next member.
///
/// The order is `Assembly::order`, which was fixed at instantiation and is a
/// function of the root. Nothing here chooses an order, which is why a failure
/// cannot change which components a *successful* run would have started or in
/// what sequence — the state changes, the plan does not.
pub fn start(assembly: &mut Assembly, starter: &mut impl Start) -> Report {
    let mut report = Report::default();
    let order: Vec<u16> = assembly.order().to_vec();

    for index in order {
        // The nearest source that did not start. Canonical order, so a
        // component waiting on two failures names the lower-indexed one — a
        // choice, made here rather than left to iteration order, because the
        // rendered topology carries it.
        let blocker = assembly
            .sources(index)
            .into_iter()
            .find(|source| !assembly.member(*source).is_some_and(|m| m.state.started()));

        let state = if let Some(source) = blocker {
            report.unstarted += 1;
            State::Unstarted(source)
        } else if wanted_and_absent(assembly, index) {
            report.absent += 1;
            State::NoDevice
        } else {
            let Some(member) = assembly.member(index) else { continue };
            let (record, at) = (member.record, member.bound);
            match starter.start(index, &record, at) {
                Ok(()) => {
                    report.started += 1;
                    State::Started
                }
                Err(why) => {
                    report.failed += 1;
                    State::Failed(why)
                }
            }
        };

        if let Some(member) = assembly.member_mut(index) {
            member.state = state;
        }
    }
    report
}
