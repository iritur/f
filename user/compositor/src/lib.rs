// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The compositor: the component that holds the machine's scene graph.
//!
//! # Why this crate exists, which is not the same as what it does
//!
//! `scene/`, `text/`, `input/` and `interface/` are libraries. Every one of them
//! is green, and until this crate not one of them was linked into anything that
//! boots — so the epoch's whole first half was a set of data structures nobody
//! had put on a core. `scene/src/lib.rs` says why that was the right order and
//! also says what closes it: *the compositor is a component and this is not. A
//! component has a manifest, a state tree and an entry point.* This is that
//! component, and it is the smallest thing that can honestly be called one.
//!
//! What it does, in one paragraph: it is spawned with a manifest that declares
//! a state tree, adopts the control ring the frame produces onto and one
//! client's data ring, takes scene deltas off that ring, holds them in a frame
//! until a commit closes it, applies the closed frame to a retained graph whole
//! or not at all, and publishes what it did into the tree its manifest declared.
//!
//! # Where the graph lives, and why there was no choice
//!
//! In the heap the frame maps, reached through a `Box`. Three constraints leave
//! nowhere else:
//!
//! - `f_scene::arena::Arena` is 131 096 bytes, because `E3-B01c` decided a
//!   thousand slots with no allocator behind them.
//! - A component's stack is four pages — `kernel::process::SPAWN_STACK_PAGES` —
//!   and the paragraph on that constant records what it cost to find out that it
//!   is a real bound and not a ceiling nobody reaches.
//! - A component may hold **no writable static at all**: its text page is mapped
//!   read-only, a mutable global would fault on its first write in a component
//!   with no way to report one, and `cargo xtask component` refuses an image
//!   that has one. `Arena::EMPTY`'s own comment says a `static` is "where a
//!   component's scene actually lives", and for this tree's components that
//!   sentence is false. It is the one place in `f-scene` that assumes an
//!   ordinary program, and finding it is part of what linking the crate into a
//!   component was for.
//!
//! So the graph goes where a component's mutable state can go: `f_ring::heap`,
//! over a region the frame described before the first instruction, which is the
//! same arrangement `user/supervisor` uses and the reason RFC 0094 exists. The
//! cost is a manifest that declares thirty-five pages of heap, and
//! [`HELD_BYTES`] against [`routing::HEAP_BYTES`] is a compile-time assertion
//! rather than a hope.
//!
//! # What putting the graph in a component cost the graph, which is the finding
//!
//! **A component's image carries every byte of every constant it materialises,
//! and `Arena::EMPTY` is 131 096 of them.** The first build of this crate
//! produced a component file of 147 280 bytes against the 65 536 the frame maps,
//! and `cargo xtask component` refused it — the whole difference was the arena's
//! constant sitting in `.rodata` waiting to be copied into the heap.
//!
//! Two changes fixed it and both are written down where they happened. The
//! smaller one is here: [`tree::Held`] borrows a graph and a batch rather than
//! owning them, because an empty batch is *not* all zeroes and an empty arena
//! now is, so the two want different treatment from the compiler. The larger one
//! is in `f_scene::kind`, where the node kinds are now numbered from one — which
//! puts `Option<Created>`'s niche at zero, makes an empty slot all zeroes, and
//! turns a 131 KiB copy out of the image into a `memset` over the heap. That is
//! a layout property no assertion in a crate forbidding `unsafe` can state, and
//! what holds it is this component's own image bound going red with the byte
//! count in the message.
//!
//! It is worth a reader's attention because it is the first thing linking a
//! library into a component taught this tree, and the second is beside it:
//! `Arena::EMPTY`'s own comment says a `static` is "where a component's scene
//! actually lives", and for the components in this tree that sentence is false.
//!
//! # What this is not, listed rather than discovered
//!
//! No renderer and no backend: nothing here draws, and `E3-B02` is what will.
//! It holds a rung and a wake time and acts on neither — `E3-B01k` publishes
//! them and `E3-B02b` is the task that lets the rung decide a renderer, which
//! is a distinction [`tree`]'s own comment keeps. No doorbell: this component
//! spins its loop and `E3-B01g` is the task that puts it to sleep on one, which
//! is also the task that gives the wake time somewhere to be spent. No
//! delta ring between two *components* — the client here is the frame itself,
//! which is the arrangement every datapath boot in this tree has and what
//! `CHAOS_GAP` in `xtask` carries as a debt. No client library: `E3-B01l` built
//! the reconciler and nothing in this crate links it, because the side that
//! produces deltas is the side that would.
//!
//! One thing it does not do is worth stating on its own, because a reader will
//! look for it. **It does not decide a class or schedule against a deadline.** A
//! commit carries one and the wire refuses a commit that carries none; what
//! `E3-B01h` added is that this component now *reads* it — the deadline is what
//! the degradation word is measured against, and it is published where a reader
//! can find it. What still does not exist is an ordering rule: nothing here runs
//! one frame before another, because an ordering rule written before there is
//! anything to order would be a rule nobody could falsify, and the queue it
//! would order is `E3-B01g`'s.

#![no_std]

// The `alloc` crate, and it is the whole of what the image feature adds to this
// crate's dependency graph. Bringing it into scope is what lets the graph be
// boxed; installing the allocator below is a separate decision and a separate
// gate, which is RFC 0094's distinction and the reason these are two `cfg`s
// rather than one.
#[cfg(feature = "image")]
extern crate alloc;

// The allocator, on the machine and in the image only. `Heap::COMPONENT` names
// one address — `f_ring::heap::AT`, which `kernel::process::SPAWN_HEAP` is
// asserted against — and it is safe *because* it names one: a constructor that
// took an address would let safe component code point its allocator at its own
// stack.
#[cfg(all(target_os = "none", feature = "image"))]
#[global_allocator]
static HEAP: f_ring::heap::Heap = f_ring::heap::Heap::COMPONENT;

// Outside every gate, because it is the layout both sides read: the frame writes
// the page this describes and the component reads it, and a set of offsets that
// existed in only one of the two builds would be a layout with two definitions.
// RFC 0047.
pub mod routing;

// The graph and what one entry does to it, compiled everywhere so that a host
// test can drive it on both architectures. Its own comment argues the split.
pub mod tree;

// The pacing arithmetic, compiled everywhere for the same reason and with a
// sharper version of it. `E3-B01h`'s exit says *two runs from one seed compute
// the same wake time to the tick on both architectures*, and the only thing in
// this project that observes the AArch64 half is the host suite on the arm
// runner — a boot runs on x86-64 and nothing else. So this module is not merely
// testable off the machine, it is a module whose exit **cannot** be closed by a
// boot, which is why every function in it is a pure function of its arguments
// and why the clock arrives as a number rather than being read.
pub mod pacing;

// Section 08's chain and one frame's trace, compiled everywhere for `pacing`'s
// reason and with one of its own: `E3-B05b` put the record in `abi/` and recorded
// that nothing in a type makes a *compositor* hand every submission to a
// recorder. This module is the only holder of an `f_abi::sync::Chain` in this
// component, so *every wait went through a door that takes the trace* is a
// property of one file a reader can finish. It is also the module `E3-B05e` will
// read: a supervisor deciding a fate for a stuck frame needs the three words this
// publishes, and nothing here decides one.
pub mod waits;

// The component half is x86-64's, and only because the door is. Nothing in
// `component.rs` is architecture-specific; the one instruction underneath it is,
// and `f_abi::door::call` is compiled only where there is a frame to call. The
// same gate every component crate in this tree has, with the same reversal: an
// AArch64 frame.
//
// The second gate is the `image` feature, and it is off in exactly one place:
// the frame, which links this crate for `routing`'s offsets and for
// [`routing::HEAP_BYTES`]. A `#[panic_handler]` is a lang item and there may be
// one per linked artefact, so the module carrying this component's would
// otherwise collide with the frame's own.
#[cfg(all(target_arch = "x86_64", feature = "image"))]
pub mod component;

/// How many bytes this component's two allocations occupy.
///
/// The graph and the frame under construction. It is here rather than in `tree`
/// because it is a *manifest* number rather than a graph one: the frame reads it
/// when it sizes the heap it maps, and the assertion below is what ties it to the
/// figure a manifest declares.
/// Unit: bytes.
pub const HELD_BYTES: usize = core::mem::size_of::<f_scene::arena::Arena>()
    + core::mem::size_of::<f_scene::commit::Batch<{ tree::FRAME_DELTAS_MAX }>>();

/// What the allocator needs on top of them: the prologue at the foot of the
/// region, and whatever the granularity rounds each of the two blocks up by.
///
/// Written out rather than folded into the assertion, because a reader checking
/// the manifest's thirty-five pages by hand needs the same numbers this does.
/// Unit: bytes.
pub const HEAP_OVERHEAD: usize =
    f_ring::heap::HEADER_BYTES as usize + 2 * f_ring::heap::GRANULARITY as usize;

// The heap this component's manifest declares is large enough for the thing it
// exists to hold — checked by the compiler, because the alternative is a
// component whose first allocation comes back null at ring 3 with nothing in the
// failure naming the cause. A graph that grows past the declared heap stops this
// build instead, and the repair is a number in `routing.rs` and the same number
// in `manifest.toml`.
const _: () = assert!(
    HELD_BYTES + HEAP_OVERHEAD <= routing::HEAP_BYTES as usize,
    "the heap this manifest declares is smaller than the graph this component holds",
);

// And not so large that the frame cannot map it. `kernel::process::HEAP_MAX` is
// sixty-four pages; this crate cannot name that constant — a component may not
// link the frame — so the bound is restated here with its owner named, which is
// the same arrangement `routing::AT` has for `process::BOARD`.
const _: () = assert!(
    routing::HEAP_BYTES <= 64 * 4096,
    "kernel::process::HEAP_MAX is sixty-four pages and a spawn refuses more",
);
