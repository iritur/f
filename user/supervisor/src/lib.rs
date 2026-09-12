// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The component that spawns and stops the others.
//!
//! RFC 0008 decided that restart is the **supervisor's** act and that the frame
//! provides only the mechanism. The policy has been in the frame ever since,
//! held there by a wall that no longer exists — a supervisor at ring 3 has to
//! drive a control ring, driving one meant adopting a mapped channel,
//! `Mapping::adopt` is `unsafe`, and a `user/` crate may not write `unsafe`.
//! RFC 0037 ended that and four components have been driving rings since.
//! RFC 0073 is the other half: the frame answering `control::op::SPAWN` and
//! `op::STOP` on a ring, rather than reaching into a place itself.
//!
//! This crate is what was missing between them. `kernel/src/component.rs` named
//! the gap in three bullets and called them one thing — *there is no supervisor
//! component* — and this is it.
//!
//! # What it does today, which is less than its name
//!
//! It is spawned into a place, it publishes a state tree, and it ends. That is
//! the same life `user/store` has had since `E1-B05` began, and it is not an
//! accident of being unfinished: **a component spawned into a place is never
//! handed a core** (`kernel/src/runtime.rs`), so nothing in this image has ever
//! executed and nothing in it can until that changes. `xtask`'s `HEAP_GAP`
//! declares exactly that, and the day an occupant is scheduled the build goes
//! red on purpose and says it is the good ending.
//!
//! So what this crate buys, before it runs a line, is that the *declaration*
//! exists and is checked on every boot: a manifest whose account, heap and
//! endpoint are the ones a supervisor needs, a state schema the frame writes out
//! before the first instruction, and a place in the generation the assembler
//! checks its routes against. The code below is written for the core it does not
//! have yet, and the reason it is written now rather than then is that a crate
//! nobody can compile is a design nobody can disagree with.
//!
//! # The bootstrap, stated rather than hidden
//!
//! A supervisor is spawned into a place like anything else, and the thing that
//! spawns *it* is the frame. That circle cannot be removed, only placed:
//! somebody is first. What this arrangement chooses is that the frame keeps
//! exactly one spawn — this one — and every other spawn moves above it. A frame
//! that spawns one component and then answers that component's ring has a
//! smaller privileged surface than a frame that spawns all of them, and that
//! difference is the whole of what `E1-B05` is worth.
//!
//! # What it may assume about where it is
//!
//! Exactly what `user/store` may: its text is mapped read-only and executable at
//! `kernel::process::TEXT`, its stack is writable pages below that, and there is
//! no writable static anywhere in this image — `cargo xtask component` refuses
//! to build one that has any, because the text page is not writable and a
//! mutable global would fault on its first write rather than fail to link.

#![no_std]

// The allocator, and it is the *image's* rather than the crate's, for the reason
// `user/store/src/lib.rs` states at length: `f_ring::heap::Heap::COMPONENT`
// names one address, and that address means something only inside a component
// the frame built and granted a `heap` need to. An allocator that is merely
// *present* in a host test binary takes every allocation the harness makes and
// the binary dies before it runs a test — which is why the gate is
// `target_os = "none"` and not `target_arch`.
#[cfg(all(target_os = "none", feature = "image"))]
extern crate alloc;

#[cfg(all(target_os = "none", feature = "image"))]
#[global_allocator]
static HEAP: f_ring::heap::Heap = f_ring::heap::Heap::COMPONENT;

// The component half is x86-64's, and only because the door is. Nothing in
// `component.rs` is architecture-specific; the one instruction underneath it is,
// and `f_abi::door::call` is compiled only where there is a frame to call. The
// `image` feature is the second gate, for the `#[panic_handler]` reason: a lang
// item may appear once per linked artefact.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod component;

#[cfg(test)]
mod tests {
    /// The two opcodes this component exists to submit are the two the frame
    /// answers, and they are named in one place.
    ///
    /// A test about constants looks like a test about nothing. It is not: these
    /// numbers are the ABI, `abi/src/control.rs` has carried them since RFC 0008
    /// with nothing implementing either, and the failure this catches is the one
    /// where a supervisor is written against an opcode space somebody renumbered
    /// while it had no submitter to break.
    #[test]
    fn the_opcodes_a_supervisor_submits_are_the_ones_the_frame_answers() {
        assert_eq!(f_abi::control::op::SPAWN, 0x14);
        assert_eq!(f_abi::control::op::STOP, 0x16);
        // And they are inside the service opcode space rather than in the range
        // RFC 0028 reserves at the top for buffer registration, which is the
        // check that stops a control opcode being added at 0xFE some day.
        assert!(!f_abi::buf::opcode::is_registration(f_abi::control::op::SPAWN));
        assert!(!f_abi::buf::opcode::is_registration(f_abi::control::op::STOP));
    }
}
