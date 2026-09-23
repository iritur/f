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
//! - [`service`] is what a **client** reaches: one [`f_abi::Sqe`] in, one
//!   [`f_abi::Cqe`] out, `abi::objects::op::READ` answered and the other four
//!   refused. It is where an application byte is counted, because it is the
//!   only place in this crate a client can be said to have asked for one.
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
//! **It is a component.** `user/objects/manifest.toml` declares it,
//! `COMPONENTS` in `xtask/src/main.rs` builds it, `user/generation.toml` names
//! it in the topology, and [`component::start`] is the body a spawn jumps to.
//!
//! It was not, and the paragraph that stood here said why: a component image
//! linking `f-blob` needs a `#[global_allocator]`, that is an `unsafe impl
//! GlobalAlloc`, and RFC 0001 forbids `unsafe` above the frame. The spec's
//! decision 4 put the heap in `ring/`, and that sentence was written while it
//! was still a plan. It is not one now — `ring/src/heap.rs` is a bump-then-free
//! -list allocator over a region the frame grants and *describes* before the
//! component's first instruction, `kernel::process::SPAWN_HEAP` is where it
//! lands, and the two are pinned to one address by a compile-time assertion so
//! they cannot drift. So the most this crate writes is a `static` naming an
//! allocator somebody else implemented, which is below the line RFC 0001 draws.
//! `user/store` and `user/supervisor` had both already written it.
//!
//! What that does **not** settle is the size. The image is 1848 bytes, smaller
//! than `user/store`'s, because `user/init/link.ld` links with `--gc-sections`
//! and nothing in `f-blob`, `f-zone`, `f-index` or `f-hash` is reachable from
//! [`component::start`] yet. So the heap unblocked the *possibility* of an image
//! and this image says nothing about whether the read path fits in the pages the
//! frame maps. That question is open, it is answered by the diff that makes an
//! opcode reachable, and `user/objects/manifest.toml` records the account that
//! moves with it.
//!
//! **There is no ring under it.** The transfers here go to a modelled device,
//! the way `zone/tests/cycle.rs`'s do and for the same reason: `user/virtio-blk`
//! answers two of RFC 0060's seven opcodes, and the boot the spec describes is
//! of this component over that driver's ring. What *is* real is the
//! registration — `f_ring::registry::Table` is the service-side table a real
//! driver checks a real submission against, unchanged — so the question
//! *was this destination registered* is answered by the same code either way.
//!
//! **There is a ring over it, and that is new.** [`serve`] adopts a mapped
//! channel the frame described, registers the region its client granted, and
//! answers `abi::objects::op::READ` off it from ring 3. `cargo xtask objects
//! read` is that boot: sixty-four entries, 192 000 application bytes into the
//! client's own memory, zero staged, and every byte checked by the client
//! against arithmetic it did itself. So the count `E2-B08` is about is now taken
//! where `intent/0006-state/spec.md` says an application byte is — on the
//! objects ring, in a boot — and what remains modelled is the device *under* the
//! store rather than the boundary above it.
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

// The allocator, and it is the *image's* rather than the crate's. Host tests use
// `std`'s and never see this; `f_ring::heap::Heap::COMPONENT` names one address,
// and that address means something only inside a component the frame built and
// granted a `heap` need to.
//
// `target_os = "none"` and not `target_arch`, and `user/store/src/lib.rs`
// records why that difference is the whole gate: the host runs this crate's
// tests on x86-64 too, and an allocator that is merely *present* takes every
// allocation the test harness makes — to an address that exists inside a
// component the frame built and nowhere else. The test binary then dies before
// it runs a test.
//
// `extern crate alloc` above is unconditional and this is not, which is the one
// place this crate differs from `user/store`: the library itself allocates, so
// `alloc` is needed on the host as well, where `std` supplies the allocator and
// this static is not compiled at all.
#[cfg(all(target_os = "none", feature = "image"))]
#[global_allocator]
static HEAP: f_ring::heap::Heap = f_ring::heap::Heap::COMPONENT;

// The component half, behind **three** gates where `user/store` needs two, and
// the third one is a scar rather than a precaution.
//
// `x86_64` because the door is: nothing in `component.rs` is architecture-
// specific, the one instruction underneath it is, and `f_abi::door::call` is
// compiled only where there is a frame to call. The `image` feature because a
// `#[panic_handler]` is a lang item and there may be exactly one in a linked
// artefact. Those are `user/store`'s two, copied.
//
// `target_os = "none"` is the third, and copying `user/store`'s gate without it
// does not compile:
//
//     error[E0152]: duplicate lang item in crate `f_objects`
//                   (which `reads` depends on): `panic_impl`
//
// `#[cfg(not(test))]` on the handler covers this crate's *unit* tests, which are
// compiled with `cfg(test)` set. It does not cover an **integration** test:
// `tests/reads.rs` links `f-objects` as an ordinary dependency, compiled without
// `cfg(test)` and with default features on, against a `std` that brings its own
// `panic_impl`. `user/store` has no `tests/` directory and so never met this;
// this crate's whole exit is an integration test, so it meets it immediately.
//
// `target_os` and not `target_arch` is what separates the two, because the host
// tests run on x86-64 as well — which is the same distinction, for the same
// reason, that `user/store/src/lib.rs` draws about the allocator one paragraph up.
#[cfg(all(target_os = "none", target_arch = "x86_64", feature = "image"))]
pub mod component;

// The serve loop, behind the same three gates as `component` and for the same
// reasons: it calls the door, it is the body of an image, and an integration
// test linking this crate against `std` may not bring a second `panic_impl`
// with it.
#[cfg(all(target_os = "none", target_arch = "x86_64", feature = "image"))]
pub mod serve;

pub mod dma;
// The face half, `E3-B03b`: a typeface out of the store by the content address
// this component's manifest declares, and the refusal of one it does not. It is
// not behind the three image gates above because nothing in it calls the door
// and nothing in it is a lang item — it is a library the component body will
// use and a host test can drive, which is the shape `read` and `service` already
// have.
pub mod face;
pub mod read;
pub mod resident;
pub mod service;
pub mod write;

pub use dma::{Landed, Landing};
pub use read::{Counters, Placement, Read, ReadPath, mount};
pub use resident::Resident;
pub use service::{Served, Service};
