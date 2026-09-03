// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The component a place holds.
//!
//! `user/init` is the first thing above the frame and it is not a component in
//! the sense RFC 0008 means: it has no manifest, it is carried as a flat image
//! at a fixed position, and the frame runs it because it was told to. This is
//! the other kind — the kind a *place* holds. It has a manifest
//! (`user/store/manifest.toml`), it is carried as a **component file** — that
//! manifest compiled to a record, followed by this image, named by one hash over
//! both — and it comes into existence only by a spawn naming that hash.
//!
//! What that buys, and what E1-B05 demonstrates on every boot, is that killing
//! it and putting a new one in its place is one mechanism rather than a special
//! case: the clients hold an endpoint to the *place*, not to this instance, so a
//! connect that arrives while the place is empty pends and is answered by the
//! refill.
//!
//! # What it does not do yet, and why the file says so
//!
//! It does not drain its control ring. Every notice a component receives is a
//! completion entry on that ring, drained at a polling point (RFC 0008, R05) —
//! and draining one means adopting a mapped channel, which is `unsafe`, which
//! this crate may not write. The safe adoption a component needs is E1-B08's,
//! and RFC 0030 records the deferral as a date rather than an intention.
//!
//! Until it lands the frame does both halves on this component's behalf: it
//! posts every notice it owes onto the ring, and it takes them back off again
//! at the polling point this crate would otherwise have. That is enough to show
//! a notice arriving as a flagged completion entry and is not enough to show a
//! component *acting* on one, which is the gap, and it has an owner rather than
//! a silence.
//!
//! # What it may assume about where it is
//!
//! Exactly what `user/init` may, and for the same reasons: its text is mapped
//! read-only and executable at `kernel::process::TEXT`, its stack is one
//! writable page, and there is no writable static anywhere in this image —
//! `cargo xtask component` refuses to build one that has any, because the text
//! page is not writable and a mutable global would fault on its first write
//! rather than fail to link.

#![no_std]

pub mod report;

// The component half is x86-64's, and only because the door is. Nothing in
// `component.rs` is architecture-specific; the one instruction underneath it is,
// and `f_abi::door::call` is compiled only where there is a frame to call. The
// same gate `user/init` has, with the same reversal: an AArch64 frame.
//
// The second gate is the `image` feature, and it is off in exactly one place:
// the frame, which links this crate as a library so that it and this component
// agree about [`report`] rather than each holding a copy of the encoding. A
// `#[panic_handler]` is a lang item and there may be one per linked artefact,
// so the module carrying this component's would otherwise collide with the
// frame's own. `user/store/Cargo.toml` states the reversal. The arrangement is
// `user/virtio-blk`'s, one crate over, for a different reason and with a
// different reversal — which is worth noticing rather than generalising.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod component;

// The runtime this component becomes when it is entered with `report::RUN`. It
// is the image's, for the same two reasons: it calls the door, and it is only
// ever reached from `component::start`.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod runtime;

#[cfg(test)]
mod tests {
    /// The manifest declares a `[[ring]]` of sixteen entries and the frame's
    /// control ring is sixteen too. Not a coincidence and not a requirement —
    /// they are different rings — but a number worth stating in a place a
    /// reader will look, because the two being equal is what makes a reader
    /// assume they are one ring.
    #[test]
    fn a_component_speaks_the_protocol_without_unsafe() {
        let sqe = f_abi::Sqe { opcode: 0, class: f_abi::class::SOFT, ..f_abi::Sqe::ZERO };
        assert_eq!(sqe.deadline, f_abi::NO_DEADLINE);
        assert_eq!(sqe.cap, f_abi::cap::Handle::NULL.bits());
    }
}
