// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The read path: a hash to a zone and an offset, and the device landing the
//! bytes in the caller's own registered buffer.
//!
//! # What `E2-B08` asks for, and where each half of it is
//!
//! *Copies per read is zero, **counted rather than asserted**; resident bytes
//! per unit of work recorded.*
//!
//! - [`read`] is the path. Name to hash through `f-index`, hash to a logical
//!   block through `f-blob`, logical block to a **zone and a device-absolute
//!   offset** through `f-zone`, and then one submission per block, each naming
//!   a buffer of the set the caller registered. The header is decoded out of
//!   the memory the device wrote by borrowing it, and the content is hashed
//!   there.
//! - [`dma`] is the device model's side: `f_ring::registry::Table` deciding
//!   which destinations are registered, and the one function in this crate that
//!   moves a byte.
//! - [`resident`] is the second half of the exit, and it refuses to report one
//!   number where there are three.
//!
//! # The word the exit turns on
//!
//! *Counted.* RFC 0024's typestate makes a zero-copy read an argument by
//! construction, and an argument by construction publishes the same zero
//! whether the property holds or the mechanism behind it was deleted. So there
//! are two tallies of *bytes moved through any buffer that is not the caller's
//! registered one*, taken on opposite sides of the boundary and required to
//! agree — [`read::Counters::staged_bytes`] and
//! [`dma::Landed::unregistered_bytes`] — and there is a provocation that makes
//! both of them non-zero on purpose, so that a zero recorded beside it is
//! evidence rather than a default.
//!
//! # The caller's buffer is a *registered* one, and the task said so first
//!
//! `TODO.md`'s `E2-B08` line carries the parenthesis: *the caller's buffer is a
//! registered one, or the zero-copy count is a copy nobody counted*. That is
//! why this crate takes `f-ring` and resolves every destination through
//! `f_ring::registry::Table` rather than trusting a slice it was handed. A
//! count over an inline buffer would be a count of the second copy while the
//! first one — the client's bytes into the component's inline payload — went
//! unremarked.
//!
//! # What this crate is not yet, said plainly
//!
//! **It is not a component.** There is no `manifest.toml`, no entry in
//! `COMPONENTS`, and no image. `intent/0006-state/plan.md` describes
//! `user/objects` as a component, and one thing stands between: a component
//! image that links `f-blob` needs a `#[global_allocator]`, that is an
//! `unsafe impl GlobalAlloc`, and RFC 0001 forbids `unsafe` above the frame.
//! The spec's decision 4 says where the heap goes — `ring/`'s, a bump-then-free
//! -list allocator over a granted region, with the `unsafe` in the frame — and
//! that heap does not exist. The plan's own *Risks* names this crate as the
//! first image that would need it. Writing an `unsafe impl GlobalAlloc` here to
//! get an image built is the one thing that section tells this task not to do,
//! so the image waits for the heap and this line is where the debt is recorded
//! rather than left for a reader to infer from an absent file.
//!
//! **There is no ring under it.** The transfers here go to a modelled device,
//! the way `zone/tests/cycle.rs`'s do and for the same reason: `user/virtio-blk`
//! answers two of RFC 0060's seven opcodes, and the boot the spec describes is
//! of this component over that driver's ring. What *is* real is the
//! registration — `f_ring::registry::Table` is the service-side table a real
//! driver checks a real submission against, unchanged — so the question
//! *was this destination registered* is answered by the same code either way.
//!
//! # Determinism
//!
//! Nothing here reads a clock, draws a value or asks an `Env` for anything, and
//! there is no `HashMap` or `HashSet` in the crate — RFC 0004. The workload in
//! `tests/reads.rs` draws its content from a seeded `Env` through a `Stream`
//! split at a named identity under RFC 0026, so that adding a draw site later
//! cannot move a number this crate recorded.
//!
//! # The frame
//!
//! No `unsafe`, and none is reachable: the workspace forbids it here at compile
//! time. Every byte that arrives from a device is decoded field by field by
//! `f_abi::store`'s validating constructors, borrowed out of the caller's
//! buffer rather than cast over it.

#![no_std]

extern crate alloc;

pub mod dma;
pub mod read;
pub mod resident;

pub use dma::{Landed, Landing};
pub use read::{Counters, Placement, Read, ReadPath, mount};
pub use resident::Resident;
