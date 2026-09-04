// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The ring's validation paths, proved rather than sampled.
//!
//! # What this crate is
//!
//! `f-ring` and `f-abi`, linked as ordinary dependencies, plus the harnesses
//! that state *nothing a peer writes produces a panic* over a shared mapping
//! every byte of which a solver chose.
//!
//! # Why it links the crate rather than recompiling a file out of it
//!
//! Because it can, and `kernel/proofs` could not. RFC 0053 compiled
//! `kernel/src/cap.rs` a second time through `#[path]` against three stand-ins,
//! and was right to: that file lives inside a bare-metal binary crate that
//! cannot be built for the host, so there was no other way to reach it.
//! `f-ring` is an ordinary `no_std` library whose own tests already run on the
//! host. So this is a path dependency, there is no second copy, and the
//! question RFC 0053 spends a section on — *does the proof still reach the file
//! the kernel ships* — does not arise here. Same guarantee, one fewer
//! mechanism. RFC 0057.
//!
//! # Why it is not a workspace member
//!
//! For RFC 0022's reason, which is unchanged: the toolchain that builds this is
//! the checker's and not the tree's. `cargo kani setup` installs a rustc of its
//! own, so a crate inside the workspace would be built by `cargo xtask test`
//! under the pinned nightly and would be either a second build that proves
//! nothing or a build failure nobody asked for. The root `Cargo.toml`'s
//! `exclude` is what keeps the two apart, and deleting this directory is the
//! whole of undoing it.
//!
//! # What is proved, and inside what bound
//!
//! Stated once here, argued at each constant, and argued at more length in
//! RFC 0057.
//!
//! * **The peer's bytes are not bounded.** [`peer::Region::scribbled`] is a
//!   mapping whose every byte is symbolic. The header, both pairs of cursors,
//!   the flags word, the index ring, both entry arrays and the arena are then
//!   the *same* bytes, in the relationship they stand in when somebody else
//!   holds the far end — which a struct of separate fields cannot express, and
//!   which is why the fixture is a region.
//! * **The cursors and the slot numbers are not bounded.** All 2^32 of each,
//!   including the ones that make a wrapping occupancy read as full and the
//!   ones that name an entry past the end of the array.
//! * **The mapping is bounded.** [`peer::REGION`] is 640 bytes, which
//!   `f_abi::layout` turns into a ring of one or two entries. That is the
//!   bound, and `wide-ring` is what stops it being an argument: the four
//!   harnesses that read a region are run again at a region holding a ring of
//!   eight.
//! * **Four smaller bounds, each stated where it costs something.**
//!   `proofs::ARENA` is eight bytes, so `write_serial`'s chunking loop makes
//!   one pass; `proofs::REGISTERED` and `proofs::SETS` bound a registration's
//!   *geometry* so that a buffer's stride is small — while the index a peer
//!   presents stays all 2^32, because the index is the peer's and the stride is
//!   not; and the registration harnesses run at a **depth of one**, meaning one
//!   registration and one operation against a table built a line earlier, so
//!   the quantifier is over inputs and not over histories. `proofs.rs`'s
//!   section 4 argues that one, and `ring/tests/entries.rs` is the instrument
//!   that does cover histories.
//!
//! # The half that makes the other half mean anything
//!
//! Two halves, and the second one is new relative to `kernel/proofs`.
//!
//! **The defects.** `cargo xtask prove` arms five of the nine deliberate
//! defects `ring/Cargo.toml` carries and requires each to fail *the harness
//! that states the property it breaks*. A proof that passes on a build with a
//! known defect in the code it is about is not a proof of anything. The four it
//! does not arm are in `xtask`'s `RING_PROOF_BLIND` with the reason each cannot
//! be seen from here, because a list of five green defects beside a manifest of
//! nine is how a reader comes to believe more than was checked.
//!
//! **The covers.** Every harness carries `kani::cover!` for each answer it can
//! produce, and an unsatisfiable cover is a *failed* verification. This is the
//! answer to the question this repository asks of a green result — *what input
//! would make it green while the property was false* — and for a proof over
//! arbitrary bytes the answer is precise: bytes that never get past the first
//! check. The first draft of `proofs::popping_an_arbitrary_entry` verified in
//! under a second because nothing it drew was ever adopted, which is a harness
//! that cannot fail wearing a proof's clothes.
//!
//! **The checker does not enforce that**, and the sentence above is only true
//! because something else does. Kani 0.67.0 prints
//! `1 of 2 cover properties satisfied (1 unreachable)` and then
//! `VERIFICATION:- SUCCESSFUL`, exit 0; there is no flag in it that changes
//! that. So `cargo xtask prove` reads the count out of every report and refuses
//! — `xtask`'s `cover_check`, and `ProofCrate::covered` says which crates owe a
//! cover line at all. A rule this file rests its own honesty on, stated in five
//! comments and mechanised in none, was the CONTRIBUTING R01 case that is worse
//! than an honestly manual one: a check somebody believes is happening.

#![cfg_attr(not(kani), allow(dead_code))]

pub mod peer;

// Not `#[cfg(kani)]`. The harnesses are what call `f-ring` — `Consumer::pop`,
// `Mapping::adopt`, `Table::resolve`, `execute` — and a module the ordinary
// build compiles out is a module the ordinary build cannot notice an API change
// under. Only the *attributes* are the checker's, so only they are conditional;
// see the `kani` shim below for what the calls mean without one.
mod proofs;

/// What `kani::any` and `kani::assume` mean when there is no checker.
///
/// # Why this exists at all
///
/// So that `cargo xtask lint-proofs` can build this crate under the *pinned*
/// nightly, in every feature configuration, in about fifteen seconds and with
/// no Kani anywhere. `kernel/proofs` gets that for free because its fixture is
/// three stand-in modules with no checker in them; this one's fixture calls
/// `kani::any()` on every line, so without a shim the ordinary build would not
/// compile and the lint would have nothing to check.
///
/// **What that build is for**, since it is not the proofs: it is the thing that
/// notices `f-ring`'s API moving under the fixture. `peer::Lane` implements
/// `f_ring::buffers::Submitter`, `peer::Domain` implements
/// `f_ring::registry::Domains`, `peer::Bucket` implements `f_ring::Sink` — a
/// change to any of those three traits breaks a build that `cargo xtask verify`
/// does not run, and the nightly would discover it twenty minutes into a job
/// hours after the person who made the change had moved on. RFC 0057's
/// `lint-proofs` section is the argument; this module is what lets it be a
/// mechanism rather than a sentence.
///
/// It covers the harnesses and not only the fixture, and the difference is the
/// whole value: the three trait implementations are in `peer`, but every call
/// this crate makes into the code it proves — `Consumer::pop`, `Collector::take`,
/// `Mapping::adopt`, `Table::register`, `execute`, `BufferSet::carve` — is in
/// `proofs`. A `mod proofs` behind `#[cfg(kani)]` would have left that half
/// unbuilt, so a changed signature would still have been the nightly's
/// discovery. Which is why the shim owes a `cover!` as well as an `any`.
///
/// # Why answering zero is safe here and would not be anywhere else
///
/// Because nothing reachable from a `#[kani::proof]` is compiled with this in
/// scope, and nothing else in this crate has a `main`, a test or a benchmark.
/// A value from here can only ever be typechecked. It is deliberately the
/// *least* interesting value rather than a plausible one: a shim that returned
/// something a test could mistake for a draw is a shim somebody will eventually
/// run.
#[cfg(not(kani))]
pub mod kani {
    /// A value, for a build that only typechecks. See the module comment.
    #[must_use]
    pub fn any<T: Zeroed>() -> T {
        T::zeroed()
    }

    /// Nothing. The checker's `assume` prunes a state space; there is no state
    /// space here.
    pub fn assume(_holds: bool) {}

    /// Nothing, for the same reason, and it must still typecheck its condition.
    ///
    /// A cover states that some answer is reachable, which is a question only a
    /// solver can be asked. What this build is for is the *other* half: the
    /// condition is ordinary Rust against `f-ring`'s API — `done.executed > 0`,
    /// `entry.opcode == op::WRITE_SERIAL` — so it goes through the type checker
    /// here even though nothing evaluates it.
    ///
    /// Expands to a block rather than a statement because `kani::cover!` is
    /// written in expression position in this crate, as the body of a `match`
    /// arm.
    macro_rules! cover {
        ($cond:expr, $why:expr $(,)?) => {{
            let _ = ($cond, $why);
        }};
    }

    /// The macro above, reachable as `kani::cover!` the way the checker's is.
    pub(crate) use cover;

    /// A type this shim can answer with.
    ///
    /// A trait of its own rather than `Default`, because the point is to be
    /// unable to answer for a type the harnesses do not actually draw: adding a
    /// draw of a new type should be a compile error here until somebody has
    /// looked at whether the real `kani::Arbitrary` covers it.
    pub trait Zeroed {
        /// The least interesting value of this type.
        fn zeroed() -> Self;
    }

    macro_rules! zeroed {
        ($($t:ty = $v:expr),* $(,)?) => {
            $(impl Zeroed for $t {
                fn zeroed() -> Self {
                    $v
                }
            })*
        };
    }

    zeroed!(bool = false, u8 = 0, u16 = 0, u32 = 0, u64 = 0, i32 = 0, usize = 0);

    impl<const N: usize> Zeroed for [u8; N] {
        fn zeroed() -> Self {
            [0; N]
        }
    }
}
