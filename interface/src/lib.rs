// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What an application declares, and what a projection may do with it.
//!
//! `docs/design/ring-scene-boot.html` part III argues the thesis — the
//! application declares a semantic structure, the system owns it, and pixels
//! are one projection of it. This crate is the first thing in the tree that
//! makes any of that concrete, and it is deliberately the *smallest* thing
//! that can: four decisions, no compositor, no renderer, no wire.
//!
//! # Why this is a library and not a component
//!
//! Nothing here runs. E3's build tasks are all blocked — on a ring and a
//! doorbell that are not written, on a machine nobody owns, on three releases
//! that have not shipped — and the four decisions in front of them are not,
//! because their exits are paper and a test. So what this crate holds is the
//! vocabulary and its refusals, exercised against the three interfaces the
//! decisions were chosen to be hard for. When the compositor exists, it takes
//! these types and this crate stops being the whole of the layer; until then
//! it is the only place the thesis is written down as something that can fail.
//!
//! # The rule that makes it possible to abandon
//!
//! Part III ends by asking for exactly one property: the compositor must
//! neither know nor care whether a delta was authored or projected, so that
//! this layer can be *measured and if necessary abandoned* without taking the
//! rendering pipeline down with it. That is why this crate depends on nothing
//! and nothing yet depends on it. A leaf can be deleted; a layer cannot.
//!
//! # The four decisions
//!
//! - [`node`] — the vocabulary, version 1. `E3-D01`, RFC 0077.
//! - [`canvas`] — what a self-rendering surface still declares. `E3-D02`, RFC 0078.
//! - [`token`] — the typed token layer and what a theme cannot break. `E3-D03`, RFC 0079.
//! - [`ladder`] — the renderer fallback ladder. `E3-D04`, RFC 0080.

#![no_std]

pub mod backend;
pub mod canvas;
pub mod ladder;
pub mod node;
pub mod token;
