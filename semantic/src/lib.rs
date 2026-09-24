// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The system owns the tree and the application holds a handle to it.
//!
//! # The inversion, and why it needs a crate rather than a comment
//!
//! `docs/design/ring-scene-boot.html` part III section 11 states the thesis in
//! one sentence: *the system owns the tree and the application holds a handle to
//! it. Today applications own their interface and grudgingly export a shadow of
//! it on request.* Everything else in that part is consequence.
//!
//! A sentence like that is held or not held by where the memory is and who may
//! write it, so this crate is two types and the difference between them:
//!
//! - [`TreeId`] is an **address**. It names a tree the system holds. It is
//!   minted when the tree is opened, it is never revoked, and [`Registry::read`]
//!   answers it for as long as the system keeps the tree — which is past the
//!   death of whatever declared it.
//! - [`Handle`] is a **right**. It is `f_abi::cap::Handle` — a slot and a
//!   generation, the same two fields the capability table has had since RFC
//!   0002 — and it is what [`Registry::apply`] demands. [`Registry::close`]
//!   moves the generation on, so every handle minted before it is stale from
//!   that instant.
//!
//! **There is no function in this crate that turns a [`TreeId`] into a
//! [`Handle`].** That absence is the whole mechanism: a reader may address a
//! tree it may not write, and holding the address buys nothing towards writing.
//! *What would reverse this:* such a function appearing, at which point the two
//! types are one type with two spellings and this crate's argument is about
//! something else.
//!
//! # Why either half alone would be unremarkable
//!
//! `TODO.md`'s `E3-B06c` says it and it is worth repeating where the code is: a
//! tree that survives its writer is what a **log** does, and a handle that dies
//! with its process is what **every** handle does. Neither is the inversion.
//! What is the inversion is the two at once over one structure — the bytes are
//! still there, still addressable by the identity their author chose, and the
//! author's right to touch them is gone.
//!
//! So [`Registry`] is arranged so that a test cannot show one half without the
//! other being visible in the same value: `close` is a single call that seals
//! the tree and stales the handle, and there is no way to do either separately.
//!
//! # What the system is, here
//!
//! The frame. `kernel/src/semantic.rs` holds a [`Registry`] in a page the frame
//! allocated, mapped into no component's address space, and calls [`close`] at
//! the same place it reaps an occupant's address space. That is what makes *the
//! system owns it* a fact about memory rather than about a struct name.
//!
//! The registry is a plain value and nothing here allocates, so the same type
//! runs in a host test on both architectures — which is where every clause below
//! is exercised, because a boot is a slow way to find out that an arithmetic is
//! wrong.
//!
//! # What this crate deliberately does not hold
//!
//! Content bodies and relation sets. [`Tree`] stores identity, place, order,
//! role and state; `SetContent` and `SetRelations` are **refused** with
//! [`Rejected::NotStored`] rather than answered and dropped. A receiver that
//! accepted an entry and stored nothing of it would be a tree that disagrees
//! with its writer about what it holds, which is worse than a refusal a writer
//! can act on. *What would reverse this:* `E3-B06f`, where content is projected
//! into part II's retained graph and therefore has somewhere to go.
//!
//! **`E3-B06f` has landed and this reversal has not fallen due**, which is
//! worth saying plainly rather than leaving a reader to check. [`emit`] projects
//! a node's *kind*, its *placement* and its *paint*; a content body reaches a
//! scene as geometry — a `SetPath` into a channel's inline arena — and nothing
//! in this tree flattens a string or a picture into one yet. So content still
//! has nowhere to go, the refusal stands, and the condition moves to the first
//! geometry encoder rather than to this diff.
//!
//! It also holds no theme, no constraints and no layout. `E3-B06d` resolves and
//! `E3-B06e` solves; a tree that carried either before there was anything to
//! resolve against would be storing a value nobody could falsify.

//! # The second thing that is here because neither crate may hold it
//!
//! [`agent`] — an agent's invocation, as a capability call. `f_interface::agent`
//! selects the node and answers what may be done with it, and it stops there:
//! the handle that authorises the doing is `f_abi::cap::Handle`, and
//! `interface/Cargo.toml` declines `f-abi` so that the vocabulary stays a leaf.
//! So the call lands here, on exactly the terms [`Registry`] lands here, and it
//! keeps the same rule one layer down — nothing turns the intent a projection
//! can *read* into the handle that would invoke it. `E3-B06j`.

//! # The third, and it is the one that decides whether the layer can be left
//!
//! [`emit`] — a declared tree as scene deltas. `f_interface` holds the meaning
//! and declines the wire; `f_scene` holds the graph and may not hold the
//! meaning, because a graph that depended on this layer is a graph this layer
//! could take down with it. So the projection lands here for the third time for
//! the same reason, and what it is answerable to is part III's one request of
//! part II: **the compositor cannot tell an authored delta from a projected
//! one**, so that this layer can be measured and abandoned without the renderer
//! going with it. `E3-B06f`, RFC 0117.

//! # The fourth, and it is the same join across a machine boundary
//!
//! [`remote`] — the declaration on a link, and the far side presenting it at its
//! own density and its own refresh rate. It is the third crate-shaped argument
//! made a fourth time: the wire is `abi`'s, the meaning is `interface`'s, the
//! graph is `scene`'s, and a remote needs all three at once. What it adds is the
//! answer to *what does a receiver do with what the format does not carry* —
//! there is no layout entry, so the far side's arrangement is the far side's,
//! and RFC 0121 is where that is argued rather than assumed. `E3-B06h`.

#![no_std]

pub mod agent;
pub mod emit;
pub mod registry;
pub mod remote;
pub mod tree;
pub mod vocabulary;

pub use registry::{Opened, Registry, TreeId};
pub use tree::{Applied, Node, Rejected, Staged, Tree};
pub use vocabulary::Interface;

/// The right to write one tree, and the type the capability table already uses.
///
/// Re-exported rather than redefined. A second handle type would be a second
/// set of rules about what a generation means, and RFC 0002's are the ones this
/// system has: generations count from one, so a zeroed field is never an
/// authority; a generation that reached [`Handle::RETIRED_GENERATION`] retires
/// its slot rather than wrapping, because a stale handle that becomes valid
/// again is the hole the counter exists to close.
pub use f_abi::cap::Handle;
