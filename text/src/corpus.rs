// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The script and direction corpus: which scripts this tree tests text against,
//! and what each one's absence would hide.
//!
//! # What this file is for
//!
//! Section 08 of `docs/design/ring-scene-boot.html` says text is the part of a
//! rendering system that is always underestimated, and the mechanism by which
//! it is underestimated is this: somebody chooses a test string, it is Latin
//! because that is what the keyboard produces, and every subsequent decision —
//! how a cluster is bounded, where a line may break, what a caret steps over,
//! whether a cache key is a string — is made against a script in which all four
//! answers are the same easy answer. The bugs are not found later. They are not
//! *expressible* later, because by then the interfaces have been shaped by a
//! script that never asked the question.
//!
//! So the corpus is chosen first, in front of the shaper (RFC 0082), in front
//! of the reordering pass (`E3-B03e`), in front of the breaker (`E3-B03f`), and
//! it is chosen by argument rather than by coverage. Every entry below carries
//! one sentence saying **what its absence would hide** — not what the script
//! is, which is the encyclopaedia's job, and not that it is important, which is
//! everybody's opinion. A script whose absence would hide nothing does not
//! belong here, and [`Script::Latin`] is in the corpus precisely because it is
//! the one entry for which that sentence has to be argued the other way round.
//!
//! # Why adding a script is a diff to one list and to nothing else
//!
//! This module is written the way `interface/src/node.rs` writes its
//! vocabulary, for the reason recorded there: two adversarial reviews found a
//! hand-written `ALL` array that a new enum variant simply did not appear in,
//! and then found the hole still open under the test written to close it,
//! because an exhaustive `match` demands an arm and not an arm that says
//! anything. A corpus is the same shape of object and fails the same way — with
//! one addition that makes it worse. A corpus entry that is in the enum and in
//! no test is not a loud failure; it is a script that silently *is not tested*,
//! which reads exactly like a script that passes.
//!
//! The answer is that there is no second place to write an entry. The
//! [`corpus!`] invocation below is the only list, and from each line it emits
//! the variant, [`Script::ALL`], [`Script::COUNT`], [`Script::index`],
//! [`Script::name`], [`Script::from_name`], [`Script::direction`],
//! [`Script::hazards`], [`Script::sample`], [`Script::ranges`],
//! [`Script::hides`], and — the part that matters — the compile-time assertions
//! that check the entry. Adding a script is one line. Removing one is one line.
//! There is no array to extend, no count to bump, no coverage table to teach,
//! and no test naming the entry, because no test names any entry.
//!
//! # Why the per-entry checks are `const` and not tests
//!
//! Because a test is something a later edit can walk past, and every failure
//! this corpus exists to prevent is a *quiet* one. The five properties an entry
//! must have are checked by `const _: () = assert!(…)` items emitted from the
//! line itself, so an entry that lacks them does not fail a test run — it fails
//! to compile, on the line that is wrong, with the entry named in the message:
//!
//! 1. Its sentence is a sentence ([`is_a_sentence`]), so the field cannot be
//!    filled with `""` or with a label.
//! 2. Every scalar of its sample lies inside a range it declared
//!    ([`sample_is_covered`]), so the sample cannot be Latin transliteration
//!    pasted in by somebody whose editor could not produce the script.
//! 3. Every range it declared is actually reached by the sample
//!    ([`every_range_is_reached`]), so a declared range is a claim the sample
//!    has to keep. This is what holds [`Script::Hangul`] together: it declares
//!    the jamo block *and* the precomposed block, so deleting either half of
//!    its sample is a compile error rather than a quietly weaker entry.
//! 4. Its ranges are narrow enough to mean something ([`RANGE_BUDGET`]), so the
//!    check in (2) cannot be satisfied by declaring the whole of Unicode.
//! 5. Its name is of the one shape a name may take ([`is_a_name`]). This one is
//!    not about names at all — it is about the tests below, and it is how the
//!    second clause of this task's exit survives contact with a reviewer. See
//!    [`is_a_name`] for the landmine it defuses.
//!
//! The corpus-level properties are `const` for the same reason and are stated
//! after the list: names distinct, sentences distinct, every [`Hazard`] and
//! every [`Direction`] exercised by some entry, and no two entries exercising
//! the same thing.
//!
//! What remains in `#[cfg(test)]` is the one thing const assertions cannot do
//! for themselves. A checker that always answers `true` makes every assertion
//! above it vacuous — that is the const-assert form of the empty match arm —
//! so each test below feeds a checker an input it must refuse and an input it
//! must accept. The tests are about the checkers. None of them mentions an
//! entry, which is why adding one does not touch them.
//!
//! # What is deliberately not here
//!
//! No expected output. Not a glyph, not an advance, not a break position. An
//! expected output needs a shaper, and RFC 0082 put the shaper behind the
//! licence boundary where this crate cannot link it; a rendered comparison
//! additionally needs `E3-B03j`, whose exit says in as many words that it is
//! blocked on an RFC resolving a contradiction between two parent lines. A
//! corpus that guessed at outputs today would be a corpus asserting what this
//! tree's first shaper happens to do, which is the fixture that makes every
//! later shaper wrong by definition. What is here is the *input* half, and it
//! is complete on its own terms: `E3-B03e`, `E3-B03f`, `E3-B03g` and `E3-B03j`
//! each iterate [`Script::ALL`] and bring their own expectations.
//!
//! **The first of those reasons is narrower than it reads, and RFC 0115 corrects
//! it here rather than leaving it to be reasoned from.** *An expected output
//! needs a shaper* is exactly right about a glyph and an advance and **false
//! about an embedding level**: a level is a function of the scalars and their
//! bidi class — paragraph direction, explicit formatting, bracket pairs — and
//! nothing behind the licence boundary is consulted to know one. So a reader
//! arriving from `E3-B03e` finds the stated reason does not apply to levels,
//! and the danger is that they conclude the *refusal* does not either. It does,
//! and the rule did not bend. What holds it is the second reason above, which is
//! untouched, plus two this section did not state. **The shape:** this is a
//! `const` table of eight entries each argued one at a time and checked at
//! compile time, and UAX #9's conformance files are hundreds of thousands of
//! cases chosen by exhaustiveness, with no sentence, no hazard and no distinct
//! signature — [`entries_exercise_different_things`] is not a statement anybody
//! can make about them. **And the decisive one: two passes have different
//! expectations about one sample.** `E3-B03e` expects levels for
//! [`Script::Mixed`], `E3-B03f` expects break positions, `E3-B03g` an atlas
//! residency, `E3-B03j` an image — so a field here would make *adding a script
//! is a diff to one list and to nothing else* false, and it would be the one
//! field in this file with no compile-time guard behind it, because
//! [`is_a_sentence`] can refuse an empty sentence and nothing here can refuse a
//! wrong level. Where they went instead: the specification's own conformance
//! files, under `third_party/unicode/` and read by a host harness (RFC 0114),
//! and the pass's own tests for these eight, each with the numbered rules it was
//! derived from written beside it.
//!
//! No language, either. [`Script::Han`] is not Chinese; the entry is about
//! boundaries and advances, and which language the sample is in changes nothing
//! that this crate will ever measure. An entry that needed a language to be
//! meaningful would be an entry about locale, which is a different axis and one
//! the shaper is structurally denied (RFC 0082, property 1).
//!
//! # Determinism
//!
//! Nothing here observes time, randomness or ordering. The corpus is a `const`
//! table and every function over it is a pure function of that table, so two
//! runs on two architectures read the same entries in the same order — which is
//! the property `E3-B03c`'s cache statistics rest on, and the reason the order
//! of the lines below is load-bearing rather than alphabetical. This crate has
//! no `f_env::Env` and should not grow one for this: a later task that wants to
//! draw a subset of the corpus takes a seed as a *parameter*, because a pure
//! function of its seed is more deterministic than one that draws, and a corpus
//! that reproduces from a number in the failure message is worth more than one
//! that reproduces from a run.

/// A closed list, written once so that it cannot be written twice.
///
/// Two of them below — [`Direction`] and [`Hazard`] — and the argument is the
/// one `interface/src/node.rs` makes at length for its own vocabulary: the
/// failure mode of a small enum is not the enum, it is the sequence beside it
/// that a new variant does not appear in. `ALL`, `COUNT`, `index` and `name`
/// are all derived from the single list of lines, so a variant that exists is a
/// variant every fold over these types sees.
///
/// Both lists are short enough that a reader may reasonably ask whether the
/// indirection is worth it. It is worth it here for a reason specific to this
/// file: [`every_hazard_is_exercised`] and [`every_direction_is_exercised`] are
/// compile-time assertions that quantify over `ALL`, so a hazard missing from
/// `ALL` would not be an unchecked hazard — it would be a hazard the corpus
/// silently claims to have covered. That is the failure this file is about,
/// one level up.
macro_rules! closed_list {
    (
        $(#[$about_list:meta])*
        $name:ident {
            $(
                $(#[$about:meta])*
                $variant:ident, $spelling:literal,
            )*
        }
    ) => {
        $(#[$about_list])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum $name {
            $($(#[$about])* $variant,)*
        }

        impl $name {
            /// How many there are.
            ///
            /// Counted from the list rather than written down, because a number
            /// written down twice is a number that can disagree with itself.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every one, in declaration order, indexed by
            /// [`index`](Self::index).
            ///
            /// Emitted from the same list as the variants, so there is no way
            /// to write a variant this array does not get. Not checked by a
            /// test: a loop over a list cannot see what the list omits, which
            /// is why the list is the only place a variant is written.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// This one's position in [`ALL`](Self::ALL).
            ///
            /// The discriminant is the position of the declaring line, which is
            /// the position of the entry in `ALL`: one list read three ways.
            #[must_use]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The name, for a failure message, a log line or a reproduction
            /// command somebody has to type.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }

            /// The one of that name, if there is one.
            ///
            /// Derived from [`ALL`](Self::ALL) rather than written as a second
            /// match, which is what makes the negative answer structural: there
            /// is no name this accepts that is not already a variant.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|it| it.name() == name)
            }
        }

        // Emitted from the line it checks, so a variant cannot arrive without
        // it. See [`is_a_name`] for why the shape of these strings is a
        // constraint rather than a convention: it is what lets a test observe
        // that `from_name` can refuse, without that test's chosen string being
        // something a later list could adopt.
        $(
            const _: () = assert!(
                is_a_name($spelling),
                concat!(
                    "`", stringify!($variant),
                    "`: a name is lowercase ASCII letters, digits and dashes, and not empty. \
                     See is_a_name.",
                ),
            );
        )*
    };
}

closed_list! {
    /// Which way a run of this entry's text reads.
    ///
    /// Three, and the third is not a convenience. A paragraph with one
    /// direction in it is laid out correctly by an implementation that has no
    /// concept of an embedding level at all, so [`Direction::Mixed`] is the
    /// only value under which UAX #9 is doing any work, and
    /// [`every_direction_is_exercised`] is what stops the corpus from drifting
    /// into a set of entries that are all one direction each.
    ///
    /// Vertical is absent, and its absence is a decision rather than an
    /// oversight: vertical layout changes what an advance *is*, not which order
    /// runs go in, so it belongs to whatever task owns the line box rather than
    /// to this axis. Adding it here would put a fourth value into a type whose
    /// consumers all branch on direction to decide reordering, and none of them
    /// would have anything to do with the new arm — which is the empty match
    /// arm this file is written to avoid.
    Direction {
        /// Reads left to right. The default the rest of this tree assumes when
        /// it is not thinking.
        LeftToRight, "ltr",
        /// Reads right to left, as one run, with no embedded run going the
        /// other way.
        RightToLeft, "rtl",
        /// Both, in one paragraph, with the levels and the neutrals between
        /// them left for UAX #9 to resolve.
        Mixed, "mixed",
    }
}

closed_list! {
    /// A way text can be harder than a Latin string, named so that an entry can
    /// say which ones it brings.
    ///
    /// These are not features of scripts. They are the specific assumptions a
    /// text stack makes when nobody is watching, each stated as the thing that
    /// breaks it: *one code point is one glyph*, *display order is storage
    /// order*, *a mark is a character*, *words are separated by spaces*, *lines
    /// break at spaces*, *a syllable is a scalar*, *digits read the way the
    /// text around them reads*. Naming them this way rather than by script is
    /// what lets [`every_hazard_is_exercised`] be a compile-time check with any
    /// meaning: a list of scripts can only assert that scripts are present,
    /// while a list of hazards asserts that the assumptions are all pressed on.
    ///
    /// Direction is not in this list, because [`Direction`] already carries it
    /// and a fact stated in two places is a fact that can come to disagree.
    /// [`Script::Hebrew`] is therefore the entry with an *empty* hazard set and
    /// a non-default direction, which is exactly what it is for.
    Hazard {
        /// The glyph chosen for a character depends on its neighbours — initial,
        /// medial, final, isolated. Breaks any shaper that maps code point to
        /// glyph and any cache keyed by a character rather than by a run.
        ContextualJoining, "joining",
        /// Display order within a cluster is not storage order. Breaks carets,
        /// hit testing and anything that assumes the nth glyph came from the
        /// nth scalar.
        Reordering, "reordering",
        /// A mark positions against a base and is not a cluster of its own.
        /// Breaks anything that counts characters and calls the answer a width.
        CombiningMarks, "marks",
        /// No U+0020 between words. Breaks any line breaker that scans for a
        /// space, and turns word segmentation into a dictionary problem.
        NoWordSpaces, "no-word-spaces",
        /// Almost every boundary between characters is a legal line break.
        /// Breaks the opposite assumption to the one above: that a breaker may
        /// refuse to break when it finds no space.
        BreakAnywhere, "break-anywhere",
        /// One displayed unit may be assembled from several scalars that are
        /// each a character in their own right. Breaks any equation between a
        /// scalar count, a cluster count and a glyph count.
        Composition, "composition",
        /// A number inside a right-to-left run reads left to right, at a level
        /// of its own. Breaks every bidirectional implementation that reverses
        /// runs and stops there.
        NumberDirection, "number-direction",
    }
}

/// A hazard's bit in a [`HazardSet`].
///
/// Derived from [`Hazard::index`] rather than written on the variant, so a
/// hazard cannot be given a bit another hazard already has — the one way a
/// hand-assigned bitset goes wrong, and it goes wrong silently, by making two
/// hazards indistinguishable to [`every_hazard_is_exercised`].
impl Hazard {
    /// This hazard's bit.
    #[must_use]
    pub const fn bit(self) -> u16 {
        1u16 << self.index()
    }
}

/// The set type is a `u16`, so a hazard past the sixteenth would silently shift
/// out of it. It is a compile error instead, and the fix is to widen the set
/// rather than to notice.
const _: () = assert!(Hazard::COUNT <= 16, "more hazards than HazardSet has bits: widen it");

/// Which hazards an entry brings.
///
/// A bitset rather than a slice because the corpus-level checks compare and
/// union these, and a `BTreeSet` would want an allocator this crate does not
/// have. Not a `HashSet` for the reason the crate documentation gives at
/// length: iteration order is seeded per process, and a corpus that reports its
/// coverage in a different order on two runs is a corpus whose failures cannot
/// be diffed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HazardSet(u16);

impl HazardSet {
    /// The set an entry that brings no hazard declares. Legal, and
    /// [`Script::Latin`] argues why.
    pub const NONE: Self = Self(0);

    /// The set of exactly these hazards.
    ///
    /// Takes a slice so that the corpus line reads as a list of names. Writing
    /// a hazard twice is harmless and silent, which is the one imprecision here
    /// and is deliberate: refusing it would need an error path in a `const fn`
    /// whose only caller is a literal somebody is reading as they write it.
    #[must_use]
    pub const fn of(hazards: &[Hazard]) -> Self {
        let mut bits = 0u16;
        let mut i = 0;
        while i < hazards.len() {
            bits |= hazards[i].bit();
            i += 1;
        }
        Self(bits)
    }

    /// Does this set bring that hazard?
    #[must_use]
    pub const fn contains(self, hazard: Hazard) -> bool {
        self.0 & hazard.bit() != 0
    }

    /// Does this set bring nothing?
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many hazards it brings.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// The raw bits, for the signature that distinguishes two entries.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }
}

/// One inclusive range of Unicode scalar values, `(first, last)`.
///
/// A pair and not a `RangeInclusive`, because every use of these is inside a
/// `const fn` and `RangeInclusive::contains` is not `const`. The cost is that a
/// reversed pair is not a type error; [`declared_width`] is what notices, since
/// a reversed pair has a width of zero and fails the budget from the wrong
/// side — and [`every_range_is_reached`] fails first, because no scalar is in
/// it.
pub type ScalarRange = (u32, u32);

/// How many scalars one entry may declare across all its ranges.
///
/// Without a budget, check (2) in the module documentation is satisfiable by
/// declaring `(0x0000, 0x10FFFF)`, and an entry that accepts every scalar makes
/// no claim about what its sample is. The number is set just above the widest
/// block a current entry needs — CJK Unified Ideographs, `0x4E00..=0x9FFF`, is
/// 20 992 scalars — and well below the 65 536 that would accept the whole Basic
/// Multilingual Plane and with it every Latin letter.
///
/// It will be wrong the day an entry needs a plane-1 extension block, and that
/// is the reversal condition: raise it in the same diff that adds the entry,
/// with the entry as the argument. It is deliberately not set high enough to
/// never be in the way.
pub const RANGE_BUDGET: u32 = 24_576;

/// The fewest bytes a sentence saying what an absence would hide can occupy.
///
/// A threshold rather than a reviewer's memory, because the field this guards
/// is the first clause of this task's exit and the way that clause dies is
/// somebody writing `"right to left"` in it and moving on. Eighty bytes is
/// about a line of prose; nothing shorter has room for both a mechanism and a
/// consequence, and every sentence below is two to three times it.
///
/// What it cannot check is that the sentence is about the *absence*. Nothing
/// mechanical can, and pretending otherwise with a substring search for the
/// word *hide* would make the check passable by writing the word. The reversal
/// condition is a genuine short sentence this refuses; the repair then is to
/// lower the number, not to delete the check.
pub const SENTENCE_MIN_BYTES: usize = 80;

/// Is this a sentence rather than a label?
///
/// Length and a full stop. That is all it is, and the documentation on
/// [`SENTENCE_MIN_BYTES`] says why it is not more.
#[must_use]
pub const fn is_a_sentence(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() >= SENTENCE_MIN_BYTES && bytes[bytes.len() - 1] == b'.'
}

/// Is this the shape a name on one of these lists is allowed to take —
/// non-empty, and lowercase ASCII letters, digits and dashes and nothing else?
///
/// This exists for one reason, and it is a reason about tests rather than about
/// names. A test that wants to observe that [`Script::from_name`] can say *no*
/// has to hand it a string that is not an entry, and the obvious way to write
/// that is to pick a script the corpus does not have — `"cyrillic"`, say. That
/// is a landmine: the day somebody argues Cyrillic onto the list, a test they
/// have no reason to be reading goes red, and adding a script has become a diff
/// to a test after all. The first draft of this file had exactly that line in
/// it.
///
/// Constraining the shape removes the possibility instead of documenting it.
/// Every name on every list here is asserted at compile time to be of this
/// shape, so a string containing a space or a capital is one that no future
/// entry can ever be called, and a test may hand one to `from_name` knowing the
/// answer can never change. The negative case stops depending on what the
/// corpus does not contain yet.
///
/// The reversal condition is a name that genuinely wants a character outside
/// this set. None is in sight — these are identifiers for failure messages and
/// reproduction commands, not display text — and if one arrives, the repair is
/// to widen this predicate and re-check that the strings the tests rely on are
/// still outside it, in the same diff.
#[must_use]
pub const fn is_a_name(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return false;
    }
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        let allowed = byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-';
        if !allowed {
            return false;
        }
        i += 1;
    }
    true
}

/// The scalar beginning at `at`, and where the next one begins.
///
/// A UTF-8 decoder written out by hand, because `str::chars` is not `const` and
/// the checks that use this have to run at compile time or they are tests
/// again. It assumes well-formed input, which is not an assumption: `&str` is
/// well-formed by construction, and this is never handed anything else.
/// `the_const_decoder_agrees_with_core_on_every_sample` is what keeps the
/// assumption honest, and it is the one test in this file that reads the
/// corpus, because it is checking this function rather than the entries.
#[must_use]
const fn scalar_at(bytes: &[u8], at: usize) -> (u32, usize) {
    let lead = bytes[at] as u32;
    if lead < 0x80 {
        (lead, at + 1)
    } else if lead < 0xE0 {
        (((lead & 0x1F) << 6) | (bytes[at + 1] as u32 & 0x3F), at + 2)
    } else if lead < 0xF0 {
        let mid = (bytes[at + 1] as u32 & 0x3F) << 6;
        (((lead & 0x0F) << 12) | mid | (bytes[at + 2] as u32 & 0x3F), at + 3)
    } else {
        let high = (bytes[at + 1] as u32 & 0x3F) << 12;
        let mid = (bytes[at + 2] as u32 & 0x3F) << 6;
        (((lead & 0x07) << 18) | high | mid | (bytes[at + 3] as u32 & 0x3F), at + 4)
    }
}

/// Is `scalar` inside `range`?
///
/// Spelled as two comparisons rather than as `contains`, for the reason
/// [`ScalarRange`] gives.
#[must_use]
const fn holds(range: ScalarRange, scalar: u32) -> bool {
    let (first, last) = range;
    first <= scalar && scalar <= last
}

/// Does every scalar of `sample` lie in one of `ranges`?
///
/// An empty sample is refused rather than passing vacuously, and so is an empty
/// range list. Both are the shape this check exists to catch: an entry that
/// declares nothing satisfies every quantifier over what it declared.
#[must_use]
pub const fn sample_is_covered(sample: &str, ranges: &[ScalarRange]) -> bool {
    let bytes = sample.as_bytes();
    if bytes.is_empty() || ranges.is_empty() {
        return false;
    }
    let mut at = 0;
    while at < bytes.len() {
        let (scalar, next) = scalar_at(bytes, at);
        let mut i = 0;
        let mut inside = false;
        while i < ranges.len() {
            if holds(ranges[i], scalar) {
                inside = true;
            }
            i += 1;
        }
        if !inside {
            return false;
        }
        at = next;
    }
    true
}

/// Does `sample` actually reach every one of `ranges`?
///
/// The half that makes a declared range a promise rather than permission. An
/// entry declaring three blocks is claiming its sample uses three blocks, and
/// this is what makes deleting one of them a compile error instead of a quietly
/// weaker corpus. [`Script::Hangul`] is the entry that turns on it.
#[must_use]
pub const fn every_range_is_reached(sample: &str, ranges: &[ScalarRange]) -> bool {
    if ranges.is_empty() {
        return false;
    }
    let bytes = sample.as_bytes();
    let mut i = 0;
    while i < ranges.len() {
        let mut at = 0;
        let mut reached = false;
        while at < bytes.len() {
            let (scalar, next) = scalar_at(bytes, at);
            if holds(ranges[i], scalar) {
                reached = true;
            }
            at = next;
        }
        if !reached {
            return false;
        }
        i += 1;
    }
    true
}

/// How many scalars `ranges` accepts in total, saturating.
///
/// Overlap is counted twice and a reversed pair counts as zero. Neither matters
/// for what this is for, which is refusing a range list wide enough to accept
/// anything; both would matter if this number were ever published, which is why
/// it is not.
#[must_use]
pub const fn declared_width(ranges: &[ScalarRange]) -> u32 {
    let mut total = 0u32;
    let mut i = 0;
    while i < ranges.len() {
        let (first, last) = ranges[i];
        if first <= last {
            total = total.saturating_add(last - first + 1);
        }
        i += 1;
    }
    total
}

/// Are these two strings byte-for-byte the same?
///
/// `str`'s own `==` is not `const`, and the distinctness checks below have to
/// run at compile time for the same reason everything else here does.
#[must_use]
const fn same_text(left: &str, right: &str) -> bool {
    let (left, right) = (left.as_bytes(), right.as_bytes());
    if left.len() != right.len() {
        return false;
    }
    let mut i = 0;
    while i < left.len() {
        if left[i] != right[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// Is every string in `list` different from every other?
#[must_use]
pub const fn all_distinct_text(list: &[&str]) -> bool {
    let mut i = 0;
    while i < list.len() {
        let mut j = i + 1;
        while j < list.len() {
            if same_text(list[i], list[j]) {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// Is every key in `list` different from every other?
#[must_use]
pub const fn all_distinct_keys(list: &[u32]) -> bool {
    let mut i = 0;
    while i < list.len() {
        let mut j = i + 1;
        while j < list.len() {
            if list[i] == list[j] {
                return false;
            }
            j += 1;
        }
        i += 1;
    }
    true
}

/// What an entry exercises, as one comparable number.
///
/// Direction in the high half, hazards in the low half. Two entries with the
/// same signature press on the same assumptions in the same direction, and the
/// corpus is supposed to be argued one entry at a time — so
/// [`entries_exercise_different_things`] refuses that, and the escape is not to
/// weaken the check but to say what makes the new entry different, which means
/// naming a [`Hazard`] the list does not have yet. Widening the hazard
/// vocabulary is itself a diff to a list, and a visible one.
#[must_use]
pub const fn signature(direction: Direction, hazards: HazardSet) -> u32 {
    ((direction.index() as u32) << 16) | hazards.bits() as u32
}

/// The corpus, written once so that it cannot be written twice.
///
/// Each line is one entry and carries everything anything asks an entry for:
/// what it is called, which way it reads, which assumptions it presses on, the
/// text itself, the scalar ranges that text is promised to stay inside and
/// cover, and the sentence saying what its absence would hide. From that line
/// come the variant, the array, the count, every accessor, and the four
/// compile-time assertions that check it — so the diff that adds a script is
/// the line, and there is nothing else in this file or any other that has to be
/// taught the entry exists.
///
/// The sample is written as `\u{…}` escapes everywhere except
/// [`Script::Latin`], and that is not fussiness. A corpus entry whose content
/// can only be checked by a reviewer whose editor and font happen to render the
/// script is a corpus entry nobody checks; escapes can be read against the code
/// charts by anybody. Latin is the exception because Latin is the one script
/// every reviewer of this tree can already read, and escaping it would hide the
/// one sample that is meant to be obvious. Each entry's documentation says what
/// its sample is in words.
macro_rules! corpus {
    (
        $(
            $(#[$about:meta])*
            $variant:ident, $spelling:literal, $direction:ident, $hazards:expr,
            $sample:expr, $ranges:expr, $hides:literal,
        )*
    ) => {
        /// A script the text path is tested against, and the argument for it.
        ///
        /// Closed, and closed the way the rest of this tree means it: no
        /// `Other`, no variant carrying a string, no `#[non_exhaustive]`. A
        /// corpus that can be extended at a call site is a corpus whose
        /// contents are not the thing that was argued.
        ///
        /// Read the entries rather than the count. Each one's documentation
        /// says what its sample is and why it is here rather than the script
        /// next to it, and [`hides`](Self::hides) is the one sentence that has
        /// to be true for the entry to be worth its place.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Script {
            $($(#[$about])* $variant,)*
        }

        impl Script {
            /// How many entries the corpus has.
            ///
            /// Counted from the list. It is not a target and it is not a
            /// budget: an entry earns its place by the sentence on it, and the
            /// number is whatever that produces.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every entry, in declaration order, indexed by
            /// [`index`](Self::index).
            ///
            /// Every consumer of this corpus folds over this array, so an entry
            /// that exists is an entry they all see. That is the whole of the
            /// mechanism and it is why there is no test asserting the array is
            /// complete: such a test could not see an omission, and deleting
            /// the possibility is worth more than watching for it.
            pub const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// This entry's position in [`ALL`](Self::ALL).
            ///
            /// Declaration order, which is argued order: [`Script::Latin`]
            /// first because it is the control, and the mixed case last because
            /// it is the one that presumes the others work.
            #[must_use]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The entry's name, for a failure message or a reproduction
            /// command.
            ///
            /// Two entries sharing a name would make
            /// [`from_name`](Self::from_name) answer with the earlier one, so
            /// the names are asserted distinct at compile time rather than
            /// tested.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }

            /// The entry of that name, if the corpus has one.
            ///
            /// Derived from [`ALL`](Self::ALL), so there is no name this
            /// accepts that is not already an entry.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|it| it.name() == name)
            }

            /// Which way this entry's sample reads.
            #[must_use]
            pub const fn direction(self) -> Direction {
                match self {
                    $(Self::$variant => Direction::$direction,)*
                }
            }

            /// Which assumptions this entry presses on.
            ///
            /// The empty set is legal and two entries have a claim on it; see
            /// [`Script::Latin`] and [`Script::Hebrew`], which are the corpus's
            /// two controls and differ only in [`direction`](Self::direction).
            #[must_use]
            pub const fn hazards(self) -> HazardSet {
                match self {
                    $(Self::$variant => HazardSet::of($hazards),)*
                }
            }

            /// The text itself.
            ///
            /// Short on purpose. The corpus is about which questions get asked,
            /// and a longer sample asks the same ones more slowly — while
            /// making the entry something a reviewer skims rather than reads.
            #[must_use]
            pub const fn sample(self) -> &'static str {
                match self {
                    $(Self::$variant => $sample,)*
                }
            }

            /// The scalar ranges the sample is promised to stay inside and to
            /// cover.
            ///
            /// Both directions are compile-time assertions: nothing outside
            /// them appears in the sample, and nothing in them is unused. See
            /// the module documentation, checks (2) and (3).
            #[must_use]
            pub const fn ranges(self) -> &'static [ScalarRange] {
                match self {
                    $(Self::$variant => $ranges,)*
                }
            }

            /// What this entry's absence would hide.
            ///
            /// The sentence this task's exit asks for, kept on the entry rather
            /// than in a document beside it, because a document beside it is
            /// the thing that stops being edited. It is asserted to be a
            /// sentence at compile time and asserted distinct from every other
            /// entry's, which is as far as a machine can take it; the rest is
            /// on whoever writes the line and whoever reviews it.
            #[must_use]
            pub const fn hides(self) -> &'static str {
                match self {
                    $(Self::$variant => $hides,)*
                }
            }

            /// The names, for the distinctness assertion. Straight off the
            /// list, so it cannot disagree with [`name`](Self::name).
            const NAMES: [&'static str; Self::COUNT] = [$($spelling,)*];

            /// The sentences, for the distinctness assertion. Copy-and-paste is
            /// the realistic way the first clause of this task's exit dies, and
            /// this is what makes it a compile error.
            const SENTENCES: [&'static str; Self::COUNT] = [$($hides,)*];

            /// What each entry exercises, for the distinctness assertion.
            const SIGNATURES: [u32; Self::COUNT] =
                [$(signature(Direction::$direction, HazardSet::of($hazards)),)*];
        }

        // The per-entry checks. They are emitted from the line they check, so
        // an entry cannot arrive without them, and they are `const` so that a
        // bad entry is a compile error naming the entry rather than a red test
        // somebody can delete. Module documentation, "Why the per-entry checks
        // are `const` and not tests".
        $(
            const _: () = assert!(
                is_a_name($spelling),
                concat!(
                    "corpus entry `", stringify!($variant),
                    "`: a name is lowercase ASCII letters, digits and dashes, and not empty. \
                     See is_a_name.",
                ),
            );
            const _: () = assert!(
                is_a_sentence($hides),
                concat!(
                    "corpus entry `", stringify!($variant),
                    "`: the sentence saying what its absence would hide is too short, or does \
                     not end in a full stop. See SENTENCE_MIN_BYTES.",
                ),
            );
            const _: () = assert!(
                sample_is_covered($sample, $ranges),
                concat!(
                    "corpus entry `", stringify!($variant),
                    "`: its sample contains a scalar outside every range it declared, or one \
                     of the two is empty.",
                ),
            );
            const _: () = assert!(
                every_range_is_reached($sample, $ranges),
                concat!(
                    "corpus entry `", stringify!($variant),
                    "`: it declares a scalar range its sample never reaches, so the sample no \
                     longer covers what the entry claims.",
                ),
            );
            const _: () = assert!(
                declared_width($ranges) <= RANGE_BUDGET,
                concat!(
                    "corpus entry `", stringify!($variant),
                    "`: its declared ranges are wide enough to accept almost anything, which \
                     makes the coverage check vacuous. See RANGE_BUDGET.",
                ),
            );
        )*
    };
}

corpus! {
    /// `The quick brown fox jumps over the lazy dog.` — the control, and the
    /// only sample written as characters rather than escapes.
    ///
    /// One scalar is one glyph, storage order is display order, words are
    /// separated by U+0020, and lines break at those spaces. Every one of those
    /// is false somewhere else in this list, which is the point of having an
    /// entry where all four hold: it is the only entry whose failure can be
    /// attributed to the common path without further argument.
    ///
    /// It is first so that a reader reaches the hardest entry having seen the
    /// easiest, and so that `index() == 0` is the one entry it is safe for a
    /// hurried caller to reach for.
    Latin, "latin", LeftToRight, &[],
    "The quick brown fox jumps over the lazy dog.",
    &[(0x0020, 0x007E)],
    "Its absence would hide which half of a failure belongs to the common path: with nothing \
     in the corpus that exercises no hazard at all, a regression in code every script runs and \
     a regression in code one script runs arrive looking identical.",

    /// `\u{05e9}\u{05dc}\u{05d5}\u{05dd} \u{05e2}\u{05d5}\u{05dc}\u{05dd}` —
    /// *shalom olam*, two words of Hebrew separated by a space.
    ///
    /// The second control, and the one that costs an entry to have. Hebrew
    /// reads right to left and does almost no shaping: no joining forms, no
    /// reordering, and none in this sample. Its hazard set is therefore empty
    /// and only its direction differs from [`Script::Latin`] — which is what
    /// separates *direction is wrong* from *shaping is wrong*, two failures
    /// that are impossible to tell apart if the only right-to-left entry is
    /// Arabic, because Arabic reverses and joins in the same pass.
    ///
    /// The space is declared as its own range so that it is visible that this
    /// entry has one; it is what makes it comparable to Latin and not to
    /// [`Script::Thai`].
    Hebrew, "hebrew", RightToLeft, &[],
    "\u{05e9}\u{05dc}\u{05d5}\u{05dd} \u{05e2}\u{05d5}\u{05dc}\u{05dd}",
    &[(0x0020, 0x0020), (0x05d0, 0x05ea)],
    "Its absence would hide a direction bug that only survives where glyphs join, because an \
     implementation that reached direction through the joining machinery would still be right \
     on every Arabic sample and wrong on every Hebrew one.",

    /// `\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} \u{0628}\u{0627}\u{0644}\u{0639}\u{0627}\u{0644}\u{0645}`
    /// — *marhaban bil-alam*, two words of Arabic.
    ///
    /// Contextual joining, which is the hazard with the widest blast radius in
    /// this list. The glyph for a letter depends on whether it has a joining
    /// neighbour on each side, so U+0644 in the middle of the second word and
    /// U+0644 at its end are different glyphs from the same scalar. Anything
    /// that maps a character to a glyph, keys a cache by a character, or
    /// measures a substring by summing its characters' advances is wrong here
    /// and nowhere else in the corpus except [`Script::Mixed`].
    ///
    /// The sample is deliberately unvocalised. Marks belong to
    /// [`Script::Devanagari`] and [`Script::Thai`], where they are the point;
    /// putting them here too would make this entry's signature overlap theirs
    /// and blur what it is for.
    Arabic, "arabic", RightToLeft, &[Hazard::ContextualJoining],
    "\u{0645}\u{0631}\u{062d}\u{0628}\u{0627} \u{0628}\u{0627}\u{0644}\u{0639}\u{0627}\u{0644}\u{0645}",
    &[(0x0020, 0x0020), (0x0621, 0x064a)],
    "Its absence would hide a shaper that picks a glyph by code point rather than by a run \
     position, which is invisible in every script where a letter has exactly one form and \
     wrong four ways over in a script where it has four.",

    /// `\u{0928}\u{092e}\u{0938}\u{094d}\u{0924}\u{0947} \u{0926}\u{0941}\u{0928}\u{093f}\u{092f}\u{093e}`
    /// — *namaste duniya*, two words of Devanagari.
    ///
    /// Reordering, and the specific scalar that produces it is U+093F in the
    /// second word: a vowel sign stored *after* the consonant it is drawn
    /// *before*. There is no ordering of the glyphs that agrees with the
    /// ordering of the scalars, which is a thing most text stacks do not so
    /// much get wrong as fail to have a place to represent. The first word adds
    /// U+094D, a virama forming a conjunct, so one cluster is two consonants
    /// and a mark rendered as one shape.
    ///
    /// This is the entry that breaks the tempting equivalence between a caret
    /// position, a scalar offset and a glyph index. A stack that passes here
    /// has had to name a cluster as a thing, which is what `E3-B03f` needs and
    /// cannot get from any other entry.
    Devanagari, "devanagari", LeftToRight, &[Hazard::Reordering, Hazard::CombiningMarks],
    "\u{0928}\u{092e}\u{0938}\u{094d}\u{0924}\u{0947} \u{0926}\u{0941}\u{0928}\u{093f}\u{092f}\u{093e}",
    &[(0x0020, 0x0020), (0x0900, 0x097f)],
    "Its absence would hide every assumption that display order is storage order: U+093F is \
     stored after the consonant it is drawn before, so a caret, a hit test and a cluster \
     boundary can each be wrong while the advances still sum to the right width.",

    /// `\u{0e2a}\u{0e27}\u{0e31}\u{0e2a}\u{0e14}\u{0e35}\u{0e0a}\u{0e32}\u{0e27}\u{0e42}\u{0e25}\u{0e01}`
    /// — *sawatdi chao lok*, three Thai words written the way Thai writes them,
    /// which is without anything between them.
    ///
    /// The declared range excludes U+0020, so the absence of spaces is not a
    /// property of this sample that a later edit could quietly lose: adding a
    /// space to it is a compile error. That is the entry's whole value. Word
    /// segmentation in Thai is a dictionary problem, and a line breaker that
    /// scans for spaces does not fail loudly here — it returns one unbreakable
    /// run and looks correct until the text is wider than the box.
    ///
    /// Above and below the consonants are U+0E31 and U+0E35, vowels that are
    /// marks, so this entry also carries the case where the number of things
    /// you can see is smaller than the number of scalars.
    Thai, "thai", LeftToRight, &[Hazard::NoWordSpaces, Hazard::CombiningMarks],
    "\u{0e2a}\u{0e27}\u{0e31}\u{0e2a}\u{0e14}\u{0e35}\u{0e0a}\u{0e32}\u{0e27}\u{0e42}\u{0e25}\u{0e01}",
    &[(0x0e01, 0x0e4e)],
    "Its absence would hide a line breaker that scans for U+0020 and calls what it finds a \
     word boundary, which produces a single unbreakable line here and stays invisible until \
     that line is wider than the box it was given.",

    /// `\u{4e16}\u{754c}\u{4f60}\u{597d}` — four Han ideographs, no spaces.
    ///
    /// The mirror of [`Script::Thai`], and the reason both are here rather than
    /// one standing for the pair. Thai has no spaces and breaking it needs a
    /// dictionary; Han has no spaces and breaks almost anywhere. A breaker that
    /// refuses to break without a space and a breaker that breaks at every
    /// opportunity agree on every other entry in this corpus and disagree here,
    /// so this is the only entry at which the difference between *no breaks
    /// found* and *breaks everywhere* is observable at all.
    ///
    /// It is also the corpus's only sample outside the ranges a one-byte or
    /// two-byte encoding reaches, which is incidental to the hazards and useful
    /// anyway: every offset arithmetic bug that assumes a narrow encoding shows
    /// up here first.
    Han, "han", LeftToRight, &[Hazard::NoWordSpaces, Hazard::BreakAnywhere],
    "\u{4e16}\u{754c}\u{4f60}\u{597d}",
    &[(0x4e00, 0x9fff)],
    "Its absence would hide the error opposite to Thai's: a breaker that refuses to break \
     without a space and a breaker that breaks anywhere it can agree on every other entry in \
     this corpus, and this is the one place they disagree.",

    /// `\u{d55c}\u{ae00} \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}` — the
    /// word *hangul* written twice: first as two precomposed syllables, then as
    /// the six jamo those syllables are composed from.
    ///
    /// One sample containing the same two syllables in both forms, so a stack
    /// that treats a scalar as a displayed unit produces two glyphs for the
    /// first half and six for the second and cannot be right about both. A
    /// count of scalars, a count of clusters and a count of glyphs are three
    /// different numbers here, and a caret that steps by the first walks into
    /// the middle of a syllable.
    ///
    /// Both halves are held in place by the declared ranges: the entry names
    /// the jamo block and the precomposed block separately, and
    /// [`every_range_is_reached`] refuses the entry if either half of the
    /// sample is deleted. That is deliberate — the realistic way this entry
    /// decays is somebody shortening the sample and leaving the easy half.
    Hangul, "hangul", LeftToRight, &[Hazard::Composition],
    "\u{d55c}\u{ae00} \u{1112}\u{1161}\u{11ab}\u{1100}\u{1173}\u{11af}",
    &[(0x0020, 0x0020), (0x1100, 0x11ff), (0xac00, 0xd7a3)],
    "Its absence would hide that a displayed syllable may be several scalars, so a count of \
     scalars, a count of clusters and a count of glyphs are three different numbers, and a \
     caret stepping by the first walks into the middle of a syllable.",

    /// `\u{0627}\u{0644}\u{0639}\u{062f}\u{062f} 42 F` — the Arabic word
    /// *al-adad*, a Western number, and a Latin letter, in one paragraph.
    ///
    /// The entry that hides the hardest bugs, and the last one because it
    /// presumes the others. Three runs at three embedding levels: the Arabic at
    /// level 1, the digits at level 2 — numbers read left to right inside a
    /// right-to-left run, which is the rule almost every homegrown
    /// implementation omits — and the Latin back at level 0. Between them are
    /// neutrals whose direction is decided by what surrounds them rather than
    /// by what they are.
    ///
    /// Nothing before this line asks for an embedding level to exist. Each of
    /// [`Script::Hebrew`] and [`Script::Arabic`] is laid out correctly by an
    /// implementation that reverses the whole paragraph and stops, which is why
    /// a corpus of single-direction entries can be fully green against a
    /// bidirectional algorithm that was never written. This is also the entry
    /// that makes [`Direction::Mixed`] non-empty, and
    /// [`every_direction_is_exercised`] is what stops it from being dropped.
    Mixed, "mixed", Mixed, &[Hazard::ContextualJoining, Hazard::NumberDirection],
    "\u{0627}\u{0644}\u{0639}\u{062f}\u{062f} 42 F",
    &[(0x0020, 0x0020), (0x0030, 0x0039), (0x0041, 0x005a), (0x0621, 0x064a)],
    "Its absence would hide all of UAX #9 above the trivial case, because a paragraph with one \
     direction in it is laid out correctly by an implementation that has no concept of an \
     embedding level and no rule for the digits between the runs.",
}

/// Is every [`Hazard`] brought by at least one entry?
///
/// The check that turns the hazard list into an obligation on the corpus rather
/// than a vocabulary beside it. Naming a hazard and covering it are then one
/// act: adding a hazard with no entry that brings it does not compile, and the
/// diff that adds it has to arrive with the entry that argues it.
#[must_use]
pub const fn every_hazard_is_exercised() -> bool {
    let mut h = 0;
    while h < Hazard::COUNT {
        let hazard = Hazard::ALL[h];
        let mut i = 0;
        let mut brought = false;
        while i < Script::COUNT {
            if Script::ALL[i].hazards().contains(hazard) {
                brought = true;
            }
            i += 1;
        }
        if !brought {
            return false;
        }
        h += 1;
    }
    true
}

/// Is every [`Direction`] read by at least one entry?
///
/// Same obligation on the other axis, and the one that matters is
/// [`Direction::Mixed`]: a corpus can lose its only mixed-direction entry and
/// still look broad, because it still has a right-to-left one.
#[must_use]
pub const fn every_direction_is_exercised() -> bool {
    let mut d = 0;
    while d < Direction::COUNT {
        let direction = Direction::ALL[d];
        let mut i = 0;
        let mut read = false;
        while i < Script::COUNT {
            if Script::ALL[i].direction().index() == direction.index() {
                read = true;
            }
            i += 1;
        }
        if !read {
            return false;
        }
        d += 1;
    }
    true
}

/// Does every entry exercise something no other entry exercises?
///
/// *Argued one entry at a time* is this task's phrase, and this is the nearest
/// a machine can come to it: two entries with the same [`signature`] press on
/// the same assumptions in the same direction, and the second is a second test
/// of the first rather than a new question.
///
/// The reversal condition is a script that genuinely belongs here and whose
/// difference is not expressible as a [`Hazard`] — Syriac, say, which joins and
/// reads right to left exactly as Arabic does. This check refuses it, and
/// refusing it is correct until somebody can name what it brings; the repair is
/// then to add the hazard, not to delete the check, and adding a hazard is
/// itself a line on a list somebody reviews.
#[must_use]
pub const fn entries_exercise_different_things() -> bool {
    all_distinct_keys(&Script::SIGNATURES)
}

const _: () = assert!(
    all_distinct_text(&Script::NAMES),
    "two corpus entries share a name, so `Script::from_name` can never answer with the second",
);
const _: () = assert!(
    all_distinct_text(&Script::SENTENCES),
    "two corpus entries share the sentence saying what their absence would hide, so at least \
     one of them has not been argued",
);
const _: () = assert!(
    entries_exercise_different_things(),
    "two corpus entries exercise the same hazards in the same direction; name what the new one \
     brings, as a Hazard, or it is a second test of an entry that already exists",
);
const _: () = assert!(
    every_hazard_is_exercised(),
    "a Hazard is named in the list and brought by no corpus entry, so the corpus claims a \
     coverage it does not have",
);
const _: () = assert!(
    every_direction_is_exercised(),
    "a Direction is named in the list and read by no corpus entry",
);

#[cfg(test)]
mod tests {
    use super::*;

    /// A string that `is_a_name` refuses, so no entry on any list in this file
    /// can ever be called it. The space and the capitals are what do the work;
    /// the words are for whoever reads the failure.
    const NOT_A_NAME: &str = "Not A Name On Any List Here";

    // Everything about an entry is checked above, at compile time, by an
    // assertion emitted from the entry's own line. What is left for a test is
    // the thing those assertions cannot check about themselves: a checker that
    // answers `true` for every input makes every `const _` above it pass while
    // observing nothing, which is the const-assert form of the empty match arm
    // this file's module documentation is about.
    //
    // So each test below hands a checker something it must refuse and something
    // it must accept. None of them names a corpus entry, and the one that reads
    // the corpus at all is reading it to check the decoder rather than the
    // entries. That is why adding a script is a diff to the list above and to
    // nothing in here.

    #[test]
    fn the_const_decoder_agrees_with_core_on_every_sample() {
        // `scalar_at` is hand-written because `str::chars` is not `const`, so
        // it is the one piece of machinery here with no compiler holding it to
        // anything. Over every sample in the corpus it must agree with core,
        // scalar for scalar and boundary for boundary — and this test grows
        // with the list on its own, which is the property the exit asks for
        // stated on the one test that could have needed editing.
        for script in Script::ALL {
            let bytes = script.sample().as_bytes();
            let mut at = 0;
            for expected in script.sample().chars() {
                let (scalar, next) = scalar_at(bytes, at);
                assert_eq!(scalar, expected as u32, "{} at byte {at}", script.name());
                assert!(next > at, "{} made no progress at byte {at}", script.name());
                at = next;
            }
            assert_eq!(at, bytes.len(), "{} ended off a boundary", script.name());
        }
    }

    #[test]
    fn a_sample_outside_its_declared_ranges_is_refused() {
        // The failure this refuses is Latin transliteration pasted in by
        // somebody whose editor could not produce the script.
        assert!(!sample_is_covered("namaste", &[(0x0900, 0x097f)]));
        assert!(sample_is_covered("\u{0928}\u{092e}", &[(0x0900, 0x097f)]));
    }

    #[test]
    fn an_entry_that_declares_nothing_is_refused_rather_than_passing_vacuously() {
        // Both quantifiers above are over what the entry declared, so an entry
        // that declares nothing satisfies them for free. That is the shape the
        // emptiness checks exist to catch, and it is checked here because a
        // corpus entry can never reach it — the assertions on the line refuse
        // it first.
        assert!(!sample_is_covered("", &[(0x0020, 0x0020)]));
        assert!(!sample_is_covered("a", &[]));
        assert!(!every_range_is_reached("a", &[]));
    }

    #[test]
    fn a_declared_range_the_sample_never_reaches_is_refused() {
        // The half that keeps a multi-block entry honest when its sample is
        // shortened: declaring a block is a promise that the sample uses it.
        assert!(!every_range_is_reached("ab", &[(0x0061, 0x0062), (0x1100, 0x11ff)]));
        assert!(every_range_is_reached("a\u{1112}", &[(0x0061, 0x0062), (0x1100, 0x11ff)]));
    }

    #[test]
    fn a_label_is_not_a_sentence() {
        assert!(!is_a_sentence(""));
        assert!(!is_a_sentence("right to left."));
        // Long enough, and still not a sentence, because it does not end.
        // Both of these clear SENTENCE_MIN_BYTES, so the two halves of the
        // check are observed one at a time rather than together.
        assert!(!is_a_sentence(
            "a clause of quite sufficient length that simply never comes to any sort of stop at all"
        ));
        assert!(is_a_sentence(
            "A clause of quite sufficient length that does come to a stop at the end, as this one does."
        ));
    }

    #[test]
    fn a_name_predicate_that_agreed_with_everything_would_disarm_the_tests() {
        // `is_a_name` is checked at compile time on every entry, and it is also
        // what makes `NOT_A_NAME` permanently safe to hand to `from_name`. A
        // predicate that answered `true` for every input would satisfy both
        // silently: the const assertions would pass observing nothing, and the
        // negative case above would become a claim about a string that could
        // one day be an entry. So it is made to refuse, one rejected character
        // class at a time.
        assert!(!is_a_name(""));
        assert!(!is_a_name("Latin"), "a capital is refused");
        assert!(!is_a_name("two words"), "a space is refused");
        assert!(!is_a_name("latin."), "punctuation is refused");
        assert!(!is_a_name(NOT_A_NAME));
        assert!(is_a_name("latin"));
        assert!(is_a_name("no-word-spaces"), "a dash is how a name gets two words");
        assert!(is_a_name("utf8"), "a digit is allowed, for a name that carries one");
    }

    #[test]
    fn the_width_budget_refuses_a_range_that_would_accept_anything() {
        // Without this, the coverage check is satisfiable by declaring the
        // whole of Unicode, which is a declaration that claims nothing.
        assert!(declared_width(&[(0x0000, 0x10ffff)]) > RANGE_BUDGET);
        assert!(declared_width(&[(0x0000, 0xffff)]) > RANGE_BUDGET);
        assert!(declared_width(&[(0x4e00, 0x9fff)]) <= RANGE_BUDGET);
        // A reversed pair contributes nothing rather than wrapping.
        assert_eq!(declared_width(&[(0x0100, 0x0010)]), 0);
    }

    #[test]
    fn the_distinctness_checks_can_say_no() {
        // The three corpus-level assertions rest entirely on these two, and a
        // distinctness check that always agrees is worse than none: it reports
        // that the corpus has been argued.
        assert!(!all_distinct_text(&["one", "two", "one"]));
        assert!(all_distinct_text(&["one", "two", "three"]));
        assert!(!all_distinct_keys(&[3, 1, 3]));
        assert!(all_distinct_keys(&[3, 1, 2]));
    }

    #[test]
    fn a_signature_separates_direction_from_hazards() {
        // `signature` packs two things into one number, so the check it feeds
        // is only as good as the packing: two entries differing only in
        // direction, or only in hazards, must not collide.
        let ltr_joining =
            signature(Direction::LeftToRight, HazardSet::of(&[Hazard::ContextualJoining]));
        let rtl_joining =
            signature(Direction::RightToLeft, HazardSet::of(&[Hazard::ContextualJoining]));
        let rtl_bare = signature(Direction::RightToLeft, HazardSet::NONE);
        assert!(!all_distinct_keys(&[rtl_joining, rtl_joining]));
        assert!(all_distinct_keys(&[ltr_joining, rtl_joining, rtl_bare]));
    }

    #[test]
    fn a_hazard_has_a_bit_of_its_own() {
        // `bit` is derived from `index`, so this is checking the derivation
        // rather than a table: a hand-assigned bitset goes wrong by giving two
        // hazards one bit, and two hazards sharing a bit would make
        // `every_hazard_is_exercised` pass for a hazard nothing brings.
        let mut seen = 0u16;
        for hazard in Hazard::ALL {
            assert_eq!(seen & hazard.bit(), 0, "{} reuses a bit", hazard.name());
            seen |= hazard.bit();
        }
        assert_eq!(seen.count_ones() as usize, Hazard::COUNT, "a hazard lost its bit");
        assert!(HazardSet::NONE.is_empty());
        assert_eq!(HazardSet::of(&Hazard::ALL).len() as usize, Hazard::COUNT);
    }

    #[test]
    fn a_name_answers_with_its_own_entry_and_nothing_else_does() {
        for script in Script::ALL {
            assert_eq!(Script::from_name(script.name()), Some(script));
            assert_eq!(Script::ALL[script.index()], script);
        }
        // The negative case is a string no list here can ever adopt, because
        // every name on every list is asserted to be `is_a_name` and this is
        // not one. Naming a script the corpus happens to lack — `"cyrillic"` —
        // would make this test go red the day somebody argues that script onto
        // the list, which is precisely the diff-to-a-test this file exists to
        // make impossible.
        assert!(!is_a_name(NOT_A_NAME));
        assert!(Script::from_name(NOT_A_NAME).is_none());
        assert!(Script::from_name("").is_none());
        for direction in Direction::ALL {
            assert_eq!(Direction::from_name(direction.name()), Some(direction));
        }
        for hazard in Hazard::ALL {
            assert_eq!(Hazard::from_name(hazard.name()), Some(hazard));
        }
    }
}
