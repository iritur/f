// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The admission arithmetic, proved rather than sampled.
//!
//! # What this crate is
//!
//! `f-abi`, linked as an ordinary dependency, plus the harnesses that state
//! what `f_abi::reserve::Table::admit` promises — over *every* machine the
//! table can be built for and *every* demand a manifest could carry, within a
//! stated bound on the machine's size.
//!
//! # Why the arithmetic gets a proof and not only a fuzzer
//!
//! RFC 0050 put the whole of the schedulability test in one function, on
//! purpose: *two implementations of a schedulability test are two
//! schedulability tests, and the one that gets audited is never the one that
//! ran*. The cost of that decision is that nothing independent checks the one
//! implementation. The frame runs it, the simulator runs it, the host tests run
//! it, and all three run the same lines — so a defect in those lines is a
//! defect every instrument agrees with. `claims/0010` gates on the *count* of
//! refusals the simulator's adversarial load produces, which is evidence that
//! the function refuses and no evidence about what it grants.
//!
//! The occasion for writing this down was somebody else's cross-check. The
//! AxonOS kernel compared its response-time analysis against an independent
//! tool, found agreement, and then found that the task set it had used never
//! exercised the analysis at all — every ceiling was one, and *what has been
//! verified is addition*. A second implementation that agrees is only worth
//! what the inputs reach. A solver is the instrument that reaches all of them,
//! so this crate asks one, and it asks the shipped function rather than a
//! model of it: RFC 0057's shape, a path dependency and no second copy.
//!
//! # What is proved, and inside what bound
//!
//! * **The demand is not bounded.** Every field of a [`f_abi::reserve::Demand`]
//!   is symbolic — the class and the domain across all 256 values each, the
//!   core count, the period, the budget and the memory across their whole
//!   width — so the unknown classes R04 refuses, the periods the frame's clock
//!   cannot observe and the memory outside the huge-page grain are all in
//!   scope.
//! * **The machine's shape is not bounded.** Sibling count, cache and
//!   bandwidth domain widths, partition count, the frame's own cores, the pool
//!   and the tick are all symbolic, and the only assumption is the one
//!   [`f_abi::reserve::Machine::check`] itself makes, because a table cannot
//!   be built over a machine that fails it.
//! * **The machine's size is bounded.** [`proofs::CORES`] caps the physical
//!   core count at eight, because `Table::admit` walks the cores looking for a
//!   free run and a bounded checker unrolls a loop rather than summarising it.
//!   `wide-machine` is what stops that being an argument: the two harnesses
//!   whose sentences are about the walk are run again at sixty-four, which is
//!   [`f_abi::reserve::CORES_MAX`] and the whole of what the bitmaps can name.
//!   The other two are not, and `xtask`'s `ABI_PROOF_HARNESSES` says what
//!   that costs — two hours for one, the checker's memory for the other — and
//!   why it concedes nothing about the walk.
//! * **The history is bounded, and by construction.** A harness builds its
//!   table by running real grants with symbolic demands — at most two of them
//!   — and never writes a slot. Every state a proof holds for is a state the
//!   table can reach. What is *not* covered is a table with three or more
//!   grants in it, and that is stated rather than implied: the properties here
//!   are about one admission against whatever is held, and two grants are the
//!   smallest history in which *two grants never share a core* is a sentence.
//!
//! # The half that makes the other half mean anything
//!
//! **The defect.** `cargo xtask prove` arms `mutate-overlapping-grant`, which
//! makes the free-run search in `abi/src/reserve.rs` stop consulting what is
//! already taken, and requires [`proofs::two_grants_never_share_a_core`] to
//! fail — with *two grants hold one core* as the sentence the failing check
//! carries. A proof that passes on a build with a known defect in the code it
//! is about is not a proof of anything.
//!
//! **The covers.** Every harness carries `kani::cover!` for each answer it can
//! produce, and `cargo xtask prove` refuses a report in which one cannot be
//! reached. This crate is the case the rule was written for: a fixture that
//! draws an arbitrary `Machine` and then assumes it checks out is one
//! over-tight `assume` away from a machine nothing can be admitted to, and
//! that harness verifies instantly and proves nothing.
//!
//! # Why it is not a workspace member
//!
//! For RFC 0022's reason, which is unchanged: the toolchain that builds this is
//! the checker's and not the tree's. The root `Cargo.toml`'s `exclude` is what
//! keeps the two apart, and deleting this directory is the whole of undoing it.

#![cfg_attr(not(kani), allow(dead_code))]

// Not `#[cfg(kani)]`. The harnesses are what call `f-abi` — `Table::new`,
// `Table::admit`, `Table::grant`, `Table::release` — and a module the ordinary
// build compiles out is a module the ordinary build cannot notice an API
// change under. Only the *attributes* are the checker's, so only they are
// conditional; see the `kani` shim below for what the calls mean without one.
pub mod proofs;

/// What `kani::any`, `kani::assume` and `kani::cover!` mean when there is no
/// checker.
///
/// `ring/proofs/src/lib.rs` argues this shim at length and the argument is not
/// repeated: it exists so that `cargo xtask lint-proofs` can build this crate
/// under the *pinned* nightly, in every feature configuration, with no Kani
/// anywhere, and thereby notice `f-abi`'s API moving under the harnesses in a
/// second rather than twenty minutes into a nightly job. A value from here can
/// only ever be typechecked — nothing reachable from a `#[kani::proof]` is
/// compiled with this in scope, and nothing else in the crate has a `main`, a
/// test or a benchmark — and it is deliberately the least interesting value
/// rather than a plausible one.
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
    macro_rules! cover {
        ($cond:expr, $why:expr $(,)?) => {{
            let _ = ($cond, $why);
        }};
    }

    /// The macro above, reachable as `kani::cover!` the way the checker's is.
    pub(crate) use cover;

    /// A type this shim can answer with.
    ///
    /// A trait of its own rather than `Default`, so that drawing a type the
    /// harnesses do not actually draw is a compile error here until somebody
    /// has looked at whether the real `kani::Arbitrary` covers it.
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

    zeroed!(bool = false, u8 = 0, u32 = 0, u64 = 0);
}
