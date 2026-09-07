// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Two record trees, and the first place they stop agreeing.
//!
//! # Why this exists rather than a hash comparison
//!
//! Because *two roots differed* is not a finding anybody can act on, and a
//! weekly job that reports one is a job whose output is a second investigation.
//! `xtask/src/generation.rs` already prints every leaf beside its name for that
//! reason; this is the same argument one step further on, where the comparison
//! is made by a machine on two artefacts rather than by a person on two logs.
//!
//! The descent is **leaves before roots**, in fold order, and that order is the
//! whole design: a leaf that moved names *the input that moved* — a component
//! file, or the frame image — and a root that moved with every leaf standing
//! still names the compiler instead. Those are two different findings with two
//! different fixes and only one of them is about the build being reproducible.
//!
//! # Why not `Option<Leaf>`
//!
//! `intent/0006-state/plan.md` writes this as `divergence(&Tree, &Tree) ->
//! Option<Leaf>`, and that signature cannot express the case the job exists to
//! separate: every leaf agreeing while the roots do not. There is no leaf to
//! return there, so a `None` would say *these two trees agree* about a pair that
//! demonstrably does not — which is the one wrong answer this function must not
//! give. [`Divergence`] is that signature widened by exactly the cases a leaf
//! cannot carry, and nothing else: a leaf, a membership, a route, the topology's
//! own head, and the compiler.
//!
//! # What it does not do
//!
//! Decide. Nothing here knows which of the two trees is right, and the variants
//! carry both sides for that reason: a job comparing runner *a* with runner *b*
//! has no basis for calling either one the deviation, and a function that
//! guessed would be inviting its caller to fix the wrong machine.

use f_abi::store::{Leaf, Route};

use crate::fold;
use crate::record::Tree;

/// The first place two record trees stop agreeing.
///
/// Ordered by the descent that produces it, which is fold order: the frame
/// leaf, the members, the routes, the topology's head, the root. A caller that
/// prints these in the order the enum declares them is printing them in the
/// order a reader should read them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Divergence {
    /// One leaf, present in both trees under the same name, naming two
    /// different contents. This is the finding the job is for: the leaf's name
    /// *is* the non-reproducible input.
    Leaf {
        /// The leaf as the left tree has it.
        left: Leaf,
        /// The leaf as the right tree has it.
        right: Leaf,
    },
    /// The two topologies do not name the same components. Not a leaf
    /// divergence, because there is no pair to compare: one side has a member
    /// the other has not, or has them in a different place. `None` on a side
    /// means that side ran out of members first.
    Membership {
        /// The member the left tree has at this position, if it has one.
        left: Option<Leaf>,
        /// The member the right tree has at this position, if it has one.
        right: Option<Leaf>,
    },
    /// Every leaf agrees and a route does not. A route is a field of the
    /// topology rather than a child of it, which is why moving one changes the
    /// topology's hash and nobody's leaf — so it has to be looked for by hand
    /// here, and would otherwise fall through to [`Divergence::Compiler`] and
    /// blame the wrong thing.
    Route {
        /// Where in canonical order, so the caller can say which one.
        /// Unit: index into the topology's route list, zero-based.
        index: u16,
        /// The route as the left tree has it, if it has one there.
        left: Option<Route>,
        /// The route as the right tree has it, if it has one there.
        right: Option<Route>,
    },
    /// Every member and every route agrees and the topology's stored head does
    /// not. Only the two counts live there, and they are implied by what has
    /// already been compared — so reaching this is a statement about the
    /// *encoder* rather than about the topology, and it is a variant rather
    /// than an `unreachable!()` because this crate is `no_std` and a panic in a
    /// comparison is a worse answer than a name.
    Head,
    /// Everything below the root agrees and the root does not.
    ///
    /// Nothing was fed two different inputs, so the fold itself is what
    /// differed: two builds of the compiler that do not agree about the
    /// arithmetic, or a `Tree` whose bytes carry something outside every field
    /// the descent above reads. That is a defect in this workspace and not a
    /// non-reproducible input, and the job says so in those words rather than
    /// naming an input it cannot name.
    Compiler,
}

/// Where two checked trees first stop agreeing, or `None` if their roots are
/// equal.
///
/// `None` is the strong answer and it is deliberately taken from the root
/// rather than from having run out of things to compare: two trees that agree
/// everywhere the descent looks and disagree at the root are a
/// [`Divergence::Compiler`], not an agreement. Fail towards naming something.
#[must_use]
pub fn divergence(left: &Tree<'_>, right: &Tree<'_>) -> Option<Divergence> {
    if fold::root(left) == fold::root(right) {
        return None;
    }

    // The frame first, because it is the generation node's first child and
    // because it is the leaf most likely to be the answer: it is the only
    // content in the tree that is built with debug information in it, and
    // therefore the only one that carried the checkout path before
    // `.cargo/config.toml` remapped it away.
    let (frame_left, frame_right) = (left.frame(), right.frame());
    if fold::leaf(&frame_left) != fold::leaf(&frame_right) {
        return Some(Divergence::Leaf { left: frame_left, right: frame_right });
    }

    // The members, in canonical order, which is the order both trees are
    // already required to be in — so position *is* identity here and a
    // positional walk cannot silently compare two different components.
    let members = left.members().max(right.members());
    for index in 0..members {
        match (left.member(index), right.member(index)) {
            (Some(a), Some(b)) if a.name == b.name => {
                if fold::leaf(&a) != fold::leaf(&b) {
                    return Some(Divergence::Leaf { left: a, right: b });
                }
            }
            (a, b) => return Some(Divergence::Membership { left: a, right: b }),
        }
    }

    let routes = left.routes().max(right.routes());
    for index in 0..routes {
        let (a, b) = (left.route(index), right.route(index));
        let same = match (a, b) {
            (Some(a), Some(b)) => a.to_bytes() == b.to_bytes(),
            _ => false,
        };
        if !same {
            return Some(Divergence::Route { index, left: a, right: b });
        }
    }

    if left.head().to_bytes() != right.head().to_bytes() {
        return Some(Divergence::Head);
    }

    Some(Divergence::Compiler)
}

impl Divergence {
    /// The name of the thing that moved, if this divergence has one.
    ///
    /// Not a formatted line: this crate is `no_std` and has no allocator, so
    /// what it can hand back is the padded name out of the record and nothing
    /// more. Rendering it — trimming the NULs, putting it beside two hashes and
    /// a sentence — is `xtask`'s, where the strings are.
    #[must_use]
    pub const fn named(&self) -> Option<&[u8]> {
        match self {
            Self::Leaf { left, .. } => Some(&left.name),
            Self::Membership { left: Some(leaf), .. }
            | Self::Membership { right: Some(leaf), .. } => Some(&leaf.name),
            Self::Route { left: Some(route), .. } | Self::Route { right: Some(route), .. } => {
                Some(&route.capability)
            }
            Self::Membership { .. } | Self::Route { .. } | Self::Head | Self::Compiler => None,
        }
    }

    /// Is this a divergence that names an *input*, rather than one that names
    /// this workspace?
    ///
    /// The distinction the weekly job reports on. A leaf or a membership means
    /// two runs were handed different bytes and the fix is in whatever produced
    /// them; [`Divergence::Head`] and [`Divergence::Compiler`] mean the two runs
    /// were handed the same bytes and folded them differently, which is a defect
    /// in `generation/` or in `abi/src/store.rs` and is nobody's build
    /// environment.
    #[must_use]
    pub const fn names_an_input(&self) -> bool {
        matches!(self, Self::Leaf { .. } | Self::Membership { .. } | Self::Route { .. })
    }
}
