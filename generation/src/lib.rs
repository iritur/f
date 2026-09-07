// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One expression, one root hash.
//!
//! # What the expression is not
//!
//! It is not a language, and the refusal is the design rather than an omission
//! waiting to be corrected. A generation expression is a closed tree of the four
//! fixed-width node kinds `f_abi::store::node` enumerates, and it is:
//!
//! - **not lazy** — there is nothing to force, because there is nothing to
//!   evaluate;
//! - **not recursive** — the tree's depth is three and the encoding cannot
//!   express a fourth, so no fold here needs a bound because none can run away;
//! - **not functional** — no functions, no application, no let;
//! - **not extensible at run time** — the kinds are a `u16` in a wire crate;
//! - and it has **no `import`, no interpolation, no fetch, no conditional, no
//!   path, no environment read and no clock**.
//!
//! The machine never evaluates anything. `generation.toml` is source, `cargo
//! xtask generation` compiles it on a host through the checker in
//! [`record`], and the record tree is the artefact. What the machine does with
//! a root it is handed is recompute the fold over the records — [`fold::root`],
//! the same arithmetic in the same crate — as a check.
//!
//! Nix's language is what is being refused, and the spec states the reversal
//! condition rather than pretending the refusal is free: a topology `E2-B05`
//! cannot express as a tree of these four kinds. The reversal is a fifth node
//! kind, which is a diff to `abi/` and an RFC.
//!
//! # The canonical form
//!
//! *The same expression yields the same root* is only true if the compiler is
//! canonical, so the ordering is enforced here — in the crate that checks — and
//! not merely described somewhere a reader could agree with and an encoder
//! could ignore:
//!
//! - **Components are sorted bytewise on the zero-padded 32-byte name.** The
//!   padded name and not the trimmed one, because the padded name is what the
//!   record holds and what the fold hashes; on an alphabet with no NUL the two
//!   orders agree, and saying which one is normative costs a sentence and
//!   settles the argument before somebody has it.
//! - **Routes are sorted on (component index, capability name).**
//! - **Duplicates are refused**, never merged and never dropped: two components
//!   with one name, or two routes with one (component, capability), are a source
//!   whose author has two beliefs about one thing.
//! - **A non-canonical source is refused with the canonical form printed as a
//!   diff**, never silently reordered — otherwise two authors with the same set
//!   produce two roots, and an author learns their file was rewritten by reading
//!   a hash. That refusal lives in `xtask/src/generation.rs`, because it is
//!   about *source*; what lives here is the property it rests on, which is that
//!   an out-of-order record tree is refused by [`record::Tree::check`].
//!
//! # Why the comparison is here too
//!
//! [`diff`] answers *where do these two trees stop agreeing* and it lives beside
//! the fold rather than in `xtask`, because it is the fold read backwards: the
//! order it descends in is the order [`fold::root`] hashes in, and the two going
//! out of step would make a job report a leaf that is not the one that moved.
//! One file over from the arithmetic it mirrors is the cheapest place for that
//! to stay true. `E2-P06` is its consumer and `xtask` is where the finding is
//! rendered into a sentence, because that is where an allocator is.
//!
//! # Why the boot-time selection is here too
//!
//! [`select`] answers *which of these modules is the one `f.root=` named*, and
//! it is the fold used as an identity rather than as a summary. It lives beside
//! the fold for [`diff`]'s reason and one more: `E2-P07` puts the reader in the
//! frame, `kernel/` is a crate with `test = false` and no host harness, and a
//! selection rule that could only be exercised by booting QEMU is a selection
//! rule whose refusals nothing checks. The frame's share is a loop over the
//! modules the loader gave it; every branch is here.
//!
//! # What this crate does not hold
//!
//! A codec. Every byte layout is `f_abi::store`'s, and the reason is in that
//! module: a decoder living beside a checker is a second implementation of the
//! format, and the whole point of a content address is that there is one.

#![no_std]

pub mod diff;
pub mod fold;
pub mod record;
pub mod select;

pub use diff::{Divergence, divergence};
pub use fold::root;
pub use record::{Refusal, Tree};
pub use select::{Chosen, Examined, Rejected, select};
