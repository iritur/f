// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One derivation, shared: how a seed becomes a stream, and how a stream
//! becomes several without any of them learning about the others.
//!
//! # What this replaces, and why one function instead of two
//!
//! There were two generators here and neither said anything true about itself.
//! [`crate::SeededEnv`] ran xorshift64 and [`crate::sim`] hashed a site label
//! with FNV-1a and finished it with SplitMix64's finaliser, and both carried the
//! same admission in a comment: chosen for reproducibility, not for statistical
//! quality. That is a defensible thing to write when a seed is used once. It
//! stops being defensible the moment the simulator multiplies streams, because
//! a sweep across streams that are secretly correlated explores less than it
//! reports, and reports the smaller number as the larger one — the one failure
//! mode a test apparatus must not have.
//!
//! So there is one derivation and everything is built from it. [`mix`] is the
//! finaliser. [`derive`] keys a seed by an identity. [`Stream`] is the
//! generator, and [`Stream::split`] makes a child from a parent *by identity*
//! rather than by consuming the parent's output — which is the property that
//! makes streams addable without disturbing the ones already there.
//!
//! # Why splitting is by identity and not by draw
//!
//! The usual way to make a second stream is to draw a seed from the first. That
//! makes the child's whole trajectory depend on how many values the parent had
//! already produced, so adding a consumer anywhere shifts every stream created
//! after it. `sim.rs` has that argument written out at length and pays for it
//! with the per-site independence property; the same argument is the reason
//! [`Stream::split`] takes an identity and does not touch `self`.
//!
//! A child is therefore a pure function of the parent's origin and the identity.
//! Ask for the same identity twice and you get the same stream. Ask for a new
//! one and nothing that already exists moves.
//!
//! # No floating point, deliberately
//!
//! Every quantity here is an integer and the tests hand-roll their statistics in
//! integer arithmetic. Two independent reasons, either sufficient. This crate is
//! compiled into a kernel that runs with the FPU in whatever state the firmware
//! left it and does not save vector registers across its own entries, so a float
//! on this path is a fault or a corruption rather than a number. And a
//! determinism substrate that rounded would be reproducible only across machines
//! that round identically, which is a smaller set than "any machine at this
//! commit" and not the set the contract names.
//!
//! # Seeds bind to a commit
//!
//! `(seed, commit_hash) -> byte-identical execution` says nothing about two
//! different commits, and that is the whole of the migration story: changing the
//! generator changes what every seed means, and a new generator is a new commit.
//! A recorded seed still reproduces the run it was recorded against, because it
//! was always recorded against a commit. Nothing in `claims/`, `ops/` or
//! `.github/workflows/` names a seed; `kernel/src/main.rs` names one, and the
//! digest it prints is a fixture for the commit it was printed at.
//!
//! RFC 0026, RFC 0004.

/// The golden-ratio odd constant, `floor(2^64 / phi) | 1`.
///
/// Odd, so adding it repeatedly walks all of `u64` rather than a subgroup of it,
/// and irrational in the ratio it approximates, so successive additions spread
/// rather than clustering. This is SplitMix64's increment and it is here for the
/// same reason it is there.
const GOLDEN: u64 = 0x9E37_79B9_7F4A_7C15;

/// SplitMix64's finaliser: the one mixing function in this crate.
///
/// Two xor-shift-multiply rounds and a final xor-shift, with Stafford's
/// constants. It is a bijection on `u64` — every step is invertible — which is
/// the property the rest of this module leans on, and it has good avalanche:
/// flipping one input bit changes each output bit with probability close to a
/// half. That is measured in the literature and is not re-measured here; what
/// this module tests is the properties it needs, not the ones it inherits.
///
/// It is not a cryptographic hash and is not used as one. Anybody who can choose
/// inputs can find a collision in the two-argument construction below with
/// birthday work, and nothing in this system's threat model hands an adversary a
/// site label or a seed.
#[must_use]
pub const fn mix(z: u64) -> u64 {
    let mut z = z;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Key a seed by an identity: the whole of the splitting rule.
///
/// The identity is mixed *before* it meets the parent. Identities in this system
/// are small structured integers — a short label's hash, an occurrence counter
/// that goes 0, 1, 2 — and a structured value xored straight into a seed leaves
/// its structure in the bits the seed does not reach. That is what the code this
/// replaces did, and it is why `sim.rs` had to rotate its label left by 17 bits
/// before use: a fix for a symptom of combining before mixing.
///
/// Two properties, both relied on and both tested:
///
/// - **Injective in each argument.** Every step — add a constant, [`mix`], xor —
///   is a bijection on `u64`, so for a fixed parent no two identities can
///   produce the same seed, and for a fixed identity no two parents can. Two
///   distinct identities cannot collide onto one stream at all, rather than not
///   colliding with high probability. What that does *not* cover is two callers
///   arriving at one identity — [`label`] is a hash and can — so the claim ends
///   where the identity begins.
/// - **Asymmetric.** `derive(a, b)` and `derive(b, a)` differ, because the
///   identity passes through a mix the parent does not. A symmetric derivation
///   would make a seed and a site label interchangeable, which is not a bug
///   anybody would hit and is still something a reader would have to check was
///   not one.
#[must_use]
pub const fn derive(parent: u64, identity: u64) -> u64 {
    mix(parent ^ mix(identity.wrapping_add(GOLDEN)))
}

/// Turn a stable text label into an identity for [`derive`].
///
/// FNV-1a, kept from the code this replaces, and demoted. Its avalanche is poor
/// — a multiply and an xor per byte with no finalisation, so two labels
/// differing in one late byte leave that difference sitting in the low bits.
/// That mattered when the result was used as a seed. It does not matter now,
/// because the result is an *identity* and [`derive`]'s first act is to mix it.
/// A weak function used where its weakness is answered is cheaper than a strong
/// one used where nothing needs it.
///
/// Callers pass compile-time constants, so this is a handful of instructions on
/// a path that only runs under simulation.
#[must_use]
pub const fn label(name: &str) -> u64 {
    let bytes = name.as_bytes();
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    let mut i = 0;
    while i < bytes.len() {
        hash ^= bytes[i] as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
        i += 1;
    }
    hash
}

/// How many steps a freshly seeded stream discards, in steps.
///
/// The state is four words and one step moves each of them one place along the
/// chain, so three steps are what it takes for a bit in `a` to have reached `c`
/// and come back out through the output. Twelve is four times that, and it is
/// the number the generator's author specifies. It costs about a hundred
/// instructions, once per stream.
const WARMUP: u32 = 12;

/// A generator: SFC64, seeded from one word.
///
/// # What it is
///
/// Chris Doty-Humphrey's "small fast counting" generator, the 64-bit variant, as
/// shipped in PractRand. Three chaining words and one counter, and the counter
/// is added into every output rather than being decoration.
///
/// # Period, stated rather than implied
///
/// **The minimum period is 2^64 outputs, and it is a guarantee rather than an
/// observation.** The counter increments by one every step and nothing feeds it
/// back into itself, so the state cannot repeat until the counter wraps. The
/// average period over seeds is far longer — on the order of 2^255 — but that is
/// a statistical statement about the chaining words, and the floor is the number
/// worth quoting because it is the one that holds for the seed you actually
/// have. At one draw per nanosecond that floor is five hundred years.
///
/// # No all-zero fixed point, and no guard for one
///
/// xorshift needed a guard because all-zero is a fixed point of a linear map.
/// This generator has no fixed point at all, for the same reason it has a period
/// floor: the counter changes every step whatever the rest of the state is
/// doing, so no state maps to itself. Starting the whole state at zero produces
/// one zero output and then leaves, which is tested rather than asserted. That
/// is why [`Stream::from_seed`] contains no check for a zero seed, and why the
/// one [`crate::SeededEnv`] used to carry is gone.
///
/// # Known weaknesses, because this section is what the task existed to write
///
/// - **Not cryptographic, and not close.** The output is the pre-update sum of
///   two state words and the counter; an observer with a handful of consecutive
///   outputs recovers the state. Capability tokens and address-space layout at
///   M4 need unpredictability against an adversary and must not be built on
///   this. `kernel/src/env.rs` records the same reversal against the generator
///   this one replaces.
/// - **Tested empirically, not proved.** Its author reports no failure in
///   PractRand out to 32 TB of output. That is evidence about one family of
///   tests, not a theorem, and there is no equidistribution result for it —
///   unlike the xorshift family, which has proofs and fails different tests.
/// - **The period floor is a floor.** A particular seed's actual period is not
///   known and is not cheaply knowable.
///
/// *Reversal:* a statistical failure found by a seed sweep, or a consumer that
/// needs unpredictability rather than reproducibility. Either is a different
/// generator and therefore a different commit, which is the whole migration
/// story.
#[derive(Clone, Debug)]
pub struct Stream {
    a: u64,
    b: u64,
    c: u64,
    /// Steps taken, plus one. Never read back into `a`, `b` or `c`, which is
    /// what makes the period floor a guarantee rather than a hope.
    counter: u64,
    /// The seed this stream was derived from, kept so [`Stream::split`] can key
    /// a child without drawing anything. Storing it is the entire difference
    /// between splitting by identity and splitting by output.
    origin: u64,
}

impl Stream {
    /// Seed a stream from one word.
    ///
    /// The three chaining words come from a SplitMix64 *sequence* off the seed
    /// rather than from three copies of it: a stream seeded with one word in all
    /// three places starts on the diagonal of the state space and takes longer
    /// to leave it, and the sequence costs three multiplies. The sequence is
    /// injective in the seed, so two seeds never share a starting state.
    #[must_use]
    pub const fn from_seed(seed: u64) -> Self {
        let s1 = seed.wrapping_add(GOLDEN);
        let s2 = s1.wrapping_add(GOLDEN);
        let s3 = s2.wrapping_add(GOLDEN);

        let mut stream = Self { a: mix(s1), b: mix(s2), c: mix(s3), counter: 1, origin: seed };

        let mut i = 0;
        while i < WARMUP {
            let _ = stream.next_u64();
            i += 1;
        }
        stream
    }

    /// The seed this stream was derived from.
    #[must_use]
    pub const fn origin(&self) -> u64 {
        self.origin
    }

    /// A child stream, independent of this one and of every other child.
    ///
    /// Takes `&self`. Splitting draws nothing, advances nothing, and is
    /// idempotent: an identity always names the same child, and a child created
    /// today is the child that would have been created before any of the others
    /// existed. That is what lets a later commit add a stream without
    /// invalidating a seed recorded before it — the property `sim.rs` spends
    /// four paragraphs defending for sites, held here for streams in general.
    #[must_use]
    pub const fn split(&self, identity: u64) -> Self {
        Self::from_seed(derive(self.origin, identity))
    }

    /// The next value, and one step of the state.
    ///
    /// `const` so that [`Self::from_seed`] can run its warm-up inside a
    /// `const fn`, which is what keeps [`crate::SeededEnv::new`] const and
    /// therefore usable where the kernel uses it.
    pub const fn next_u64(&mut self) -> u64 {
        // The three shifts are the author's — 11 right on `b`, 3 left on `c`, a
        // 24-bit barrel rotate — and they are tuned as a set against PractRand.
        // Changing one of them is choosing a different generator, not tuning
        // this one.
        const RIGHT: u32 = 11;
        const LEFT: u32 = 3;
        const ROTATE: u32 = 24;

        let out = self.a.wrapping_add(self.b).wrapping_add(self.counter);
        self.counter = self.counter.wrapping_add(1);
        self.a = self.b ^ (self.b >> RIGHT);
        self.b = self.c.wrapping_add(self.c << LEFT);
        self.c = self.c.rotate_left(ROTATE).wrapping_add(out);
        out
    }
}

/// The mean of the bit-agreement count over `draws` pairs of outputs.
///
/// Sixteen bits per draw: half of the low 32 the statistic looks at.
#[cfg(test)]
pub(crate) const fn agreement_mean(draws: u64) -> u64 {
    16 * draws
}

/// How far a bit-agreement count may sit from [`agreement_mean`] before a test
/// calls it correlation, for `draws` pairs of outputs compared over their low 32
/// bits.
///
/// Five standard deviations of the binomial the count is, computed rather than
/// tuned. Two independent uniform streams make each of the `32 * draws` bit
/// comparisons an independent fair coin, so the number of agreements is
/// `Binomial(32 * draws, 1/2)`: mean `16 * draws`, variance `8 * draws`, and a
/// standard deviation of the square root of `8 * draws`.
///
/// The square root is an integer floor, which makes the band very slightly
/// tighter than five sigma — stated rather than rounded away. At five sigma a
/// correct generator falls outside with probability about 6e-7 per pair, so a
/// test comparing a few dozen pairs would be wrong about a correct generator
/// roughly once in fifty thousand runs. These tests run on fixed seeds, so in
/// practice they either pass forever at this commit or fail at it; the sigma
/// argument is what says the band was derived rather than fitted.
#[cfg(test)]
pub(crate) const fn agreement_band(draws: u64) -> u64 {
    5 * (8 * draws).isqrt()
}

/// Bits of the low 32 that these two values agree on.
///
/// The whole statistic, and its whole limitation: a per-bit-position marginal
/// test on a pair of streams. It detects two streams that are the same stream,
/// that share bits, or that agree bit for bit at the same index more or less
/// often than chance. It does not detect correlation *between* bit positions,
/// structure across successive outputs, anything in the high 32 bits, or a
/// stream that is another one at a different phase — which is why the phase case
/// has a test of its own rather than being assumed away.
#[cfg(test)]
pub(crate) const fn agreeing_bits(x: u64, y: u64) -> u32 {
    (!(x ^ y) & 0xffff_ffff).count_ones()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Outputs compared per pair of streams, in draws. Large enough that the
    /// band is a few hundredths of a percent of the mean, small enough that this
    /// module's tests are a few hundred thousand integer operations.
    const DRAWS: u64 = 1024;

    /// Streams compared. Eight gives twenty-eight pairs, which is enough that a
    /// derivation correlating one identity with one other would be caught.
    const STREAMS: usize = 8;

    fn agreement(mut x: Stream, mut y: Stream) -> u64 {
        let mut total = 0u64;
        for _ in 0..DRAWS {
            total += u64::from(agreeing_bits(x.next_u64(), y.next_u64()));
        }
        total
    }

    #[test]
    fn the_finaliser_does_not_collide_on_the_inputs_identities_actually_are() {
        // Not a proof over 2^64 inputs: every step of `mix` is individually
        // invertible and that is the argument. This checks the consequence over
        // the range where a mistranscribed constant would show — small
        // consecutive integers, which is what an occurrence counter is.
        const N: usize = 512;
        let mut seen = [0u64; N];
        for (i, slot) in seen.iter_mut().enumerate() {
            *slot = mix(i as u64);
        }
        for i in 0..N {
            for j in (i + 1)..N {
                assert_ne!(seen[i], seen[j], "mix collided on {i} and {j}");
            }
        }
    }

    #[test]
    fn derivation_is_injective_in_both_arguments() {
        // What makes "two sites cannot share a stream" a statement rather than a
        // probability.
        const N: usize = 256;
        let mut by_identity = [0u64; N];
        let mut by_parent = [0u64; N];
        for i in 0..N {
            by_identity[i] = derive(0xDEAD_BEEF_0BAD_F00D, i as u64);
            by_parent[i] = derive(i as u64, 0x5E1F_1234_5678_9ABC);
        }
        for i in 0..N {
            for j in (i + 1)..N {
                assert_ne!(by_identity[i], by_identity[j], "identities {i} and {j} collided");
                assert_ne!(by_parent[i], by_parent[j], "parents {i} and {j} collided");
            }
        }
    }

    #[test]
    fn derivation_is_asymmetric() {
        // If it were symmetric, a seed and a site label would be
        // interchangeable. Nothing would break today; a reader would have to
        // check that nothing did, which is a cost paid on every reading.
        for (a, b) in [(1u64, 2u64), (0, 1), (u64::MAX, 7), (0x1234_5678, 0x9ABC_DEF0)] {
            assert_ne!(derive(a, b), derive(b, a), "derive({a}, {b}) is symmetric");
        }
    }

    #[test]
    fn a_stream_reproduces_itself_from_its_seed() {
        let mut a = Stream::from_seed(0xC0FF_EE00_1234_5678);
        let mut b = Stream::from_seed(0xC0FF_EE00_1234_5678);
        for i in 0..256 {
            assert_eq!(a.next_u64(), b.next_u64(), "diverged at draw {i}");
        }
    }

    #[test]
    fn the_all_zero_state_is_not_a_fixed_point() {
        // The guard xorshift needed and this generator does not. Built by hand,
        // because `from_seed` cannot produce this state and the claim in the
        // type's documentation is about the state, not about the seeding.
        let mut stream = Stream { a: 0, b: 0, c: 0, counter: 0, origin: 0 };
        assert_eq!(stream.next_u64(), 0, "the first output of the zero state is zero");
        let mut nonzero = 0;
        for _ in 0..16 {
            if stream.next_u64() != 0 {
                nonzero += 1;
            }
        }
        assert!(nonzero > 0, "the all-zero state is a fixed point after all");
    }

    #[test]
    fn splitting_does_not_consume_the_parent() {
        // The property this module is shaped around: a stream created later
        // cannot move a stream created earlier. If `split` drew from the parent
        // this would fail at the first draw after the split.
        let parent = Stream::from_seed(0xABCD_1234_ABCD_1234);

        let mut untouched = parent.clone();
        let mut after_splitting = parent.clone();
        let mut child = after_splitting.split(0x1111);
        for _ in 0..64 {
            let _ = child.next_u64();
        }

        for i in 0..256 {
            assert_eq!(
                untouched.next_u64(),
                after_splitting.next_u64(),
                "splitting moved the parent, at draw {i}"
            );
        }
    }

    #[test]
    fn the_same_identity_always_names_the_same_child() {
        let parent = Stream::from_seed(7);
        let mut first = parent.split(42);
        let mut second = parent.split(42);
        for _ in 0..64 {
            assert_eq!(first.next_u64(), second.next_u64());
        }
    }

    #[test]
    fn children_at_different_identities_never_share_a_prefix() {
        // "Never produce the same first K outputs", checked as prefixes rather
        // than as first values: two streams agreeing on one value and then
        // diverging is not the failure this is looking for. Two streams that are
        // one stream is.
        const K: usize = 8;
        const N: usize = 64;
        let parent = Stream::from_seed(0x0F0F_0F0F_0F0F_0F0F);

        let mut prefixes = [[0u64; K]; N];
        for (identity, prefix) in prefixes.iter_mut().enumerate() {
            let mut child = parent.split(identity as u64);
            for slot in prefix.iter_mut() {
                *slot = child.next_u64();
            }
        }
        for i in 0..N {
            for j in (i + 1)..N {
                assert_ne!(prefixes[i], prefixes[j], "children {i} and {j} share {K} outputs");
            }
        }
    }

    #[test]
    fn cross_stream_correlation_stays_inside_the_band() {
        // The test this task existed to make writable. Every pair of streams
        // split from one parent is compared bit for bit over its low 32 bits,
        // and the count of agreements must sit inside five standard deviations
        // of the binomial it would be if the two streams were independent.
        //
        // The band comes from `agreement_band` and is derived from `DRAWS`, not
        // tuned until green. Read that function before changing either constant.
        let parent = Stream::from_seed(0xFEED_FACE_CAFE_BABE);
        let mean = agreement_mean(DRAWS);
        let band = agreement_band(DRAWS);

        for i in 0..STREAMS {
            for j in (i + 1)..STREAMS {
                let agreed = agreement(parent.split(i as u64), parent.split(j as u64));
                let distance = agreed.abs_diff(mean);
                assert!(
                    distance <= band,
                    "streams {i} and {j} agreed on {agreed} of {} bits, \
                     which is {distance} from {mean} and the band is {band}",
                    32 * DRAWS
                );
            }
        }
    }

    #[test]
    fn the_correlation_band_can_actually_fail() {
        // A statistic nobody has watched reject anything is a statistic that
        // cannot. Two copies of one stream agree on every bit, which is the
        // maximum the count can take and is far outside the band.
        let seed = 0xFEED_FACE_CAFE_BABE;
        let agreed = agreement(Stream::from_seed(seed), Stream::from_seed(seed));
        assert_eq!(agreed, 32 * DRAWS, "two copies of one stream must agree on every bit");
        assert!(
            agreed.abs_diff(agreement_mean(DRAWS)) > agreement_band(DRAWS),
            "the band admits two identical streams, so it proves nothing"
        );
    }

    #[test]
    fn no_child_is_another_child_at_a_different_phase() {
        // The failure the bit-agreement statistic is blind to: stream j equal to
        // stream i shifted along by some number of draws agrees at exactly
        // chance and passes. Checked directly instead — no child's first output
        // appears anywhere in the opening of any other child's.
        let parent = Stream::from_seed(0x1357_9BDF_2468_ACE0);

        let mut firsts = [0u64; STREAMS];
        for (identity, slot) in firsts.iter_mut().enumerate() {
            *slot = parent.split(identity as u64).next_u64();
        }

        for i in 0..STREAMS {
            let mut other = parent.split(i as u64);
            for step in 0..DRAWS {
                let value = other.next_u64();
                for (j, first) in firsts.iter().enumerate() {
                    if i == j && step == 0 {
                        continue;
                    }
                    assert_ne!(
                        value, *first,
                        "child {i} produced child {j}'s first output at step {step}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_stream_does_not_cycle_inside_a_window_far_shorter_than_its_floor() {
        // What this detects: a stream that is constant, or that has a period
        // below the window. What it does not detect: anything about the real
        // period, which is at least 2^64 by construction and is not observable
        // from here. It is here because a short cycle is what a mistranscribed
        // shift constant produces, and it would otherwise be invisible.
        const WINDOW: usize = 2048;
        let mut stream = Stream::from_seed(0x2222_3333_4444_5555);
        let mut seen = [0u64; WINDOW];
        for slot in seen.iter_mut() {
            *slot = stream.next_u64();
        }
        for i in 0..WINDOW {
            for j in (i + 1)..WINDOW {
                assert_ne!(seen[i], seen[j], "the stream repeated a value at {i} and {j}");
            }
        }
    }

    #[test]
    fn a_label_is_a_stable_identity_and_two_labels_differ() {
        assert_eq!(label("ring.publish"), label("ring.publish"));
        assert_ne!(label("ring.publish"), label("ring.publisi"));
        // The empty label is the FNV offset basis, which is a legitimate
        // identity and not a special case. Stated so that nobody adds a guard.
        assert_eq!(label(""), 0xcbf2_9ce4_8422_2325);
    }
}
