// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Instantiating a topology from a root: refold what arrived, and believe
//! nothing that does not fold to the hash the boot was asked for.
//!
//! # The order of the checks, which is the design
//!
//! Cheapest disbelief first, and each offset a function of a count already
//! believed:
//!
//! 1. the boot module decodes — [`f_abi::boot::Module`];
//! 2. the record tree checks — `f_generation::record::Tree`;
//! 3. the fold over it **equals the root the boot was selected with** — and if
//!    it does not, nothing below runs. That is the one refusal `E2-P07` exists
//!    to exercise and the reason the spec calls this *the same crate doing the
//!    same arithmetic*;
//! 4. the module carries exactly as many component files as the tree has
//!    members;
//! 5. every file's SHA-256 is the address its leaf names, and every record's
//!    own `name` is the name its leaf carries.
//!
//! Five is the one worth arguing for, because it looks redundant after three: a
//! root that folds means the *leaves* are what the compiler wrote, and the
//! leaves name content addresses. But a leaf is a name and a hash, not the
//! bytes — the module could carry a correct tree beside a component file that
//! is not the one the tree names, and every check above three would pass. So
//! the address is recomputed over the bytes that arrived. The name is checked
//! for the neighbouring reason: a record whose `name` disagreed with its leaf
//! would give the topology two names for one component, and `sibling:` routing
//! resolves by name.
//!
//! # Why a start order is computed here and not at start time
//!
//! Because it is part of what a root determines. A topology whose components
//! could be started in more than one order would be a topology that is not a
//! function of its root, and one whose routes form a cycle has no order at all
//! — refused here rather than discovered as a start that never makes progress.
//! The order is the unique one: repeatedly the lowest-index member all of whose
//! sources are already placed.

use alloc::vec::Vec;

use f_abi::boot::Module;
use f_abi::manifest::{NAME_MAX, Record};
use f_abi::store::Route;
use f_generation::fold;
use f_generation::record::Tree;

use crate::bind::Address;
use crate::start::State;

/// Why a boot module was not believed as a topology.
///
/// One variant per thing that can be wrong, and not one for *malformed*,
/// because the reader of this is somebody holding a root hash and a machine
/// that will not boot — and *the module is wrong* is not a sentence anybody can
/// act on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// The boot module's own head did not decode. Carries the packed
    /// `f_abi::error` [`Module::read`] refused with.
    Module(i32),
    /// The record tree inside it did not check. Carries `f-generation`'s
    /// refusal, which names which property of a *tree* was violated.
    Tree(f_generation::Refusal),
    /// The fold over the tree is not the root this boot was selected with.
    ///
    /// The refusal `E2-P07` exists to exercise, and the only one here that is
    /// about the *identity* of what arrived rather than its shape.
    Root,
    /// The module carries a different number of component files than the tree
    /// has members. The *n*th file is the *n*th member and there is no index,
    /// so a count that disagrees is a module with no answer to *which file is
    /// this*.
    Files,
    /// A component file was refused. Carries the member's index and the
    /// manifest refusal, which names the field.
    Component(u16, f_abi::manifest::Refusal),
    /// A component file's SHA-256 is not the address its leaf names.
    Content(u16),
    /// A record's own `name` is not the name its leaf carries.
    Named(u16),
    /// A route hands a component a capability its manifest never asked for, or
    /// asked for from somebody else. Carries the consuming member's index and
    /// the capability's name.
    ///
    /// The refusal `user/generation.toml` was waiting for: until it existed, a
    /// route was a statement about intent that changed the root and bound
    /// nothing.
    Unrouted(u16, [u8; NAME_MAX]),
    /// A component declares a `sibling:` need that is not optional and the
    /// topology routes it nothing. Carries the member's index and the need's
    /// name.
    ///
    /// Refused here rather than at the spawn that would be refused later, for
    /// the reason every check in this function has: the file that has to change
    /// is the topology, and a refusal that arrives from the frame names the
    /// component instead.
    Unsupplied(u16, [u8; NAME_MAX]),
    /// The routes form a cycle, so there is no order in which every component
    /// starts after the components it needs. Carries the lowest member index
    /// still unplaced when progress stopped, which is a member of the cycle.
    Cycle(u16),
}

impl Refusal {
    /// A line for whoever is holding the root that would not boot.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Module(_) => "the boot module's head is not one this build decodes",
            Self::Tree(_) => "the record tree in the boot module is not one this build believes",
            Self::Root => "the fold over the boot module is not the root this boot selected",
            Self::Files => "the module carries a different number of files than the tree names",
            Self::Component(_, _) => "a component file in the module was refused",
            Self::Content(_) => "a component file is not the bytes its leaf names",
            Self::Named(_) => "a component file's own name is not the name the topology gives it",
            Self::Unrouted(_, _) => {
                "a route hands over a capability the component's manifest does not declare"
            }
            Self::Unsupplied(_, _) => {
                "a component needs a capability from a sibling and the topology routes it none"
            }
            Self::Cycle(_) => "the routes form a cycle, so no component can start first",
        }
    }
}

/// One member of an instantiated topology: what it is, what it was bound to,
/// and whether it started.
#[derive(Clone, Copy, Debug)]
pub struct Instance {
    /// Where this member sits in the topology's canonical order.
    /// Unit: index into the topology's member list, zero-based.
    pub index: u16,
    /// The name the leaf gives it, NUL-padded.
    /// Unit: bytes of ASCII from `[a-z0-9-]`, at most [`NAME_MAX`].
    pub name: [u8; NAME_MAX],
    /// The content address of its component file.
    /// Unit: bytes, exactly 32 — the SHA-256 recomputed over the file the
    /// module carried, not the one the leaf claimed.
    pub content: [u8; 32],
    /// Its manifest, copied out of the module.
    ///
    /// Owned rather than borrowed, and `f_abi::manifest::Record::read_unaligned`
    /// says why: the files sit at offsets the module's format chose, not at
    /// ones a loader aligned.
    /// Unit: none — a record, every field of which states its own.
    pub record: Record,
    /// The device it was bound to, if it declared one and the bus had it.
    ///
    /// `None` on every component that declares no device, which is most of
    /// them, and on a driver whose part is not in this machine. The second case
    /// is not a refusal here: it is a driver that cannot start, and
    /// [`crate::start`] is where that is decided.
    /// Unit: none — a bus address, discovered and never declared.
    pub bound: Option<Address>,
    /// Whether it started, failed, or was never tried.
    /// Unit: none — a [`State`].
    pub state: State,
}

impl Instance {
    /// The name without its padding.
    #[must_use]
    pub fn label(&self) -> &[u8] {
        let end = self.name.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
        self.name.get(..end).unwrap_or(&[])
    }
}

/// A topology instantiated from one root.
///
/// Everything about it is a function of the root: the members and their order
/// come from the tree, the routes come from the tree, the start order is
/// computed from the routes by a rule with no choices in it, and the binding is
/// a function of the members and of a bus whose *set* — never its order —
/// reaches this type. [`crate::render::topology`] is the statement of that, in
/// bytes.
#[derive(Clone, Debug)]
pub struct Assembly {
    /// The root this was instantiated from and refolded to.
    /// Unit: bytes, exactly 32 — a SHA-256.
    root: [u8; 32],
    /// The members, in the tree's canonical order.
    members: Vec<Instance>,
    /// The routes, in the tree's canonical order.
    routes: Vec<Route>,
    /// The order components are started in: member indices, each appearing
    /// once.
    order: Vec<u16>,
}

impl Assembly {
    /// Instantiate a topology from a root and the boot module that carries it.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]. Nothing here is skipped, clamped or repaired: a module
    /// that does not fold to the root it was selected with is not a topology
    /// this build will run, and there is no partial answer worth returning.
    pub fn instantiate(root: &[u8; 32], module: &[u8]) -> Result<Self, Refusal> {
        let module = Module::read(module).map_err(Refusal::Module)?;
        let tree = Tree::check(module.tree()).map_err(Refusal::Tree)?;

        // The whole reason a machine is handed records rather than a root
        // alone: the same fold, in the same crate, over the bytes that arrived.
        if fold::root(&tree) != *root {
            return Err(Refusal::Root);
        }
        if module.files() != tree.members() as usize {
            return Err(Refusal::Files);
        }

        let mut members = Vec::with_capacity(tree.members() as usize);
        for index in 0..tree.members() {
            let leaf = tree.member(index).ok_or(Refusal::Files)?;
            let file = module.file(index as usize).ok_or(Refusal::Files)?;
            let record =
                Record::read_unaligned(file).map_err(|why| Refusal::Component(index, why))?;
            if f_hash::sha256(file) != leaf.hash {
                return Err(Refusal::Content(index));
            }
            if record.name != leaf.name {
                return Err(Refusal::Named(index));
            }
            members.push(Instance {
                index,
                name: leaf.name,
                content: leaf.hash,
                record,
                bound: None,
                state: State::Untried,
            });
        }

        let mut routes = Vec::with_capacity(tree.routes() as usize);
        for index in 0..tree.routes() {
            routes.push(tree.route(index).ok_or(Refusal::Files)?);
        }

        routed(&members, &routes)?;
        let order = start_order(members.len(), &routes)?;
        Ok(Self { root: *root, members, routes, order })
    }

    /// The root this topology was instantiated from.
    #[must_use]
    pub const fn root(&self) -> [u8; 32] {
        self.root
    }

    /// The members, in the tree's canonical order.
    #[must_use]
    pub fn members(&self) -> &[Instance] {
        &self.members
    }

    /// The routes, in the tree's canonical order.
    #[must_use]
    pub fn routes(&self) -> &[Route] {
        &self.routes
    }

    /// The order components are started in: member indices.
    #[must_use]
    pub fn order(&self) -> &[u16] {
        &self.order
    }

    /// One member, by index.
    #[must_use]
    pub fn member(&self, index: u16) -> Option<&Instance> {
        self.members.get(index as usize)
    }

    /// One member, mutably. Crate-private: the two things that change after
    /// instantiation are the binding and the state, and both have a function of
    /// their own that changes them.
    pub(crate) fn member_mut(&mut self, index: u16) -> Option<&mut Instance> {
        self.members.get_mut(index as usize)
    }

    /// Every member this one routes a capability *from* — what it waits for.
    ///
    /// The edge the exit turns on, seen from the waiting end, which is the end
    /// [`crate::start`] walks: a route says component C needs capability K from
    /// source S, so S is what C waits for.
    #[must_use]
    pub fn sources(&self, component: u16) -> Vec<u16> {
        let mut out = Vec::new();
        for route in &self.routes {
            if route.component == component && !out.contains(&route.source) {
                out.push(route.source);
            }
        }
        out.sort_unstable();
        out
    }

    /// The same edge from the providing end, closed transitively: **the subtree
    /// of one member**, which is exactly what its failure costs.
    ///
    /// Two directions and one relation, and it is worth having both rather than
    /// deriving one at each call site. [`Assembly::sources`] is what a start
    /// walk asks, one member at a time; this is what an *operator* asks — *what
    /// did that failure take with it* — and it is also what a test can compare a
    /// set of unstarted components against, which is a stronger statement of the
    /// exit than naming three components by hand.
    ///
    /// The member itself is not in the result: a subtree is what hangs below,
    /// and the failure is not part of what the failure cost. It terminates on
    /// any topology, cycle or not, because a member is added once — and
    /// `Assembly::instantiate` has already refused a cycle anyway.
    #[must_use]
    pub fn subtree(&self, source: u16) -> Vec<u16> {
        let mut found: Vec<u16> = Vec::new();
        let mut frontier: Vec<u16> = alloc::vec![source];
        while let Some(next) = frontier.pop() {
            for route in &self.routes {
                if route.source == next
                    && route.component != source
                    && !found.contains(&route.component)
                {
                    found.push(route.component);
                    frontier.push(route.component);
                }
            }
        }
        found.sort_unstable();
        found
    }
}

/// The routes and the manifests agreeing with each other, in both directions.
///
/// # Why this is the check that makes "declaratively" mean something
///
/// `user/generation.toml` has carried the promise since the compiler landed:
/// the routes *are declared there and checked against the manifests by
/// `E2-B05`*, and until they were, a route was — in that file's own words — a
/// statement about intent that changed the root. This is where it stops being
/// one. Without it the assembler would happily route a capability no component
/// asked for and start a component that would then be refused its handle at
/// spawn, which is a topology that folds to a root and cannot run.
///
/// Both directions, because each catches a different mistake:
///
/// - **A route with no need.** The topology says `store` gets `block` from
///   `virtio-blk`, and `store`'s manifest never asked for `block`. Somebody
///   renamed a capability in one of the two files.
/// - **A need with no route.** A component declares `from = "sibling:x"` and
///   the topology routes it nothing. Its spawn is refused later, for a reason
///   that is in a *different file* from the one that has to change — unless the
///   need is `optional`, which is the declaration that an empty slot is
///   acceptable, and is therefore exactly the case this does not refuse.
///
/// `docs/manifest.md` says a `sibling:` need is *checked for shape here and for
/// existence by the topology, which is not in this file*. This is that
/// sentence's other half, arriving in the crate the sentence pointed at.
fn routed(members: &[Instance], routes: &[Route]) -> Result<(), Refusal> {
    for route in routes {
        let (Some(consumer), Some(source)) =
            (members.get(route.component as usize), members.get(route.source as usize))
        else {
            return Err(Refusal::Files);
        };
        let satisfied = consumer.record.needs().iter().any(|need| {
            need.name == route.capability
                && need.route == f_abi::manifest::route::SIBLING
                && need.sibling == source.name
        });
        if !satisfied {
            return Err(Refusal::Unrouted(route.component, route.capability));
        }
    }

    for member in members {
        for need in member.record.needs() {
            if need.route != f_abi::manifest::route::SIBLING || need.optional != 0 {
                continue;
            }
            let routed = routes
                .iter()
                .any(|route| route.component == member.index && route.capability == need.name);
            if !routed {
                return Err(Refusal::Unsupplied(member.index, need.name));
            }
        }
    }
    Ok(())
}

/// The one order in which every component starts after the components it needs.
///
/// Repeatedly take the **lowest-index** member all of whose sources are already
/// placed. That tie-break is what makes the order unique rather than merely
/// valid: any topological order would satisfy the routes, and a topology with
/// two valid orders would not be a function of its root — which is the property
/// `E2-B05` is measured on, so the choice is made here and stated rather than
/// left to whichever order a loop happened to visit.
///
/// # Errors
///
/// [`Refusal::Cycle`], naming the lowest member still unplaced when progress
/// stopped. A cycle is refused rather than broken: breaking one would be this
/// function choosing which component starts without its capability, and nothing
/// here is entitled to choose that.
fn start_order(members: usize, routes: &[Route]) -> Result<Vec<u16>, Refusal> {
    let mut placed = alloc::vec![false; members];
    let mut order = Vec::with_capacity(members);

    while order.len() < members {
        let mut progress = false;
        for index in 0..members {
            if placed[index] {
                continue;
            }
            let index = index as u16;
            let ready = routes
                .iter()
                .filter(|route| route.component == index)
                .all(|route| placed.get(route.source as usize).copied().unwrap_or(false));
            if ready {
                placed[index as usize] = true;
                order.push(index);
                progress = true;
                // Break, rather than carrying on down the list. Taking the
                // lowest ready member and then re-scanning from the top is what
                // makes the result the unique lowest-first order; finishing the
                // pass would place a higher member before a lower one that
                // became ready during the same pass.
                break;
            }
        }
        if !progress {
            let stuck = placed.iter().position(|done| !done).unwrap_or(0);
            return Err(Refusal::Cycle(stuck as u16));
        }
    }
    Ok(order)
}
