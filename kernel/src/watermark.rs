// SPDX-License-Identifier: Apache-2.0 OR MIT
//! An untyped account's watermark, and the three questions it is asked.
//!
//! # Why this file exists, and why it is not in `cap.rs`
//!
//! **RFC 0096.** A deductive proof here annotates the file the kernel ships —
//! not a copy of it and not a model of it, which is RFC 0053's rule and the
//! reason `kernel/proofs` reaches `cap.rs` through `#[path]` rather than
//! holding a second copy. That mechanism does not transfer to Verus: Kani
//! verifies ordinary Rust, so an unannotated second compile is a complete
//! input, while Verus verifies only what is written inside `verus!{ … }` with
//! `requires` and `ensures`. Either the shipped file carries the annotations or
//! there is a model beside it, and 0096 takes the first.
//!
//! What it does not have to take is *the whole of `cap.rs`*. Verus runs a
//! compiler front end over its input, so a file that says `use crate::…` needs
//! the crate around it — which is why `kernel/proofs` carries three stand-in
//! modules. This file names nothing above itself. It is arithmetic over four
//! `u64`s, it is a module of `f-kernel` like any other, and `verus` reads it as
//! a crate root exactly as it sits on disk. `cargo xtask verus` is the route.
//!
//! # What is proved, and why these three
//!
//! Because they are the frame's only arithmetic on an account's watermark, and
//! **two of them carried a precondition nobody had written down**:
//!
//! - `Table::retype` advanced the watermark with `wrapping_add` under the
//!   comment *"Checked immediately above, so neither of these can wrap"*. What
//!   was checked immediately above is `extent >= FRAME_SIZE`, which bounds the
//!   subtraction and says nothing at all about the addition. An account whose
//!   `object` is within a frame of the end of the address space wraps to zero
//!   and hands out a frame at address zero.
//! - `Table::refund` did `slot.extent = slot.extent.saturating_add(bytes)`, so
//!   `object + extent` was not preserved: a refund whose `extent + bytes` would
//!   pass `u64::MAX` lost the difference silently. That one was found by
//!   pointing Verus at a reduction of the function on 2026-09-20 and is what
//!   RFC 0096 was written from — the first result in this tree that shows the
//!   difference between a bounded check and a deductive one rather than arguing
//!   it. `kernel/proofs/src/mem.rs` sets `FRAME_SIZE` to 256 and the harnesses
//!   run an eight-slot table, so `u64` overflow is not in Kani's space and
//!   never will be.
//!
//! **Neither is claimed to be reachable on a running machine.** `extent` is
//! bytes left in an untyped region and no machine here has one that large, and
//! `object` is a physical address. What the refutations establish is that the
//! preconditions were never stated, which is a different and checkable claim.
//! Both are stated here now, and the arithmetic that rests on them is refused
//! rather than wrapped.
//!
//! # Conservation, which is the property both moves share
//!
//! `object + extent` is the top of the account and does not move: carving a
//! frame walks the base up and the remainder down by the same number, and a
//! refund walks them back. Every `ensures` below says so in those words, and it
//! is the postcondition that fails when the guard is taken away — see
//! `mutate-saturating-refund`, which `cargo xtask verus` arms and requires this
//! file to be named by.
//!
//! # What it costs
//!
//! `f-kernel` takes `verus_builtin` and `verus_builtin_macros` as ordinary
//! dependencies, pinned by `=` because both are published by date rather than
//! semantically. RFC 0096 states that price and it is paid here: the frame is
//! the one tree RFC 0001 concentrates `unsafe` in, and this is the first
//! dependency it has taken for a checker rather than for a thing it does.
//! Under rustc the macro erases every specification and what is left is the
//! three function bodies below.

// The spec traits — `SpecAdd`, `SpecSub` and the rest — that `+` and `-` inside
// a `requires` or an `ensures` resolve against. Under Verus the import is
// load-bearing; under rustc the macro has erased every expression that used it
// and the import has no user left, which is the one line of cost this file pays
// for being compiled by two compilers. `allow` and not `expect`, because an
// expectation fulfilled under one of them and not the other is a warning under
// `-D unfulfilled-lint-expectations` whichever way round it is written.
#[allow(unused_imports)]
use verus_builtin::*;
use verus_builtin_macros::verus;

verus! {

/// May a frame be carved off this account?
///
/// The predicate `Table::grow` selects an account by, and the one
/// [`carved`] refuses on — one function rather than two, so that a
/// selection which says yes cannot be followed by a charge that says no.
///
/// The second clause is the one that was missing: `extent >= frame` bounds the
/// subtraction and says nothing about the addition.
pub fn carvable(object: u64, extent: u64, frame: u64) -> (out: bool)
    ensures
        out == (extent >= frame && object + frame <= u64::MAX),
{
    extent >= frame && object <= u64::MAX - frame
}

/// Carve a frame off the top of an account, and answer where the watermark
/// lands.
///
/// `None` is an account that cannot afford one, by either clause of
/// [`carvable`]. The caller's refusal is its own — `Table::retype` answers
/// `RESOURCE/EXHAUSTED` for both, because from a holder's side there is no
/// difference between a region with nothing left and a region whose next frame
/// would not have an address.
pub fn carved(object: u64, extent: u64, frame: u64) -> (out: Option<(u64, u64)>)
    ensures
        match out {
            Some((base, left)) => base == object + frame && left + frame == extent
                && base + left == object + extent,
            None => extent < frame || object + frame > u64::MAX,
        },
{
    if extent < frame {
        return None;
    }
    if object > u64::MAX - frame {
        return None;
    }
    Some((object + frame, extent - frame))
}

/// Give `bytes` back to the top of an account, and answer where the watermark
/// lands.
///
/// `floor` is where the region started: a refund that would take the base below
/// it is the frame having lost count, and is refused rather than allowed to
/// hand out memory nobody owns.
///
/// `None` is that case, or either of the two overflows — and the third clause
/// of the `ensures` is the one the shipped function did not have. It used to
/// do `extent.saturating_add(bytes)` and answer `Ok`, so a refund at the top of
/// the range moved the base down and did not move the remainder up by as much.
pub fn refunded(object: u64, extent: u64, bytes: u64, floor: u64) -> (out: Option<(u64, u64)>)
    ensures
        match out {
            Some((base, left)) => base + bytes == object && left == extent + bytes
                && base + left == object + extent && base >= floor,
            None => floor + bytes > u64::MAX || object < floor + bytes
                || extent + bytes > u64::MAX,
        },
{
    if floor > u64::MAX - bytes {
        return None;
    }
    if object < floor + bytes {
        return None;
    }
    // The guard the shipped function did not have, and the one
    // `mutate-saturating-refund` takes away: with it gone the conservation
    // clause of the `ensures` above is refuted and `cargo xtask verus` goes
    // red naming this file. It is a `cfg` and not an edit for the reason every
    // other defect in `DEFECTS` is one — a defect that lives in a patch is a
    // defect nobody runs.
    #[cfg(not(feature = "mutate-saturating-refund"))]
    if extent > u64::MAX - bytes {
        return None;
    }
    Some((object - bytes, extent + bytes))
}

} // verus!
