// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The first application: it declares an interface, and owns none of it.
//!
//! # Why this crate exists, which is not the same as what it does
//!
//! `docs/design/ring-scene-boot.html` part III section 11: *the system owns the
//! tree and the application holds a handle to it. Today applications own their
//! interface and grudgingly export a shadow of it on request.* Every other
//! component in this tree is a **server** — it holds something and answers for
//! it — and that is the shape an application under the old arrangement has. This
//! one is the other shape, and it is the smallest thing that can be it: it holds
//! no tree, it answers no ring, and everything it has to say about its own
//! interface it says by submitting entries to somebody else.
//!
//! What it does, in one paragraph: it is spawned with a manifest that declares a
//! state tree, adopts one ring as a **client**, agrees a vocabulary version,
//! declares four nodes and a state across that ring, waits for each entry's
//! completion, publishes what it counted into the state tree its manifest
//! declared, and ends. It never learns whether the tree it declared still
//! exists, because that is not its business — which is the whole of the
//! inversion, said as a component rather than as a sentence.
//!
//! # What it deliberately does not hold
//!
//! **No `f_semantic::Tree` and no allocator.** [`script`] is a fixed array of
//! entries and every one of them is encoded onto the stack, one at a time, into
//! bytes the ring's own arena already owns. So this component has no heap it
//! uses, no writable static — `cargo xtask component` refuses an image with one
//! — and an image small enough that RFC 0100's bound is not close.
//!
//! That is not frugality, it is the property. A component that held a copy of
//! what it declared would be exporting a shadow of its own tree on request,
//! which is the arrangement section 11 is written against. The reversal
//! condition is a field on anything in this crate that remembers a node.
//!
//! # What it is not, listed rather than discovered
//!
//! It draws nothing and it is not an interface anybody would want to use: four
//! nodes and one state, chosen so that the boot's arithmetic is checkable by eye
//! rather than so that the panel is useful. `E3-B06f` is where a real
//! declaration is projected into part II's graph, and `E3-P04` is where one
//! unmodified application reaches four projections.
//!
//! It also does not reconcile. `E3-B01l` built the reconciler that turns an
//! immediate-mode redraw into deltas — section 11's *concession that decides
//! adoption* — and nothing here links it, because this component declares one
//! frame and has no previous tree to diff against. A second frame is the day
//! that row appears in `Cargo.toml`.

#![no_std]

// Outside every gate, because it is the layout both sides read: the frame writes
// the page this describes and the component reads it, and a set of offsets that
// existed in only one of the two builds would be a layout with two definitions.
// RFC 0047.
pub mod routing;

// The entries this component submits, compiled everywhere so that the frame can
// check a report against the script rather than against a copy of it. Its own
// comment argues the split, which is `user/compositor`'s between `tree` and
// `component` and earns its keep the same way.
pub mod script;

// The component half is x86-64's, and only because the door is. Nothing in
// `component.rs` is architecture-specific; the one instruction underneath it is,
// and `f_abi::door::call` is compiled only where there is a frame to call. The
// same gate every component crate in this tree has, with the same reversal: an
// AArch64 frame.
//
// The second gate is the `image` feature, and it is off in exactly one place:
// the frame, which links this crate for `routing`'s offsets and for [`script`].
// A `#[panic_handler]` is a lang item and there may be one per linked artefact,
// so the module carrying this component's would otherwise collide with the
// frame's own.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod component;
