// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Which effect is downgraded first when a frame will not fit, written once, as
//! a table, in the order it is applied.
//!
//! Section 10 of `docs/design/ring-scene-boot.html` asks for one sentence to be
//! true of this: *the compositor downgrades effects in a fixed, deterministic
//! priority order*. The word doing the work is **fixed**. A priority order that
//! is deterministic but unwritten — the order a `for` loop happened to visit in,
//! the order a `match` happened to be typed in, the order a stable sort happened
//! to preserve — is reproducible on the machine it was observed on and is not a
//! decision anybody made. It is also invisible in review: nothing in a diff says
//! *this is the order*, so nothing in a later diff says *this changed it*.
//!
//! So the order is [`Criterion::ORDER`], the whole of the ordering is
//! [`rank`], and every other function here is a reader of that one. There is no
//! `if` in this file that decides which effect goes first and no `match` arm
//! whose position means anything: the arms the macro emits answer *what does
//! this criterion measure*, which is a question with no order in it, and the
//! sequence they are written in is the sequence of the declaration they are
//! emitted from. Reordering the table is the only edit that reorders the policy,
//! which is what *the order is data* has to mean if it is to mean anything.
//!
//! # The order, and the argument for each step of it
//!
//! Four criteria, read in sequence, the first that separates two effects
//! deciding between them. In words, and the variants below carry the rest:
//!
//! 1. **Spend the invisible changes first.** A fallback that keeps the effect on
//!    the screen goes before one that removes it.
//! 2. **Then buy the most per node touched.** Among equally invisible choices,
//!    the largest saving, so the frame is repaired by changing the fewest things.
//! 3. **Then lose the smallest fraction of an effect.** Among equal savings, the
//!    larger estimate, because the same absolute saving is a smaller proportional
//!    loss taken out of a bigger effect.
//! 4. **Then the node identifier**, which settles everything left and is the
//!    reason there is nothing left.
//!
//! The first three are judgements and are argued on their own variants,
//! including what would reverse each. The fourth is not a judgement; it is the
//! terminator, and its presence at the end of the table is a compile-time
//! assertion further down rather than a habit.
//!
//! # This is not RFC 0080's ladder, and the distance between them is one frame
//!
//! RFC 0080 picks a **rasteriser**, once, when a compositor starts, out of four
//! named rungs, and never promotes. This picks an **effect**, inside a frame
//! that is already late, and the next frame starts with every effect back at
//! full fidelity. The two share the word *fallback* and nothing else, and RFC
//! 0080 says why the confusion is worth guarding against rather than merely
//! noting: a system that answered a missed frame by changing renderers would
//! have chosen the one response guaranteed to miss the next frame too — a new
//! rasteriser has to be started, and starting it costs a frame the compositor
//! has already established it does not have.
//!
//! Here that separation is not a paragraph. `f-scene` depends on `f-abi` and on
//! nothing else; `Rung` is in `f-interface`, which this crate does not take and
//! could not take without a dependency row somebody has to write. **There is no
//! expression in this file that can name a rung**, so *a missed frame never
//! changes the rung* is not a convention this module keeps — it is a sentence
//! about a type this module cannot reach. `E3-B07g` asserts it end to end, where
//! a compositor holds both; what it is asserting against is this absence.
//!
//! # There is no seed here, and that is the point
//!
//! The exit this file is accepted on says *one overload from one seed produces
//! the same downgrades in the same sequence on both architectures*. The seed in
//! that sentence belongs to the **overload** and never to the policy. A scene
//! is generated from a seed, the frame it implies is late, and the question is
//! whether the downgrades that follow are a function of that scene alone.
//!
//! So nothing here draws. Every function in this module is a pure function of
//! its arguments, this crate takes no `f_env::Env`, and a function whose answer
//! depends only on its argument is more deterministic than one that draws: it is
//! reproducible from the argument alone, with no substrate to agree about and no
//! draw order to preserve. A policy that took a seed would be *choosing* among
//! downgrades rather than *determining* them, which is the opposite of what
//! section 10 asks for, and it would make a replay depend on how many times
//! something upstream had drawn.
//!
//! The tests below hold the seed instead, and generate their overload from it
//! with integer mixing — the corpus is a function of one `u64`, and the sequence
//! the policy produces from it is compared against a digest written into this
//! file. `cargo xtask test` runs the workspace on x86-64 and on AArch64; one
//! literal asserted by both jobs is the whole of *on both architectures*, and it
//! is a literal rather than a recomputation because a value compared against
//! itself agrees on any machine.
//!
//! # Why the sort's tie-breaking does not matter, and why that is provable
//!
//! [`order`] sorts with `sort_unstable_by_key`, which is core's and allocates
//! nothing — and an unstable sort is normally the wrong instrument for a
//! reproducible sequence, because equal keys come out in an order the algorithm
//! chose. It is the right one here because **[`rank`] is injective on
//! [`Effect`]**, and that is arithmetic rather than hope.
//!
//! An [`Effect`] is three numbers: a node, an estimate and a saving. A rank
//! packs four criteria at [`FIELD_BITS`] each, and three of those criteria are
//! the three numbers at their full width — the fourth,
//! [`Criterion::Fidelity`], is a function of two of them and adds no
//! information. So a rank determines the effect that produced it, distinct
//! effects have distinct ranks, no two keys are ever equal, and the sorted
//! permutation is unique. The algorithm's behaviour on ties is unobservable
//! because there are none.
//!
//! One caveat, stated rather than discovered: a caller may pass the same
//! declaration twice. [`downgrade`] expects at most one declaration per node and
//! says so, but if it gets two, the two are *equal values* — equal rank means
//! equal effect, by the paragraph above — so the sequence of downgrades is the
//! same sequence whichever way the sort put them. The precondition buys clarity;
//! it is not load-bearing.
//!
//! # When there is nothing left to give back
//!
//! The policy cannot always save the frame. If every effect degraded still does
//! not close the gap, that is [`Downgrades::Short`]: a named outcome carrying how
//! much is still owed, rather than a panic, a wildcard, or a silent decision to
//! degrade something nobody declared degradable. What the compositor does then —
//! render the degraded frame and miss, and record the miss — is `E3-B07d`'s, and
//! this module's contribution is that the case has a name at all, because an
//! outcome with no name is the one nobody writes the handler for.
//!
//! # No clock, no randomness, no binary floating point, no allocator
//!
//! Costs are integers with their scale in their names, per RFC 0004: the two
//! architectures do not agree on binary fractions, and a frame that degraded
//! differently on one of them would be this whole thesis failing quietly. The
//! rank is a `u128`, whose arithmetic is exact and identical on both targets,
//! which is precisely what a float is not. No collection, so no iteration order
//! to seed; the caller owns the slice and this module sorts it in place.

use core::num::{NonZeroU32, NonZeroUsize};

use crate::effect::{Effect, Instead};

/// How many bits of a [`Rank`] one criterion occupies.
///
/// Every criterion reads a `u32` and every field is stored at its full width,
/// so a reading can never overflow the space it is packed into — there is no
/// mask, no truncation and no per-criterion width to keep in step with a value.
/// That is what makes [`rank`]'s injectivity the arithmetic it is claimed to be
/// above rather than a property of the numbers that happen to be declared today.
/// Unit: bits.
pub const FIELD_BITS: u32 = u32::BITS;

/// Which way round a criterion's reading runs.
///
/// Declared per criterion on the table's own line, because *larger first* and
/// *smaller first* are as much part of the order as the sequence is, and a sense
/// kept somewhere else is a second place the order is written. The normalisation
/// below is the only code that knows what a sense does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sense {
    /// The larger reading is downgraded first.
    Larger,
    /// The smaller reading is downgraded first.
    Smaller,
}

impl Sense {
    /// The reading as it is packed, so that smaller always sorts earlier.
    ///
    /// A bitwise complement rather than `u32::MAX - reading`, which is the same
    /// map: the complement says *reverse this field* without an arithmetic
    /// expression for a reader to check for an off-by-one, and it cannot
    /// underflow at either end of the range.
    /// Unit: the criterion's own, complemented — a packed field and not a
    /// quantity anybody should read back.
    const fn packed(self, reading: u32) -> u32 {
        match self {
            Self::Larger => !reading,
            Self::Smaller => reading,
        }
    }
}

/// Whether two distinct effects in one frame can read the same value here.
///
/// The table's last line must be a criterion where they cannot, or the order is
/// not total and the sequence of downgrades is whatever the sort felt like. That
/// requirement is a `const` assertion below rather than a test, because a table
/// that cannot settle its own ties should not produce an artefact at all — and
/// because a test is exactly what a later criterion appended after the
/// terminator would walk straight past.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Collision {
    /// Two effects in one frame can read the same value here.
    Possible,
    /// No two distinct effects in one frame read the same value here, so this
    /// criterion settles every pair on its own.
    Never,
}

/// The order, written once so that it cannot be written twice.
///
/// `kind.rs` argues at length why a macro is worth its indirection in a tree
/// that mostly refuses them, and the argument is the same one here with a
/// sharper edge: the four sequences that would otherwise have to agree are the
/// variants, the table, the priority index and the readings, and this project
/// has twice watched a variant be added to an enum and left out of an array
/// every loop iterates. The repair both times was not a better test. It was the
/// removal of the second place to write the thing.
///
/// What a line has to supply is: the variant, the word a log prints, which way
/// the reading runs, whether two effects can read the same value, and the
/// reading itself. A criterion missing any of the five does not parse, so each
/// of those questions is asked when the criterion is proposed rather than when
/// something downstream trips over the absence of an answer.
macro_rules! criteria {
    (
        $(
            $(#[$about:meta])*
            $variant:ident, $spelling:literal, $sense:ident, $collision:ident, $reading:expr,
        )*
    ) => {
        /// One ranked question asked about an effect, in the order it is asked.
        ///
        /// **The declaration order is the priority order.** The variant's
        /// discriminant is its position in the list, which is its position in
        /// [`ORDER`](Self::ORDER), which is the bit range it occupies in a
        /// [`Rank`]: one list read three ways and no second sequence to keep in
        /// step with the first. Moving a variant moves the policy, and nothing
        /// else does.
        ///
        /// Not `#[non_exhaustive]`, for `interface/src/node.rs`'s reason about
        /// `Role` and `kind.rs`'s about `Kind`: that attribute forces every
        /// consumer to write a wildcard arm, and a wildcard arm is where a
        /// criterion nobody understood goes to be ignored. A consumer recording
        /// why a downgrade was chosen — `E3-B07d` — should be stopped by its
        /// compiler on the day a fifth criterion exists.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Criterion {
            $($(#[$about])* $variant,)*
        }

        impl Criterion {
            /// How many criteria the order has.
            ///
            /// Counted from the list rather than written as a literal, so it
            /// cannot disagree with what it counts.
            /// Unit: criteria.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// The order, most significant first.
            ///
            /// Emitted from the same list as the enum, so it holds every
            /// variant the enum has — not because a test checks it, since a
            /// loop over a list cannot see what the list omits, but because
            /// there is no way to write a variant this array does not get.
            /// This is *the* table: the sentence *the downgrade priority order
            /// is data, and it is fixed* is a statement about this constant and
            /// about nothing else in the file.
            pub const ORDER: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// This criterion's place in [`ORDER`](Self::ORDER).
            ///
            /// The enum's own discriminant, which is the position of the line
            /// that declared it. Zero is asked first.
            /// Unit: none — a position, not a quantity.
            #[must_use]
            pub const fn priority(self) -> usize {
                self as usize
            }

            /// The word a log or a trace prints.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }

            /// Which way round this criterion's reading runs.
            #[must_use]
            pub const fn sense(self) -> Sense {
                match self {
                    $(Self::$variant => Sense::$sense,)*
                }
            }

            /// Whether two distinct effects in one frame can read the same
            /// value here.
            #[must_use]
            pub const fn collision(self) -> Collision {
                match self {
                    $(Self::$variant => Collision::$collision,)*
                }
            }

            /// What this criterion reads off an effect, before the sense is
            /// applied.
            ///
            /// Public because `E3-B07d` records *why* a frame chose what it
            /// chose, and a record that named the criterion without its value
            /// would be a record nobody can check against the declaration.
            /// Unit: whatever the criterion measures — microseconds scaled by
            /// 100 for the two that are durations, a node identifier for the
            /// terminator, and 0 or 1 for the one that is a fidelity.
            #[must_use]
            pub fn reading(self, effect: &Effect) -> u32 {
                match self {
                    $(Self::$variant => ($reading)(effect),)*
                }
            }
        }
    };
}

criteria! {
    /// Does the fallback keep the effect on the screen?
    ///
    /// Reads 0 when the declared fallback is [`Instead::Reduced`] and 1 when it
    /// is [`Instead::Skip`], so *smaller first* is *keep what can be kept*.
    ///
    /// First, because it is the only step of this order that section 10 already
    /// argues for. That section's justification for degrading at all is that
    /// *dropping a blur to a cheaper approximation for one frame is, in
    /// practice, invisible* — a claim about **reduction**, and not a claim about
    /// removal. Removing an effect is a change to the picture that a person can
    /// see. So the order spends the changes the design document has an argument
    /// for before it spends the ones it does not, and reaches for a removal only
    /// when every reduction together was not enough.
    ///
    /// The cost is real and is not hidden: reductions give back less than
    /// removals, so this criterion makes the policy touch more nodes than it
    /// otherwise would. That is the trade, and it is the same trade section 10
    /// makes one layer up — more work, less visible change.
    ///
    /// *What would reverse this:* a declaration that said how much of the frame
    /// an effect covers. The reduction of a full-screen blur is plainly more
    /// visible than the removal of a four-pixel shadow, and this policy cannot
    /// know that, because [`Instead`] deliberately carries no recipe and no node
    /// carries an area. Supplying one is a new field in `abi/src/scene.rs`
    /// reviewed as an ABI change — at which point coverage becomes the first
    /// line of this table and fidelity moves below it.
    Fidelity, "fidelity", Smaller, Possible,
        |it: &Effect| match it.fallback().instead() { Instead::Reduced => 0, Instead::Skip => 1 },

    /// What downgrading this node gives back.
    ///
    /// Larger first, so the deficit is closed by changing the fewest things. Two
    /// reasons, and they point the same way for once: every node degraded is a
    /// node whose picture changed, so fewer of them is less change; and the walk
    /// terminates sooner, which matters because this walk happens inside a frame
    /// that has already been established to be short of time.
    ///
    /// The counter-argument is worth stating because it is the obvious one: the
    /// effect with the largest saving is often the most prominent effect on the
    /// screen — the big background blur — and it is therefore the one a person
    /// is most likely to notice losing. That objection is answered above rather
    /// than here, by [`Criterion::Fidelity`] ranking above this: the prominent
    /// blur is reached only if its declared fallback keeps it on the screen, or
    /// if every reduction in the frame has already been spent.
    ///
    /// *What would reverse this:* a measured corpus in which closing a deficit
    /// with one large downgrade is reported worse than closing it with several
    /// small ones. That is a claim about perception, it needs the rig
    /// `E3-P01` builds, and until it exists *fewest things changed* is the
    /// defensible default rather than the proven one.
    Saving, "saving", Larger, Possible, |it: &Effect| it.saving_us_x100().get(),

    /// What the effect was expected to cost before it was degraded.
    ///
    /// Larger first, and this is the least obvious line in the table, so it is
    /// argued rather than asserted. Two effects give back the same number of
    /// microseconds. One cost 900 and one cost 200. Degrading the second takes
    /// most of that effect away; degrading the first takes a fraction of it. The
    /// same absolute saving is a smaller proportional loss out of a bigger
    /// effect, so the bigger effect is the cheaper place to take it from.
    ///
    /// The honest label on this is that proportional loss is a **proxy** for
    /// visible change and not a measurement of it, chosen because it is
    /// computable from numbers the declaration already carries. The alternative
    /// that was considered and refused is ranking by the ratio of saving to
    /// estimate directly: that is a division, a division needs a fixed-point
    /// scale and a rounding convention, and both would be numbers invented in
    /// this file to decide a tie that the pair of whole numbers already decides.
    ///
    /// *What would reverse this:* the same corpus the line above wants. If
    /// proportional loss turns out not to track what people notice, this line is
    /// the first one to delete, because it is the only one of the three whose
    /// argument rests on a proxy.
    Estimate, "estimate", Larger, Possible, |it: &Effect| it.estimate_us_x100().get(),

    /// The node the declaration is about.
    ///
    /// Not a judgement. This is the terminator: a node identifier is unique
    /// within a frame, so this criterion settles every pair that reaches it, and
    /// the order is therefore total. Ascending rather than descending for no
    /// reason worth defending — what matters is that it is *stated*, because an
    /// order that is total by accident is an order somebody will later make
    /// partial by accident.
    ///
    /// Its position at the end of the table is checked at compile time below,
    /// along with its being the only line in the table that claims to settle
    /// anything. A criterion appended after it would leave pairs undecided and
    /// hand the sequence back to the sorting algorithm, which is the failure
    /// this whole file exists to make impossible.
    Node, "node", Smaller, Never, |it: &Effect| it.node(),
}

// The terminator is last, and it is the only one. Asserted here rather than in a
// test for the reason the file opens with: a table that cannot settle its own
// ties produces a sequence the sorting algorithm chose, and a build that can
// produce one should not link. A fifth criterion appended after `Node` fails
// this on the afternoon it is written; a criterion inserted *above* `Node` does
// not, and should not, because that is how this order is meant to grow.
const _: () = {
    let mut at = 0;
    let mut settling = 0;
    while at < Criterion::COUNT {
        if matches!(Criterion::ORDER[at].collision(), Collision::Never) {
            settling += 1;
            assert!(
                at == Criterion::COUNT - 1,
                "a criterion that settles every pair is not the last one asked, so the criteria \
                 after it can never decide anything"
            );
        }
        at += 1;
    }
    assert!(
        settling == 1,
        "the downgrade order must end in exactly one criterion no two effects can tie on, or the \
         sequence of downgrades is whichever one the sort produced"
    );
};

// A rank has to hold every criterion at its full width, and a `u128` holds four.
// This is a real ceiling and it is stated as one: a fifth criterion does not
// silently truncate a field, it fails to build, and the repair is a decision
// somebody makes in this file — narrow a field and say why it cannot overflow,
// or widen the rank and say what it is now made of.
const _: () = {
    assert!(
        Criterion::COUNT * FIELD_BITS as usize <= u128::BITS as usize,
        "the criteria no longer fit a u128 rank at full field width"
    );
};

/// How wide the packed part of a [`Rank`] is.
///
/// The high `u128::BITS - RANK_BITS` bits of a rank are always zero, which
/// [`decided_by`] has to subtract off when it turns a leading-zero count into a
/// position in [`Criterion::ORDER`]. Derived from the table rather than written
/// down, so it moves when the table does.
/// Unit: bits.
const RANK_BITS: u32 = FIELD_BITS * Criterion::COUNT as u32;

/// Where an effect stands in the downgrade order.
///
/// Every criterion's reading, sense applied, packed most significant first, so
/// that comparing two ranks *is* reading the table from the top and stopping at
/// the first disagreement. There is no comparator anywhere in this module and no
/// second procedure that walks the criteria in order — lexicographic comparison
/// of the packed integer is that walk, performed by the `Ord` the compiler
/// derives, and a lexicographic order cannot be written down wrongly the way a
/// hand-rolled `cmp` chain can.
///
/// Opaque on purpose: no accessor hands the bits back. A consumer that wants to
/// know *why* one effect came before another asks [`decided_by`], which answers
/// with a [`Criterion`] — a number carrying four packed fields is a thing
/// somebody would eventually decode, and a second decoder is a second opinion
/// about the order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank(u128);

/// Where this effect stands in the downgrade order.
///
/// Smaller is downgraded earlier. The whole of the ordering is here: the fold
/// walks [`Criterion::ORDER`] in order and shifts, so the table's sequence is
/// the significance of the fields, and there is nothing else in this file that
/// decides which effect goes first.
///
/// Injective, for the reason the module documentation gives in full: three of
/// the four criteria are the three numbers an [`Effect`] holds, each at its
/// full width, so two effects with one rank are one effect.
#[must_use]
pub fn rank(effect: &Effect) -> Rank {
    let mut bits: u128 = 0;
    for criterion in Criterion::ORDER {
        let field = criterion.sense().packed(criterion.reading(effect));
        bits = (bits << FIELD_BITS) | u128::from(field);
    }
    Rank(bits)
}

/// Which criterion put one of these two ahead of the other.
///
/// `None` when they rank identically, which by the injectivity above means they
/// are the same effect. Otherwise the highest-priority criterion on which the
/// two disagree — which is the criterion that decided, because every criterion
/// above it read the same on both.
///
/// Computed from the two ranks rather than by re-walking the table, and that is
/// the point rather than a micro-optimisation: a second walk would be a second
/// reading of the order, and two readings of one table is how the two halves of
/// a policy come to disagree. The exclusive-or of two ranks has its highest set
/// bit in the first field that differs, so this function cannot contradict
/// [`rank`]; it is [`rank`], read backwards.
#[must_use]
pub fn decided_by(one: &Effect, other: &Effect) -> Option<Criterion> {
    let differing = rank(one).0 ^ rank(other).0;
    if differing == 0 {
        return None;
    }
    // `differing` is below `2^RANK_BITS`, so it has at least `u128::BITS -
    // RANK_BITS` leading zeros and the subtraction cannot underflow. Dividing
    // the remainder by the field width is the field's position from the top,
    // which is its index in `ORDER`, because that is the order the fold packed
    // them in.
    let from_the_top = differing.leading_zeros() - (u128::BITS - RANK_BITS);
    Some(Criterion::ORDER[(from_the_top / FIELD_BITS) as usize])
}

/// Put these effects into downgrade order, earliest first.
///
/// Sorted in place, because this crate has no allocator and the caller owns the
/// frame's candidates already. `sort_unstable_by_key` is core's, allocates
/// nothing, and its tie-breaking is unobservable here: [`rank`] is injective, so
/// there are no ties for it to break. The module documentation carries that
/// argument in full, and it is the reason an unstable sort is the correct
/// instrument rather than a corner cut.
pub fn order(effects: &mut [Effect]) {
    effects.sort_unstable_by_key(rank);
}

/// How far past the frame's remaining budget the running estimate is.
///
/// A newtype and not a bare number, because the commonest thing to pass at this
/// call site by mistake is the *remaining budget* — the other number in the same
/// sentence, of the same type, in the same unit, meaning the opposite. A
/// compositor that passed one for the other would degrade hardest on the frames
/// with the most room.
///
/// Non-zero, so *degrade something to recover nothing* is not a call that can be
/// made. A frame that is inside its budget degrades nothing, and that is the
/// caller's early return rather than a walk that returns an empty answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Overrun(NonZeroU32);

impl Overrun {
    /// How far over the frame is, or `None` if it is not over.
    ///
    /// Unit of `over_budget_us_x100`: microseconds, scaled by 100 — the scale
    /// `effect.rs` declares costs and savings in, because this number is
    /// compared against their sum.
    #[must_use]
    pub const fn new(over_budget_us_x100: u32) -> Option<Self> {
        match NonZeroU32::new(over_budget_us_x100) {
            Some(over) => Some(Self(over)),
            None => None,
        }
    }

    /// How far over the frame is.
    /// Unit: microseconds, scaled by 100. Never zero.
    #[must_use]
    pub const fn over_budget_us_x100(self) -> NonZeroU32 {
        self.0
    }
}

/// What the policy chose for one late frame.
///
/// Two variants rather than a struct with a flag, so that *covered, and still
/// short by 400* and *short, having taken 0 nodes* are not values this type has
/// — they are absent rather than refused. `effect.rs` opens with the same
/// argument about two fields that must agree over one fact, and it would be a
/// poor neighbour that reintroduced the shape one file along.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[must_use]
pub enum Downgrades {
    /// The overrun was closed. The first `taken` effects of the ordered slice
    /// are the downgrades, in the sequence they were chosen.
    Covered {
        /// How many of the ordered slice to downgrade, from its front.
        ///
        /// The shortest prefix that closes the overrun: stopping one earlier
        /// would leave the frame late. Never zero, because an [`Overrun`] is
        /// never zero and no effect saves nothing.
        /// Unit: effects.
        taken: NonZeroUsize,
        /// How much the last downgrade gave back beyond what was owed.
        ///
        /// Zero when the prefix lands exactly on the overrun. It is reported
        /// rather than discarded because it is the measure of how coarse the
        /// last step was, which is the number that tells a later reader whether
        /// this policy is taking more picture than the deadline required.
        /// Unit: microseconds, scaled by 100. Always below the last downgrade's
        /// own saving.
        spare_us_x100: u32,
    },
    /// Every effect in the slice is downgraded and the frame is still late.
    ///
    /// The whole slice is the sequence. There is nothing further this policy can
    /// do: what remains is to render what is left and miss, and to record the
    /// miss, which is `E3-B07d`'s. The case has a name because an outcome with
    /// no name is the one nobody writes the handler for.
    Short {
        /// What is still owed after everything has been given back.
        /// Unit: microseconds, scaled by 100. Never zero — a frame that was
        /// closed is [`Downgrades::Covered`].
        short_by_us_x100: NonZeroU32,
    },
}

/// Choose the downgrades for a late frame.
///
/// Puts `effects` into downgrade order — [`order`], and therefore
/// [`Criterion::ORDER`], and therefore nothing else — and then takes from the
/// front until the overrun is closed. The sequence is the prefix, and it is the
/// same prefix for the same effects whatever order the caller handed them over
/// in, because the sort is total and injective.
///
/// Taking a prefix rather than searching for a best-fitting subset is a decision
/// and not an oversight. A subset sum would give back less picture for the same
/// microseconds; it would also be a search, inside a frame whose defining
/// property is that it has run out of time, with a cost that depends on the
/// scene rather than on the number of effects in it. A prefix walk is one pass,
/// its cost is the sort's, and what it gives up is stated here rather than
/// discovered.
///
/// # Panics
///
/// Never. Every arithmetic step below is saturating or is guarded by the type of
/// its operand.
///
/// The caller is expected to pass at most one declaration per node. Two
/// declarations of one node do not break anything — equal ranks mean equal
/// effects, so the sequence is unchanged — but they mean the frame recovered one
/// saving twice, which is a defect above this line.
pub fn downgrade(effects: &mut [Effect], overrun: Overrun) -> Downgrades {
    order(effects);
    let mut owed = overrun.over_budget_us_x100();
    for (at, effect) in effects.iter().enumerate() {
        let saving = effect.saving_us_x100().get();
        // `saturating_sub` and then `NonZeroU32::new` is one test doing two
        // jobs: the saturation is the *did this one close it* question and the
        // non-zero type is the answer carried onward, so there is no comparison
        // here whose result a later line has to be trusted to remember.
        match NonZeroU32::new(owed.get().saturating_sub(saving)) {
            None => {
                return Downgrades::Covered {
                    taken: NonZeroUsize::MIN.saturating_add(at),
                    spare_us_x100: saving - owed.get(),
                };
            }
            Some(still_owed) => owed = still_owed,
        }
    }
    Downgrades::Short { short_by_us_x100: owed }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effect::Declared;
    use crate::kind::{Change, Created, Kind};
    use f_abi::NO_DEADLINE;
    use f_abi::scene::{CreateNode, Delta, Entry, NO_NODE, op};

    /// The overload every test below is run against.
    ///
    /// One `u64`, and the corpus is a pure function of it. This is the seed the
    /// exit sentence means: it belongs to the scene, not to the policy, and
    /// nothing in the module above can see it.
    const SEED: u64 = 0x5EED_B07B_0000_0001;

    /// How many effects the overloaded frame declares.
    /// Unit: effects.
    const OVERLOAD: usize = 64;

    /// How many distinct cost estimates the corpus draws from.
    ///
    /// Six, which makes twenty-one distinct (estimate, saving) pairs, which is
    /// fewer than [`OVERLOAD`]. That is a pigeonhole and not a hope: with more
    /// effects than shapes, two of them must agree on both durations, so the
    /// terminator is guaranteed to be the criterion that decides at least one
    /// adjacent pair. A corpus that never reaches the last line of the table
    /// would be a corpus that cannot observe the last line of the table.
    /// Unit: shapes.
    const SHAPES: u64 = 6;

    /// The unit the corpus's durations step in.
    /// Unit: microseconds, scaled by 100 — one step is one microsecond.
    const STEP: u32 = 100;

    /// How far over budget the overloaded frame is, in the tests that close it.
    ///
    /// Chosen to need a prefix of several effects rather than one or all: a
    /// deficit one downgrade closes would not observe the sequence at all, and
    /// one nothing closes would not observe [`Downgrades::Covered`].
    /// Unit: microseconds, scaled by 100.
    const OVER_BUDGET: u32 = 4_000;

    /// A deadline far enough from zero to be unmistakably a deadline.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// SplitMix64, and the reason it is written out here rather than drawn from
    /// an `Env`.
    ///
    /// This crate takes no `f_env::Env` and should not grow one for a test: a
    /// corpus that is a pure function of a `u64` is reproducible from that
    /// `u64` alone, with no substrate to agree about and no draw order for a
    /// later edit to disturb. Every operation is a wrapping integer operation,
    /// so the sequence is identical on x86-64 and on AArch64 — which is what
    /// makes the digest below a statement about both architectures rather than
    /// about the one that happened to run first.
    const fn mixed(state: u64) -> u64 {
        let mut z = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// The creation token a create delta for `node` produces.
    ///
    /// Built through `Change::of` rather than by hand, because there is no by
    /// hand: `Created` has no public constructor. The envelope comes from
    /// `op::carries_deadline` rather than from a literal, so a change to which
    /// opcodes are scheduled reaches this file from the list that states it.
    fn created(node: u32) -> Created {
        let body = Entry::CreateNode(CreateNode {
            node,
            parent: NO_NODE,
            before: NO_NODE,
            kind: Kind::Effect.wire(),
        });
        let scheduled = op::carries_deadline(body.opcode()) == Some(true);
        let delta = Delta {
            user_data: 0,
            class: 0,
            deadline: if scheduled { SCHEDULED_AT } else { NO_DEADLINE },
            payload_offset: 0,
            flags: 0,
            body,
        };
        match Change::of(&delta) {
            Ok(Change::Created(token)) => token,
            other => panic!("a create delta did not create: {other:?}"),
        }
    }

    /// One believed declaration.
    fn effect(node: u32, estimate_us_x100: u32, saving_us_x100: u32) -> Effect {
        Effect::declared(&created(node), Declared { estimate_us_x100, saving_us_x100 })
            .expect("the corpus declares only whole declarations inside the range")
    }

    /// An overloaded frame, generated from one seed.
    ///
    /// Node identifiers are `1..=OVERLOAD`, so the caller's precondition — at
    /// most one declaration per node — holds by construction. Savings run from
    /// one step up to the whole estimate, so the corpus contains both kinds of
    /// fallback: the ones that save everything are `Instead::Skip` and the rest
    /// are `Instead::Reduced`.
    fn overload(seed: u64) -> [Effect; OVERLOAD] {
        let mut state = seed;
        core::array::from_fn(|at| {
            state = mixed(state);
            let shape = u32::try_from(state % SHAPES).expect("a remainder below six fits a u32");
            state = mixed(state);
            let cut = u32::try_from(state % u64::from(shape + 1)).expect("a remainder below six");
            let node = u32::try_from(at).expect("sixty-four fits a u32") + 1;
            effect(node, (shape + 1) * STEP, (cut + 1) * STEP)
        })
    }

    /// The same effects, handed over in a different order.
    ///
    /// Fisher-Yates from the same mixer. The point is not the shuffle; it is
    /// that the policy's answer must not depend on the order the arena happened
    /// to walk the scene in, and the only way to observe that is to walk it
    /// twice differently.
    fn shuffled(effects: &[Effect; OVERLOAD], seed: u64) -> [Effect; OVERLOAD] {
        let mut out = *effects;
        let mut state = seed;
        let mut at = OVERLOAD;
        while at > 1 {
            at -= 1;
            state = mixed(state);
            let span = u64::try_from(at).expect("sixty-four fits a u64") + 1;
            let pick = usize::try_from(state % span).expect("a remainder below sixty-four");
            out.swap(at, pick);
        }
        out
    }

    /// A digest of a sequence of downgrades, over everything that makes one.
    ///
    /// The node, the two durations and what the renderer is being asked to do,
    /// folded through the same integer mixer. It is wrapping integer arithmetic
    /// over values that are already integers, so the digest of one sequence is
    /// one number on every machine this tree builds for.
    fn digest(sequence: &[Effect]) -> u64 {
        let mut acc = 0;
        for one in sequence {
            acc = mixed(acc ^ u64::from(one.node()));
            acc = mixed(acc ^ u64::from(one.estimate_us_x100().get()));
            acc = mixed(acc ^ u64::from(one.saving_us_x100().get()));
            acc = mixed(
                acc ^ match one.fallback().instead() {
                    Instead::Skip => 1,
                    Instead::Reduced => 2,
                },
            );
        }
        acc
    }

    #[test]
    fn the_sequence_is_the_table_read_from_the_top() {
        // The exit's first clause. Not *the order is what this test says it is*
        // — that would be a second copy of the order, and the second copy is the
        // defect. What is asserted is that the produced sequence obeys
        // `Criterion::ORDER` read through `sense()`, for every adjacent pair,
        // with the table supplying both the sequence of questions and which way
        // each one runs. A criterion added, moved, or given the other sense is
        // covered by this loop on the day it is written, without the loop being
        // touched.
        let mut effects = overload(SEED);
        order(&mut effects);

        let mut decided = [false; Criterion::COUNT];
        for pair in effects.windows(2) {
            let (earlier, later) = (&pair[0], &pair[1]);
            let criterion = decided_by(earlier, later)
                .expect("the order is total, so no two distinct effects tie");
            decided[criterion.priority()] = true;

            // Every criterion above the deciding one read the same on both, and
            // the deciding one runs the way its own line says it does.
            for above in &Criterion::ORDER[..criterion.priority()] {
                assert_eq!(
                    above.reading(earlier),
                    above.reading(later),
                    "{} decided a pair that {} had already separated",
                    criterion.name(),
                    above.name()
                );
            }
            match criterion.sense() {
                Sense::Larger => assert!(
                    criterion.reading(earlier) > criterion.reading(later),
                    "{}: the smaller reading was downgraded first",
                    criterion.name()
                ),
                Sense::Smaller => assert!(
                    criterion.reading(earlier) < criterion.reading(later),
                    "{}: the larger reading was downgraded first",
                    criterion.name()
                ),
            }
        }

        // Every line of the table decided at least one pair. A criterion that
        // never decides anything is a line in the order that does not order
        // anything, and this is the assertion that refuses to let one sit there
        // looking load-bearing.
        for criterion in Criterion::ORDER {
            assert!(
                decided[criterion.priority()],
                "{} never decided a pair in this corpus, so nothing here observes it",
                criterion.name()
            );
        }
    }

    #[test]
    fn two_effects_with_one_rank_are_one_effect() {
        // The property the unstable sort rests on, over the corpus. It is an
        // argument about widths rather than about this corpus — three of the
        // four criteria are the three numbers an `Effect` holds, each packed at
        // its full width — and this test is what would fail first if a later
        // criterion narrowed one of them.
        let effects = overload(SEED);
        for (at, one) in effects.iter().enumerate() {
            for other in &effects[at + 1..] {
                assert_eq!(
                    rank(one) == rank(other),
                    one == other,
                    "two effects share a rank without being the same effect"
                );
                assert_eq!(decided_by(one, other).is_none(), one == other);
            }
        }
        // And the degenerate direction, which no corpus of distinct nodes can
        // reach: an effect ranks equal to itself and nothing decides between
        // them.
        assert_eq!(decided_by(&effects[0], &effects[0]), None);
    }

    #[test]
    fn one_overload_from_one_seed_is_one_sequence() {
        // The exit's second clause, in the only form a test on one machine can
        // honestly take: the answer is a function of the declarations and of
        // nothing else — not of the order they arrived in, not of a clock, not
        // of a draw — and the sequence it produces is a literal both
        // architectures' jobs assert. `cargo xtask test` runs this on x86-64 and
        // on AArch64; the digest below is the same number in both.
        let overrun = Overrun::new(OVER_BUDGET).expect("the frame is over budget");

        let mut straight = overload(SEED);
        let chosen = downgrade(&mut straight, overrun);
        let Downgrades::Covered { taken, .. } = chosen else {
            panic!("this corpus has enough savings in it to close this overrun: {chosen:?}");
        };

        // Handed over in three different orders, answered the same way three
        // times. This is the clause about arrival order, which is the one thing
        // a caller can vary without changing the frame.
        for disturbance in [1, 2, 3] {
            let mut jumbled = shuffled(&overload(SEED), SEED ^ disturbance);
            assert_eq!(downgrade(&mut jumbled, overrun), chosen);
            assert_eq!(jumbled, straight, "the same effects sorted two different ways");
        }

        assert_eq!(
            digest(&straight[..taken.get()]),
            0x84A8_FABA_A2BB_208F,
            "the downgrades this seed produces are not the ones recorded here"
        );
    }

    #[test]
    fn the_prefix_is_the_shortest_one_that_closes_the_frame() {
        // What `Covered` promises: the overrun is closed, and dropping the last
        // downgrade would reopen it. A policy that took one node more than it
        // needed would be taking picture the deadline did not ask for, and
        // nothing above this line would have said so.
        let overrun = Overrun::new(OVER_BUDGET).expect("the frame is over budget");
        let mut effects = overload(SEED);
        let Downgrades::Covered { taken, spare_us_x100 } = downgrade(&mut effects, overrun) else {
            panic!("this corpus has enough savings in it to close this overrun");
        };

        let recovered = |upto: usize| -> u32 {
            effects[..upto].iter().map(|one| one.saving_us_x100().get()).sum()
        };
        assert!(recovered(taken.get()) >= OVER_BUDGET);
        assert!(recovered(taken.get() - 1) < OVER_BUDGET);
        assert_eq!(recovered(taken.get()) - OVER_BUDGET, spare_us_x100);
        assert!(spare_us_x100 < effects[taken.get() - 1].saving_us_x100().get());
    }

    #[test]
    fn a_frame_nothing_can_close_says_so_rather_than_pretending() {
        // Every effect degraded and the frame still late. The number carried out
        // is what is still owed, so a caller recording the miss records how far
        // it missed by rather than that it missed.
        let mut effects = overload(SEED);
        let available: u32 = effects.iter().map(|one| one.saving_us_x100().get()).sum();
        let hopeless = Overrun::new(available + STEP).expect("more than everything is over budget");
        assert_eq!(
            downgrade(&mut effects, hopeless),
            Downgrades::Short {
                short_by_us_x100: NonZeroU32::new(STEP).expect("one step is not nothing")
            }
        );

        // An empty frame is the same outcome by the same route: nothing to give
        // back, so everything is still owed. No special case in the walk, and
        // none needed.
        let overrun = Overrun::new(OVER_BUDGET).expect("the frame is over budget");
        assert_eq!(
            downgrade(&mut [], overrun),
            Downgrades::Short { short_by_us_x100: overrun.over_budget_us_x100() }
        );
    }

    #[test]
    fn a_frame_that_is_not_late_is_not_a_frame_this_policy_is_asked_about() {
        // Zero is not an overrun. The type refuses it, so *degrade something to
        // recover nothing* is not a call a caller can make and not a case the
        // walk above has to carry an arm for.
        assert_eq!(Overrun::new(0), None);
        assert!(Overrun::new(1).is_some());
    }

    #[test]
    fn a_reduction_is_spent_before_a_removal() {
        // The first line of the table, stated the way a reader would state it,
        // against a pair built for it: a removal that gives back far more is
        // still reached after a reduction that gives back far less. This is the
        // one place the order is deliberately *not* the greedy one, so it is the
        // one place worth pinning with values a reader can check by eye.
        let reduction = effect(1, 9_000, 100);
        let removal = effect(2, 200, 200);
        assert_eq!(removal.fallback().instead(), Instead::Skip);
        assert_eq!(reduction.fallback().instead(), Instead::Reduced);
        assert_eq!(decided_by(&reduction, &removal), Some(Criterion::Fidelity));
        assert!(rank(&reduction) < rank(&removal));

        let mut both = [removal, reduction];
        order(&mut both);
        assert_eq!(both, [reduction, removal]);
    }
}
