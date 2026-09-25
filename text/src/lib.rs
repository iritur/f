// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Text, and the two constraints that bite harder here than anywhere else.
//!
//! Section 08 of `docs/design/ring-scene-boot.html` calls text the part of a
//! rendering system that is always underestimated. This crate holds the parts
//! of it that can be argued before a shaper exists, which is more than it
//! sounds: an advance is a number, a number in this tree is a fixed-point
//! integer with its scale in its name (RFC 0004), and *where the rounding
//! happens* is a decision that gets made once or gets made at every call site.
//!
//! The second constraint is the shaping cache, and it is the reason this crate
//! has no dependencies. A cache keyed by string, face, size and features is the
//! single most natural place in this system to reach for a `HashMap`, whose
//! iteration order is seeded per process — which would make hit and miss counts
//! differ between two runs of one seed and destroy the only property that makes
//! a cache testable at all. Nothing here can inherit that type from somebody
//! else's public signature.
//!
//! [`cache`] is that cache, and it kept the row empty: its key is the run, the
//! face's address, the grid, the em and the feature set stored **verbatim**, so
//! it hashes nothing and needs nobody's hasher. The sentence above turns out to
//! have understated the problem — a seeded hasher makes the counts irreproducible,
//! which is loud, while a *collision* returns the wrong shaping with the hit
//! count intact, which is not — and that module is written around the second
//! one.
//!
//! Where the shaper itself comes from is `E3-B03a`'s decision, and if the
//! answer is *imported* then it arrives behind the licence boundary and is
//! reached over a ring rather than by a dependency row here. RFC 0003.
//!
//! [`bidi_class`] and [`bidi_brackets`] are the two files in this crate — and in
//! the permissive tree — that are not purely this tree's: they are generated
//! from Unicode's data by `cargo xtask unicode` and carry both licences in their
//! first line (RFC 0114). They hold values and no code; [`property`] is the code
//! that reads them. Neither is a dependency, so the crate still has none.

#![no_std]

pub mod bidi_brackets;
pub mod bidi_class;
pub mod cache;
pub mod corpus;
pub mod face;
pub mod metric;
pub mod property;
