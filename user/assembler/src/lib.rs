// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One root in, one topology out — and the same root twice gives the same
//! bytes.
//!
//! # What this crate claims
//!
//! `E2-B05`'s exit is two sentences and they are load-bearing separately:
//!
//! 1. **Boot is a pure function of one hash.** The same root produces a
//!    byte-identical topology. [`render::topology`] says what "byte-identical"
//!    is taken over, in bytes rather than in prose, and it is deliberately not
//!    the module: the module is the *input*, and a comparison over the input
//!    would be a hash comparison wearing an assembler's clothes.
//! 2. **A driver that fails to start leaves its subtree unstarted rather than
//!    failing the boot.** [`start`] is where that happens, and the word that
//!    carries it is *subtree*: the components a failed one provides for, and
//!    those alone. Everything else starts, and the boot is alive at the end.
//!
//! # Why nothing here is a second reader of anything
//!
//! The whole crate is three borrowed codecs and one decision. The boot module
//! is decoded by [`f_abi::boot::Module`]; the record tree by `f_abi::store`
//! through `f_generation::record`; each component file by
//! `f_abi::manifest::Record`. The fold that turns the tree back into a root is
//! `f_generation::fold` — *the same function `cargo xtask generation` used to
//! produce the root in the first place*, which is what makes the check here
//! arithmetic rather than a second opinion.
//!
//! The spec's stated reversal for the whole boot-module arrangement is **a
//! second reader of the format appearing anywhere**. This crate is the first
//! reader and is written so that it is not also the second: it defines no
//! layout, no magic and no offset of its own, and the day it needs to it is the
//! day the reversal has arrived.
//!
//! # Why a `Start` trait rather than a spawn
//!
//! Because there is no supervisor component yet. `kernel/src/component.rs` says
//! so at length and `cargo xtask lint-owed` holds the sentence: nothing
//! implements `f_abi::control::op::SPAWN`, and the frame is holding the ground
//! a supervisor will stand on. So this crate decides **what to start, in what
//! order, bound to which device** — which is all of `E2-B05` that can exist
//! before that day — and the act of starting is injected as [`start::Start`].
//!
//! That is dependency injection and not a callback, the same distinction
//! `cargo xtask lint-callbacks` draws for `f_env::Env`: the trait is
//! implemented by the *system* and called by this crate, and nothing a peer
//! sends registers anything.
//!
//! *Reversal, with an owner:* `E1-B05` moves restart policy above the frame and
//! `E2-B06` puts the swap opcodes on the control ring. On that day the
//! implementation of this trait is a component submitting `SPAWN`, and what
//! moves is one impl rather than this crate.
//!
//! # What this crate is not, and what it costs
//!
//! It is not a component yet, and **nothing at boot calls it**. There is no
//! `manifest.toml` beside it and no entry in `COMPONENTS`; the frame's own
//! `kernel/src/component.rs` still walks boot modules by magic and fills its
//! places itself. RFC 0066 is that decision and its residual: *boot is a pure
//! function of one hash* is demonstrated here of the **assembly**, and is not
//! yet a property of the frame's boot path.
//!
//! Two things hold it there and neither is an oversight. `E1-B05`'s restart
//! policy has not left the frame — `cargo xtask lint-owed` reports it — so
//! there is no supervisor to be this crate's caller; and an image linking this
//! crate needs a `#[global_allocator]`, that is an `unsafe impl GlobalAlloc`,
//! and RFC 0001 forbids `unsafe` above the frame, so it waits for the heap
//! `intent/0006-state/spec.md`'s decision 4 puts in `ring/` — which is
//! `user/objects/src/lib.rs`'s debt one directory over, unchanged.
//!
//! The `alloc` this crate takes is genuine and is one `BTreeMap` and three
//! `Vec`s: the map is `bind::Bus`, keyed by a device's address, and the
//! `BTreeMap` is the point rather than a convenience — RFC 0004's iteration
//! order is what stops a bus scan's order from reaching a topology.

#![no_std]

extern crate alloc;

pub mod bind;
pub mod render;
pub mod start;
pub mod topology;

pub use bind::{Address, Bus, Discovered};
pub use start::{Start, State};
pub use topology::{Assembly, Instance, Refusal};
