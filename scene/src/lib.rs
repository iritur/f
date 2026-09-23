// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The retained scene graph, and the rules a delta has to satisfy to change it.
//!
//! Part II of `docs/design/ring-scene-boot.html` asks for one thing that most
//! window systems do not do: the system holds the scene, and a client sends the
//! differences. No pixel buffer crosses the boundary, which is why the parent
//! task's exit is a *count* — boundary crossings per UI frame under ten — and
//! why that count can gate on the machines this project has, in the way
//! `claims/0005` established for a blast radius.
//!
//! # Why this is a crate and not a module of the compositor
//!
//! Because the compositor is a component and this is not. A component has a
//! manifest, a state tree and an entry point; a graph is a data structure, and
//! the two have different reasons to change. Part III asks that the compositor
//! be unable to tell an authored delta from a projected one — that property is
//! a statement about *this* crate's inputs, and it is checkable here without a
//! compositor existing at all, which is the whole reason the work below the
//! supervisor could start before the supervisor finished.
//!
//! # What it does not have
//!
//! No allocator and no `Vec`. The arena has a named maximum and exceeding it is
//! a refusal with a name rather than a panic, for the reason every fixed bound
//! in this tree has: a renderer that can allocate under load is a renderer
//! whose worst case is the allocator's, and nobody measures that one.

#![no_std]

pub mod arena;
pub mod commit;
pub mod degrade;
pub mod dirty;
pub mod effect;
pub mod encode;
pub mod kind;
pub mod reconcile;
