// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The shaping cache: what names a shaped run, and what the cache is allowed to
//! forget.
//!
//! # The trap this module is written around
//!
//! `E3-B03c`'s exit names it: *from one seed the hit and miss counts are
//! identical across runs and across architectures, which is the property a
//! cache keyed by a hashed string loses silently.* The word doing the work is
//! **silently**. A cache whose key is a hash of the run has two separate ways to
//! stop being reproducible and neither of them shows up as a wrong pixel:
//!
//! 1. The hasher's own seed. The standard library's map draws one per process,
//!    which is exactly why RFC 0004 forbids that map here — two runs of one seed
//!    then walk the same buckets in different orders, evict different entries,
//!    and report different hit counts for identical work.
//! 2. A **collision**. If two distinct runs can map to one key, the cache hands
//!    back the wrong shaping *and the hit count is still right*. No count
//!    catches it. This is the failure the exit cannot be written to detect, so
//!    it is designed out rather than tested for, and [`Key`] is where.
//!
//! # The key is the thing itself, and is therefore injective
//!
//! [`Key`] stores the run's bytes, the face's address, the grid, the em and the
//! feature set **verbatim**. It is not a digest of them. Two keys are equal
//! exactly when all five are, so the map from (face, grid, em, features, run) to
//! `Key` is injective by construction and there is no collision to price.
//!
//! Three consequences, each of which is a rule in the code below rather than a
//! hope:
//!
//! - **A run past [`TEXT_BYTES_MAX`] is refused, never truncated.** Truncating
//!   is precisely how a verbatim key stops being injective: two runs sharing a
//!   prefix would become one key, and the wrong shaping would come back with the
//!   hit count intact. [`NotCacheable::RunTooLong`] is the alternative, and a
//!   caller that gets it shapes without the cache — the bound decides *what is
//!   cached* and never *what is correct*.
//! - **The tail of the stored run is zeroed.** A fixed-size array carries
//!   whatever was in it; if the bytes past the run's length were left alone, the
//!   residue of a previous key would be part of this one's identity, and one run
//!   would be two keys depending on where it was built. That is the same defect
//!   as a collision with the sign flipped — a miss that never resolves — and it
//!   is just as invisible to a reader of the code.
//! - **The ordering is total and is a function of the key alone.** `Ord` is
//!   derived over the fields in declaration order. Which order that is does not
//!   matter and is deliberately not argued; what matters is that it depends on
//!   nothing but the key, so the table below is the same table on both
//!   architectures and under any arrival order.
//!
//! The one digest anywhere near this module is the face's content address, which
//! is computed by `f-hash` somewhere else and arrives here as thirty-two opaque
//! bytes. **This module never hashes anything**, and that is why the
//! no-dependency row in this crate's manifest survives the cache that crate
//! comment predicted would break it. A collision *there* would already be a
//! different face answering to a declared address, which is
//! `kernel/src/objects.rs`'s problem and RFC 0012's *one identity*; the cache
//! neither adds to that risk nor is the layer that could detect it.
//!
//! # Why the grid is in the key although the address already decides it
//!
//! A face's bytes carry its units-per-em, so a key carrying both the address and
//! the grid carries one fact twice, and a field that is a function of another
//! field is a field two callers can disagree about. It is here anyway, and the
//! reason is the shape of the two failures rather than tidiness. If the grid
//! were left out, a caller that shaped a face at a grid those bytes do not state
//! would store a wrong shaping under a key that looks right, and every later
//! lookup would return it — a silent wrong answer. With the grid in the key that
//! same caller gets a **miss it never resolves**, the hit count falls to zero,
//! and the test this task is exited on is the thing that notices. Given a choice
//! between a redundant field and an undetectable one, this module takes the
//! redundancy and says so.
//!
//! *What would reverse it:* a face type that hands out its grid, so that the key
//! could derive the grid from the face rather than be told it. Then the two
//! cannot disagree and the field is only redundant.
//!
//! # Why there is no shaping type here
//!
//! [`Cache`] is generic over its value and this crate ships nothing to put in
//! it. RFC 0082 put the shaper behind the licence boundary where this crate
//! cannot link it, and [`crate::corpus`] already refuses to write down expected
//! outputs for the same reason: a shaped-run type invented today would be this
//! tree asserting what its first shaper happens to produce, which is the fixture
//! that makes every later shaper wrong by definition. What this module decides
//! is **identity and residency**. What a shaping *is* belongs to whoever shapes.
//!
//! # Determinism
//!
//! There is no map here at all — the crate is `no_std`, so the two types RFC
//! 0004 names are not in scope and could not be reached for by habit. What
//! replaces them is an ordered table in a slice the caller owns, searched by
//! bisection over [`Key`]'s total order. Two runs that perform the same
//! operations reach the same table, in the same slots, with the same counters,
//! on either architecture: every comparison is over bytes and integers, and
//! nothing here divides, rounds or reads a clock.

use core::cmp::Ordering;

/// Bytes in a face's content address.
///
/// Thirty-two, because the one digest in this tree is SHA-256 and `f-hash` is
/// where it lives. The number is here rather than imported because importing it
/// would be a dependency row, and this crate's whole argument for having none is
/// that the cache is the place a dependency would smuggle a hasher in. A second
/// spelling of a constant is a real cost and it is the smaller one: this is
/// thirty-two bytes of opaque identity, and a build in which it were not would
/// fail at every call site that passes an address.
/// Unit: bytes.
pub const ADDRESS_BYTES: usize = 32;

/// The longest run this cache will hold, in bytes of UTF-8.
///
/// A hundred and twenty-eight. It is **declared, not derived**, which is RFC
/// 0101's rule applied before it can bite: a bound computed from something
/// counted elsewhere — the longest sample in the corpus, the widest line a
/// display has — is a bound that goes wrong the day the thing counted stops
/// being the thing bounded, and goes wrong silently. Nothing in this module
/// counts runs, so there is nothing to derive it from and nothing to decay.
///
/// It is also the one bound here that **cannot be wrong in a way that matters**,
/// because a run past it is refused rather than truncated and the caller shapes
/// it without the cache. Too small costs hit rate; too large costs bytes per
/// slot. Neither costs a correct answer, which is why the number is allowed to
/// be a judgement at all.
/// Unit: bytes.
pub const TEXT_BYTES_MAX: usize = 128;

/// A shaping feature this build knows how to be asked for.
///
/// A closed set, and the closure is the point. An arbitrary four-byte tag would
/// make the feature half of a key variable-length — which a fixed-size key
/// cannot store, so it would have to be summarised, and a summary of a set is a
/// digest with all of the collision problem this module exists to avoid. Worse,
/// a feature this build cannot be asked for cannot change a shaping in this
/// build, so admitting an unknown tag into the key would create two keys that
/// must name one shaping; and *dropping* an unknown tag instead would create one
/// key naming two. There is no third option, so the set is closed and a feature
/// that is not here does not exist yet.
///
/// *What would reverse it:* a shaper whose feature list is open, reached over a
/// ring under RFC 0003. Then the tags are that shaper's vocabulary, the key's
/// feature half becomes whatever it says a request is, and this enum goes rather
/// than being kept beside it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Feature {
    /// `kern`: pair positioning. The feature whose absence is most often
    /// mistaken for a broken face.
    Kerning,
    /// `liga`: the ligatures a reader expects and does not notice.
    StandardLigatures,
    /// `rlig`: the ligatures a script requires, where turning them off is not a
    /// style choice but a misspelling.
    RequiredLigatures,
    /// `calt`: substitutions that depend on neighbours — the feature that makes
    /// [`crate::corpus::Hazard::ContextualJoining`] a shaping problem rather
    /// than a font problem.
    ContextualAlternates,
    /// `mark`: attaching a mark to its base, which is where
    /// [`crate::corpus::Hazard::CombiningMarks`] stops being a count of
    /// characters.
    MarkPositioning,
}

impl Feature {
    /// Every feature, in one list, because a caller that wants to sweep them has
    /// to get them from somewhere and a second hand-written list is a list that
    /// drifts.
    pub const ALL: [Self; 5] = [
        Self::Kerning,
        Self::StandardLigatures,
        Self::RequiredLigatures,
        Self::ContextualAlternates,
        Self::MarkPositioning,
    ];

    /// This feature's position, and therefore its bit.
    #[must_use]
    pub const fn index(self) -> u32 {
        match self {
            Self::Kerning => 0,
            Self::StandardLigatures => 1,
            Self::RequiredLigatures => 2,
            Self::ContextualAlternates => 3,
            Self::MarkPositioning => 4,
        }
    }

    /// The OpenType tag this feature is spelled with wherever anybody else
    /// writes it down.
    #[must_use]
    pub const fn tag(self) -> &'static str {
        match self {
            Self::Kerning => "kern",
            Self::StandardLigatures => "liga",
            Self::RequiredLigatures => "rlig",
            Self::ContextualAlternates => "calt",
            Self::MarkPositioning => "mark",
        }
    }

    /// This feature's bit in a [`FeatureSet`].
    ///
    /// Derived from [`Feature::index`] rather than written on the variant, for
    /// the reason `crate::corpus::Hazard::bit` gives: a hand-assigned bitset
    /// goes wrong by giving two members one bit, and it goes wrong silently, by
    /// making them indistinguishable to everything downstream — which here means
    /// two feature sets that are one key.
    #[must_use]
    pub const fn bit(self) -> u16 {
        1u16 << self.index()
    }
}

/// Which features a run is to be shaped with.
///
/// A bitset over a closed set, which makes it fixed-width, totally ordered, and
/// injective on the set it represents — the three properties a key half needs
/// and the three an arbitrary tag list does not have.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct FeatureSet(u16);

impl FeatureSet {
    /// No features at all, which is a real request and not an absence: it is
    /// what a shaper is asked for when a caller wants the face's glyphs and
    /// none of its opinions.
    pub const NONE: Self = Self(0);

    /// A set from a list, duplicates and order both irrelevant — which is the
    /// property that makes this injective on *sets* rather than on lists.
    #[must_use]
    pub const fn of(features: &[Feature]) -> Self {
        let mut bits = 0u16;
        let mut at = 0;
        while at < features.len() {
            bits |= features[at].bit();
            at += 1;
        }
        Self(bits)
    }

    /// Is this feature in the set?
    #[must_use]
    pub const fn contains(self, feature: Feature) -> bool {
        self.0 & feature.bit() != 0
    }

    /// Is the set empty?
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// How many features are in it.
    #[must_use]
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// The bits, for a caller that has to put them on a wire.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }
}

/// Why a run cannot be named by a [`Key`].
///
/// Two variants and no third, because there are exactly two things a key can
/// disbelieve: a run it cannot store whole, and a size at which no
/// [`crate::metric::Pen`] could exist. Both are refusals rather than
/// adjustments, for [`crate::metric::NotAMetric`]'s reason one module over —
/// there is no nearest legal answer to a run that is too long, only a shorter
/// run nobody asked about.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotCacheable {
    /// Longer than [`TEXT_BYTES_MAX`]. The caller shapes it without the cache;
    /// nothing is truncated and nothing is wrong, there is simply no key.
    RunTooLong,
    /// The grid and the em are not a size, as [`crate::metric::Scale`] decides
    /// it. The refusal it gave is carried rather than restated, because a second
    /// spelling of *which bound* is a second bound.
    Size(crate::metric::NotAMetric),
}

impl NotCacheable {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::RunTooLong => "the run is longer than a key can hold whole, and is not truncated",
            Self::Size(_) => "the grid and the em are not a size any pen could be set at",
        }
    }
}

/// What names one shaped run: a face, a grid, an em, a feature set and the run's
/// own bytes.
///
/// Every one of them stored as it was given. See this module's header for why
/// none of it is hashed, why the run is refused rather than truncated past
/// [`TEXT_BYTES_MAX`], and why the tail is zeroed.
///
/// It is `Copy` and about two hundred bytes. That is deliberate and is the price
/// of the injectivity argument: a borrowed key would be smaller and would have a
/// lifetime, and the table below would then hold a reference into whatever
/// buffer the run arrived in — which is the one thing a cache must not do,
/// because the entry outlives the call that made it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Key {
    /// The face's content address, as `f-hash` computed it somewhere else.
    face: [u8; ADDRESS_BYTES],
    /// The design grid the face states. Unit: design units per em.
    upem: u16,
    /// The em this run is set at. Unit: sixty-fourths of a pixel.
    em_px_x64: i32,
    /// Which features the shaper is being asked for.
    features: FeatureSet,
    /// How many of the bytes below are the run. Unit: bytes.
    text_len: u16,
    /// The run, and zeroes to the end.
    text: [u8; TEXT_BYTES_MAX],
}

impl Key {
    /// Name a run, or say why it cannot be named.
    ///
    /// # Errors
    ///
    /// [`NotCacheable::RunTooLong`] past [`TEXT_BYTES_MAX`], and
    /// [`NotCacheable::Size`] for a grid and an em that are not a size. The
    /// second is decided by [`crate::metric::Scale::new`] and not here: that
    /// module says its bounds are checked in one place, and a key that checked
    /// them again would be the second place. What the key keeps is the pair;
    /// what `Scale` contributes is the refusal.
    pub fn new(
        face: [u8; ADDRESS_BYTES],
        upem: u16,
        em_px_x64: i32,
        features: FeatureSet,
        text: &str,
    ) -> Result<Self, NotCacheable> {
        if text.len() > TEXT_BYTES_MAX {
            return Err(NotCacheable::RunTooLong);
        }
        match crate::metric::Scale::new(upem, em_px_x64) {
            Ok(_) => {}
            Err(why) => return Err(NotCacheable::Size(why)),
        }
        // Zeroed first and filled second, so the bytes past the run are a
        // function of nothing. See the header: a residue here is a run that is
        // two keys.
        let mut bytes = [0u8; TEXT_BYTES_MAX];
        bytes[..text.len()].copy_from_slice(text.as_bytes());
        Ok(Self {
            face,
            upem,
            em_px_x64,
            features,
            // Fits: `text.len()` is at most `TEXT_BYTES_MAX`, checked above, and
            // that constant is far inside a `u16`. The cast is narrowing and is
            // written rather than inferred so the bound it rests on is named.
            text_len: text.len() as u16,
            text: bytes,
        })
    }

    /// The run, as it was given.
    ///
    /// The bytes came out of a `&str` and were copied whole, so they are UTF-8
    /// and this never takes the fallback. It is `unwrap_or` and not `expect`
    /// because a cache that panics is worse than a cache that answers with an
    /// empty run: the caller is in the middle of laying out a line, and a panic
    /// is a failure nothing in the tree can act on.
    #[must_use]
    pub fn text(&self) -> &str {
        core::str::from_utf8(&self.text[..self.text_len as usize]).unwrap_or("")
    }

    /// The face's address.
    #[must_use]
    pub const fn face(&self) -> &[u8; ADDRESS_BYTES] {
        &self.face
    }

    /// Which features this run is to be shaped with.
    #[must_use]
    pub const fn features(&self) -> FeatureSet {
        self.features
    }
}

impl core::fmt::Debug for Key {
    /// The run, the features and the front of the address — and deliberately not
    /// the grid and the em.
    ///
    /// Hand-written for [`crate::metric::Scale`]'s reason: the derive would
    /// print a hundred and twenty-eight bytes of mostly zeroes, and it would
    /// also print `upem` beside `em_px_x64`, which together are every number a
    /// consumer needs to do its own rounding. That module declines to hand the
    /// pair back out of a `Scale`, and a `Debug` here that printed it would be
    /// the way around that decision rather than a convenience.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Key")
            .field("text", &self.text())
            .field("features", &self.features.bits())
            .field("face", &&self.face[..4])
            .finish_non_exhaustive()
    }
}

/// One resident shaping: what named it, what it is, and when it was last wanted.
struct Entry<V> {
    key: Key,
    value: V,
    /// The tick of the most recent lookup or insert that touched this entry.
    /// Unit: operations.
    used_at: u64,
}

/// One place in the table a caller lends the cache.
///
/// The cache allocates nothing — this crate is `no_std` and the heap arrives in
/// `ring/` later — so residency is a slice of these and the caller decides how
/// many. [`Slot::EMPTY`] exists so that the slice can be written
/// `[Slot::EMPTY; N]` for any value type at all, including one with no sensible
/// default; that is the whole reason the entry is an `Option` rather than the
/// fields inline.
pub struct Slot<V> {
    entry: Option<Entry<V>>,
}

impl<V> Slot<V> {
    /// A slot holding nothing, and the only way to make one.
    pub const EMPTY: Self = Self { entry: None };
}

impl<V> Default for Slot<V> {
    fn default() -> Self {
        Self::EMPTY
    }
}

/// What the cache did with an insert.
///
/// Four answers and every one of them countable, which is what `E3-B03c`'s exit
/// is about: a cache whose eviction cannot be counted is a cache whose residency
/// is an opinion.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Admission {
    /// It went into a free slot. Nothing left.
    Placed,
    /// An entry with this exact key was already resident and its value was
    /// overwritten. Residency is unchanged — this is the arm a caller reaches
    /// when it re-shapes a run it had already shaped, which is not an error and
    /// is worth being able to count separately from a fresh placement.
    Replaced,
    /// The table was full, the least recently used entry was removed, and this
    /// one took its place.
    Evicted,
    /// There was nowhere to put it, which happens only for a cache with no slots
    /// at all. That is a legal configuration — it is how a caller turns caching
    /// off without changing a call site — so it is an answer and not a panic.
    NoRoom,
}

/// Every number this cache will admit to, and the whole of what `E3-B03c` is
/// exited on.
///
/// Six counters rather than the two the exit names, because *hits and misses are
/// identical across runs* is only interesting if the things that move them are
/// visible too. A cache whose hit count reproduced while its eviction count did
/// not would satisfy the sentence and not the property.
///
/// They saturate rather than wrap. A counter that wrapped would make a long run
/// report a smaller number than a short one, and the one thing these are for is
/// being compared between two runs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    /// Lookups that found a resident entry. Unit: operations.
    pub hits: u64,
    /// Lookups that did not. Unit: operations.
    pub misses: u64,
    /// Inserts that put a value in the table, whether fresh, after an eviction,
    /// or over an entry with the same key. Unit: operations.
    pub admissions: u64,
    /// The subset of those that overwrote an entry with the same key.
    /// Unit: operations.
    pub replacements: u64,
    /// Entries removed to make room. Unit: entries.
    pub evictions: u64,
    /// Inserts that could not be honoured, which is [`Admission::NoRoom`].
    /// Unit: operations.
    pub refused: u64,
}

/// A fixed-residency cache from [`Key`] to whatever a shaper produces.
///
/// # The table
///
/// The occupied slots are a prefix of the slice, held **sorted by [`Key`]**, and
/// a lookup bisects them. Sorted rather than merely scanned, and the reason is
/// not speed: a table kept in arrival order is a table whose layout is a
/// function of the order things arrived, so two runs that cache the same set of
/// runs in different orders hold different tables and evict differently. A
/// sorted table is a function of the key *set*, which is one fewer thing that
/// has to be reproduced for the counts to reproduce.
///
/// # The residency bound
///
/// `slots.len()`, and it is the caller's to choose because the caller owns the
/// memory. That is also the only bound in this module that is not derived from
/// anything at all, which is RFC 0101's rule taken seriously: there is no count
/// elsewhere in the tree that decides how many shaped runs are worth keeping,
/// so a number derived from one would be a number that is wrong as soon as the
/// thing counted stops being the thing bounded.
///
/// # What is evicted, and why it is countable
///
/// The least recently used entry, where *recently* is [`Cache`]'s own tick: a
/// counter incremented on every [`Cache::get`] and every [`Cache::insert`], and
/// stamped on the entry each one touches. Every eviction increments
/// [`Counts::evictions`], so residency is never something a reader has to infer.
///
/// **The tick is not a clock, and this is the sharpest objection to the module.**
/// RFC 0004 says nothing observes time or ordering except through `f_env::Env`.
/// What `Env` is for is the *environment's* choices — a clock reading, a die
/// roll, which of two concurrent tasks got there first — because those are what
/// two runs of one seed would otherwise differ in. This counter observes the
/// caller's own call sequence, which is not the environment's choice but the
/// program's: two runs of one program call in the same order by construction,
/// and if they do not then the divergence is upstream and these counters are how
/// it becomes visible rather than how it is hidden.
///
/// *What would reverse that:* a cache shared between two components, or between
/// two cores. Then the interleaving of the calls is the environment's choice
/// after all, the tick becomes an observation of it, and the ordering has to
/// come from whatever orders those two — which in this tree is `f_env::Env` and
/// nothing else. A shared shaping cache is a plausible thing to want, so this
/// paragraph is a condition and not a disclaimer.
///
/// At `u64::MAX` operations the tick saturates and every entry eventually shares
/// one stamp, at which point the policy degenerates to *evict the least key* —
/// still total, still reproducible, still counted. It is priced rather than
/// repaired: the repair is a renumbering pass over every entry, paid on some
/// operation nobody can predict, to protect a case that needs more lookups than
/// this machine has instructions.
pub struct Cache<'a, V> {
    slots: &'a mut [Slot<V>],
    /// How many of the leading slots are occupied, and they are sorted.
    len: usize,
    /// Operations so far. Unit: operations.
    tick: u64,
    counts: Counts,
}

impl<'a, V> Cache<'a, V> {
    /// A cache over slots the caller owns.
    ///
    /// The slots are taken as they are and are not inspected: an entry left in
    /// one by a previous cache is not adopted, because `len` starts at zero. A
    /// caller that wants the contents back does not get them, which is the
    /// correct answer for a cache and is why there is no way to build one over a
    /// table somebody else filled.
    #[must_use]
    pub fn new(slots: &'a mut [Slot<V>]) -> Self {
        Self { slots, len: 0, tick: 0, counts: Counts::default() }
    }

    /// How many entries this cache may hold. Unit: entries.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.slots.len()
    }

    /// How many it holds. Unit: entries.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Does it hold nothing?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every number this cache will admit to.
    #[must_use]
    pub const fn counts(&self) -> Counts {
        self.counts
    }

    /// The resident keys, in the table's own order — which is [`Key`]'s order
    /// and not the order they arrived.
    ///
    /// This exists for the test that the table is a function of the key set
    /// rather than of arrival order, and it is public because that property is
    /// the module's claim and a claim only a private test can check is a claim a
    /// consumer has to take on trust.
    pub fn keys(&self) -> impl Iterator<Item = &Key> {
        self.slots[..self.len].iter().filter_map(|slot| slot.entry.as_ref()).map(|entry| &entry.key)
    }

    /// Where `key` is, or where it would go.
    ///
    /// A slot inside the occupied prefix that holds nothing cannot happen —
    /// `len` is maintained against exactly that — and it is answered as
    /// `Greater` rather than unwrapped, because the cost of being wrong about
    /// the invariant should be a miss and not a panic in the middle of a line
    /// of text.
    fn search(&self, key: &Key) -> Result<usize, usize> {
        let mut low = 0usize;
        let mut high = self.len;
        while low < high {
            let mid = low + (high - low) / 2;
            let ordering = match self.slots[mid].entry.as_ref() {
                Some(entry) => entry.key.cmp(key),
                None => Ordering::Greater,
            };
            match ordering {
                Ordering::Less => low = mid + 1,
                Ordering::Equal => return Ok(mid),
                Ordering::Greater => high = mid,
            }
        }
        Err(low)
    }

    /// Which entry goes when the table is full.
    ///
    /// The smallest `(used_at, key)` pair. The key is in the comparison as a
    /// tie-break that cannot be reached while the tick still increments — no two
    /// entries can share a stamp before it saturates — and it is there anyway,
    /// because *after* saturation the choice has to remain a function of the
    /// table rather than of which slot happened to be scanned first.
    fn victim(&self) -> Option<usize> {
        let mut best: Option<(usize, u64, &Key)> = None;
        for index in 0..self.len {
            let Some(entry) = self.slots[index].entry.as_ref() else { continue };
            let take = match best {
                None => true,
                Some((_, at, key)) => (entry.used_at, &entry.key) < (at, key),
            };
            if take {
                best = Some((index, entry.used_at, &entry.key));
            }
        }
        best.map(|(index, _, _)| index)
    }

    /// Take the entry at `index` out and close the gap, keeping the prefix
    /// sorted.
    fn remove(&mut self, index: usize) {
        self.slots[index..self.len].rotate_left(1);
        self.slots[self.len - 1].entry = None;
        self.len -= 1;
    }

    /// The shaping this key names, if the cache still has it.
    ///
    /// A hit stamps the entry, so lookups are what keep an entry resident. That
    /// makes this `&mut self`, which is not an inconvenience to route around: a
    /// cache whose reads did not affect residency would evict by insertion age,
    /// and insertion age is a worse predictor of the next lookup than the last
    /// lookup is.
    pub fn get(&mut self, key: &Key) -> Option<&V> {
        self.tick = self.tick.saturating_add(1);
        let at = self.tick;
        match self.search(key) {
            Ok(index) => match self.slots[index].entry.as_mut() {
                Some(entry) => {
                    entry.used_at = at;
                    self.counts.hits = self.counts.hits.saturating_add(1);
                    Some(&entry.value)
                }
                None => {
                    self.counts.misses = self.counts.misses.saturating_add(1);
                    None
                }
            },
            Err(_) => {
                self.counts.misses = self.counts.misses.saturating_add(1);
                None
            }
        }
    }

    /// Put a shaping in, evicting the least recently used entry if the table is
    /// full.
    ///
    /// The answer says which of the four things happened, and each one moves a
    /// counter. A caller that ignores the answer still gets a correct cache;
    /// what it loses is the ability to say why its hit rate is what it is, which
    /// is the whole reason [`Admission`] is returned rather than discarded.
    pub fn insert(&mut self, key: &Key, value: V) -> Admission {
        self.tick = self.tick.saturating_add(1);
        let at = self.tick;
        if self.slots.is_empty() {
            self.counts.refused = self.counts.refused.saturating_add(1);
            return Admission::NoRoom;
        }
        match self.search(key) {
            Ok(index) => {
                self.slots[index].entry = Some(Entry { key: *key, value, used_at: at });
                self.counts.admissions = self.counts.admissions.saturating_add(1);
                self.counts.replacements = self.counts.replacements.saturating_add(1);
                Admission::Replaced
            }
            Err(mut place) => {
                let mut evicted = false;
                if self.len == self.slots.len() {
                    let Some(victim) = self.victim() else {
                        // Unreachable while the invariant holds — a full table
                        // has an occupant — and a refusal rather than a panic
                        // for `search`'s reason.
                        self.counts.refused = self.counts.refused.saturating_add(1);
                        return Admission::NoRoom;
                    };
                    self.remove(victim);
                    self.counts.evictions = self.counts.evictions.saturating_add(1);
                    evicted = true;
                    // Removing before `place` shifts the insertion point down
                    // with it. Getting this wrong puts the table out of order,
                    // which a bisecting lookup reports as a miss rather than as
                    // a fault — so the table's order is asserted by a test and
                    // not only by this line.
                    if victim < place {
                        place -= 1;
                    }
                }
                self.slots[self.len].entry = Some(Entry { key: *key, value, used_at: at });
                self.slots[place..=self.len].rotate_right(1);
                self.len += 1;
                self.counts.admissions = self.counts.admissions.saturating_add(1);
                if evicted { Admission::Evicted } else { Admission::Placed }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::corpus::Script;

    /// A face address and the grid the face at it would state.
    ///
    /// Fabricated rather than computed, and that is the point: computing one
    /// would mean hashing, hashing means `f-hash`, and a dependency row on the
    /// crate that holds the tree's one digest is exactly what this crate's
    /// manifest declines. An address here is thirty-two opaque bytes because
    /// that is all the cache is ever handed.
    const FACES: [([u8; ADDRESS_BYTES], u16); 2] =
        [([0x11; ADDRESS_BYTES], 1_000), ([0x22; ADDRESS_BYTES], 2_048)];

    /// Four ems, and the last one is not a whole pixel — eleven and a half of
    /// them, in sixty-fourths — because a size that divides evenly is the size
    /// at which a key confusion between two ems is least likely to show.
    /// Unit: sixty-fourths of a pixel.
    const EMS_X64: [i32; 4] = [12 * 64, 16 * 64, 24 * 64, 11 * 64 + 32];

    /// Three feature sets: none, one, and three — so that a key half that was
    /// dropped, and one that collapsed every non-empty set together, are two
    /// different failures rather than one.
    const FEATURES: [FeatureSet; 3] = [
        FeatureSet::NONE,
        FeatureSet::of(&[Feature::Kerning]),
        FeatureSet::of(&[Feature::Kerning, Feature::StandardLigatures, Feature::MarkPositioning]),
    ];

    /// The spans of a sample this workload asks for, as (characters skipped,
    /// characters taken).
    ///
    /// A small fixed set rather than two more draws, because a working set is
    /// what a cache is for and a run drawn freshly from a wide range is a cache
    /// that never hits. The last entry is the one that decides whether the
    /// refusal is exercised at all: sixty characters is under
    /// [`TEXT_BYTES_MAX`] in Latin and past it in every multi-byte script here,
    /// so the same span is cached for one script and refused for another — which
    /// makes the refusal count a number about the corpus rather than a constant.
    const SPANS: [(usize, usize); 6] = [(0, 3), (0, 6), (2, 4), (3, 8), (5, 5), (0, 60)];

    /// Bytes in the paragraph a run is cut out of. Unit: bytes.
    ///
    /// **No span of any sample in [`crate::corpus`] reaches
    /// [`TEXT_BYTES_MAX`]** — the longest entry is a greeting, and that module
    /// says in as many words that what it holds is the *input* half and not a
    /// document. A workload that only sliced samples could therefore never
    /// produce a [`NotCacheable::RunTooLong`], and that branch is the one
    /// standing between a bound and a truncation: the whole injectivity argument
    /// rests on it. So the sample is repeated into a paragraph first, which is
    /// also the more honest subject — a shaping run is a span of a paragraph,
    /// and a greeting is not one.
    const PARAGRAPH_BYTES: usize = 512;

    /// Fill `into` with as many whole copies of `sample` as fit, and say how
    /// many bytes that was.
    ///
    /// Whole copies only, so the result is still UTF-8 by construction rather
    /// than by a check that could be wrong.
    fn paragraph(sample: &str, into: &mut [u8; PARAGRAPH_BYTES]) -> usize {
        let mut at = 0;
        while at + sample.len() <= PARAGRAPH_BYTES {
            into[at..at + sample.len()].copy_from_slice(sample.as_bytes());
            at += sample.len();
        }
        at
    }

    /// How many operations one workload performs. Unit: operations.
    ///
    /// Two thousand, chosen so that the table fills, evicts, and is still hit
    /// often enough that the hit count is a number rather than a rounding of
    /// zero. A workload that never evicted would exit this task on a cache whose
    /// residency policy had never run.
    const OPERATIONS: usize = 2_000;

    /// The seed the written-down counts below belong to.
    const SEED: u64 = 0x5f3d_1c27_a91e_0b64;

    /// A seeded integer generator, and the reason it is written here rather
    /// than taken from somewhere.
    ///
    /// `f_env::Env` is where this tree draws randomness, and this crate has no
    /// dependencies — which is a decision its manifest argues and which a test
    /// may not quietly reverse. What is needed here is weaker than `Env`
    /// anyway: not a source of randomness but a *fixed sequence*, the same one
    /// on both architectures, so that the workload below is a program rather
    /// than a sample. Every operation in it is a wrapping integer operation,
    /// which `u64` defines identically everywhere; there is no float in it, and
    /// nothing in it is read from the machine.
    struct Seeded {
        state: u64,
    }

    impl Seeded {
        const fn from_seed(seed: u64) -> Self {
            Self { state: seed }
        }

        /// The next word. SplitMix64, whose whole recommendation here is that it
        /// is four lines and has no state a reader has to reason about.
        fn draw(&mut self) -> u64 {
            self.state = self.state.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.state;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            z ^ (z >> 31)
        }

        /// A number below `bound`, modulo bias and all: the bias is a property
        /// of the sequence and is therefore reproduced too, which is the only
        /// thing being asked of it.
        fn below(&mut self, bound: usize) -> usize {
            if bound == 0 { 0 } else { (self.draw() % bound as u64) as usize }
        }

        /// A number below `bound`, skewed towards the small ones.
        ///
        /// The smaller of two draws, which is the crudest possible way to get a
        /// working set — and a working set is what makes the hit count mean
        /// anything. A uniform draw over a key space of a few thousand with a
        /// table of thirty-two would report a hit rate that measures the table
        /// size and nothing else; real text re-asks for a handful of runs far
        /// more often than the rest, and it is that re-asking a cache exists
        /// for. The exact shape of the skew is irrelevant and is deliberately
        /// not argued: what the workload needs is locality and reproducibility,
        /// and two draws and a minimum give both.
        fn skewed(&mut self, bound: usize) -> usize {
            let first = self.below(bound);
            let second = self.below(bound);
            if first < second { first } else { second }
        }
    }

    /// A span of a sample, cut at character boundaries.
    ///
    /// Character boundaries rather than byte offsets, because most of this
    /// corpus is multi-byte and slicing a `&str` through a scalar is a panic.
    /// Cutting by characters is also what a run *is*: a shaping run is a span of
    /// a paragraph, not a window over its encoding.
    fn span(sample: &str, skip_chars: usize, take_chars: usize) -> &str {
        let start = match sample.char_indices().nth(skip_chars) {
            Some((at, _)) => at,
            None => sample.len(),
        };
        let rest = &sample[start..];
        let end = match rest.char_indices().nth(take_chars) {
            Some((at, _)) => start + at,
            None => sample.len(),
        };
        &sample[start..end]
    }

    /// A stand-in for a shaping: a function of everything the key names and of
    /// nothing else.
    ///
    /// This is what makes the workload more than a counter. If the key were not
    /// injective — a truncated run, a dropped feature set, a grid left out — two
    /// different requests would share an entry, and the hit count would still be
    /// right. The value would not be, and the workload asserts on it.
    fn stamp(face: usize, upem: u16, em_px_x64: i32, features: u16, text: &str) -> u32 {
        let mut acc: u32 = 2_166_136_261;
        let mut eat = |byte: u8| {
            acc = (acc ^ u32::from(byte)).wrapping_mul(16_777_619);
        };
        for byte in text.as_bytes() {
            eat(*byte);
        }
        eat(face as u8);
        for byte in upem.to_le_bytes() {
            eat(byte);
        }
        for byte in em_px_x64.to_le_bytes() {
            eat(byte);
        }
        for byte in features.to_le_bytes() {
            eat(byte);
        }
        acc
    }

    /// One workload. Returns how many runs the key declined to name, which is a
    /// number the cache itself never sees.
    fn workload(cache: &mut Cache<'_, u32>, seed: u64) -> u64 {
        let mut rng = Seeded::from_seed(seed);
        let mut buffer = [0u8; PARAGRAPH_BYTES];
        let mut too_long = 0u64;
        for _ in 0..OPERATIONS {
            let script = Script::ALL[rng.skewed(Script::COUNT)];
            let filled = paragraph(script.sample(), &mut buffer);
            let text = core::str::from_utf8(&buffer[..filled])
                .expect("whole copies of a sample are still what the sample was");
            let (skip, take) = SPANS[rng.skewed(SPANS.len())];
            let run = span(text, skip, take);
            let face_index = rng.below(FACES.len());
            let (face, upem) = FACES[face_index];
            let em_px_x64 = EMS_X64[rng.skewed(EMS_X64.len())];
            let features = FEATURES[rng.skewed(FEATURES.len())];
            let key = match Key::new(face, upem, em_px_x64, features, run) {
                Ok(key) => key,
                Err(NotCacheable::RunTooLong) => {
                    too_long += 1;
                    continue;
                }
                Err(other) => panic!("the workload built a size no pen admits: {other:?}"),
            };
            let want = stamp(face_index, upem, em_px_x64, features.bits(), run);
            match cache.get(&key) {
                Some(found) => assert_eq!(
                    *found, want,
                    "a hit handed back a shaping named by some other key: {key:?}"
                ),
                None => {
                    cache.insert(&key, want);
                }
            }
        }
        too_long
    }

    fn table<const N: usize>() -> [Slot<u32>; N] {
        [Slot::<u32>::EMPTY; N]
    }

    /// One key spelled out field by field, with a name for the field that was
    /// changed.
    ///
    /// A named type rather than the tuple spelled at the use site, for the
    /// reason `crate::face::tests::Lie` is one: a signature a reader has to
    /// parse before they can read the cases is a signature in the way. Clippy
    /// refuses the inline spelling anyway, which is the same judgement arrived
    /// at mechanically.
    type Variant = (&'static str, [u8; ADDRESS_BYTES], u16, i32, FeatureSet, &'static str);

    fn key_of(text: &str) -> Key {
        Key::new(FACES[0].0, FACES[0].1, EMS_X64[1], FeatureSet::NONE, text)
            .expect("a short run at a legal size")
    }

    /// The exit's own sentence, run twice in one process.
    ///
    /// This is the weaker half and is labelled as such: two runs *here* share an
    /// architecture, so what it shows is that nothing in the cache carries state
    /// between runs. The across-architectures half is the test below it, whose
    /// numbers are literals in this file and are therefore checked by whichever
    /// runner compiles it — the arm job included.
    #[test]
    fn one_seed_produces_one_pair_of_counts_however_often_it_is_run() {
        let mut first_slots = table::<32>();
        let mut second_slots = table::<32>();
        let mut first = Cache::new(&mut first_slots);
        let mut second = Cache::new(&mut second_slots);
        let refused_first = workload(&mut first, SEED);
        let refused_second = workload(&mut second, SEED);
        assert_eq!(first.counts(), second.counts(), "one seed, two runs, two answers");
        assert_eq!(refused_first, refused_second);
        assert_eq!(first.len(), second.len());
    }

    /// The counts this seed produces, written down.
    ///
    /// **These literals are the across-architectures half of `E3-B03c`'s exit,
    /// and they are the only form that half can take in a file.** Two runs in
    /// one process share an architecture; what makes these numbers an
    /// architecture claim is that they are compiled and asserted by whichever
    /// runner builds this crate. `f-text` carries `host: None` in `xtask`'s
    /// portability table, so `cargo xtask test-host` runs this file on the arm
    /// runner as well as on x86-64, and a number that differed there would fail
    /// here and on that runner alone — which is exactly what the exit asks for
    /// and is a thing no machine without an AArch64 runner can observe.
    ///
    /// They are golden numbers, so they are kept honest by the identities
    /// beneath them, which are arithmetic rather than observation: every
    /// operation is a lookup or a refusal, every lookup is a hit or a miss,
    /// every miss inserts, and residency is what was admitted less what was
    /// replaced and evicted. A wrong golden number that still satisfied all of
    /// them would have to be a consistent rewrite of the whole workload.
    #[test]
    fn the_counts_this_seed_produces_are_written_down_here() {
        let mut slots = table::<32>();
        let mut cache = Cache::new(&mut slots);
        let refused = workload(&mut cache, SEED);
        let counts = cache.counts();
        assert_eq!(
            counts,
            Counts {
                hits: 160,
                misses: 1_821,
                admissions: 1_821,
                replacements: 0,
                evictions: 1_789,
                refused: 0,
            },
            "the counts moved; if the workload changed, move these numbers with it, and if it \
             did not, this is the failure this task exists to produce"
        );
        assert_eq!(
            refused, 19,
            "runs the key declined to name, which is the long span in a multi-byte script and \
             nothing else"
        );

        let lookups = counts.hits + counts.misses;
        assert_eq!(lookups + refused, OPERATIONS as u64, "every operation is one of three things");
        assert_eq!(counts.misses, counts.admissions, "a miss inserts, and only a miss does");
        assert_eq!(
            counts.admissions - counts.replacements - counts.evictions,
            cache.len() as u64,
            "residency is what went in, less what was overwritten and what was thrown out"
        );
        assert_eq!(cache.len(), cache.capacity(), "this workload fills the table");
    }

    /// A key confusion is caught by a value, because it cannot be caught by a
    /// count.
    ///
    /// Six keys that differ from a base in exactly one field each. If any field
    /// were left out of the key, two of these would be one entry, the second
    /// lookup would be a *hit*, and the hit count would be higher rather than
    /// wrong-looking. What fails instead is the value.
    #[test]
    fn keys_differing_in_one_field_are_six_different_entries() {
        let mut slots = table::<16>();
        let mut cache = Cache::new(&mut slots);
        let variants: [Variant; 6] = [
            ("the base", FACES[0].0, 1_000, 16 * 64, FeatureSet::NONE, "ratio"),
            ("another face", FACES[1].0, 1_000, 16 * 64, FeatureSet::NONE, "ratio"),
            ("another grid", FACES[0].0, 2_048, 16 * 64, FeatureSet::NONE, "ratio"),
            ("another em", FACES[0].0, 1_000, 24 * 64, FeatureSet::NONE, "ratio"),
            ("another feature set", FACES[0].0, 1_000, 16 * 64, FEATURES[1], "ratio"),
            ("another run", FACES[0].0, 1_000, 16 * 64, FeatureSet::NONE, "ration"),
        ];
        for (index, (what, face, upem, em, features, text)) in variants.iter().enumerate() {
            let key = Key::new(*face, *upem, *em, *features, text).expect("a legal key");
            assert!(cache.get(&key).is_none(), "{what} was already resident");
            cache.insert(&key, index as u32);
        }
        assert_eq!(cache.len(), variants.len(), "six keys, six entries");
        for (index, (what, face, upem, em, features, text)) in variants.iter().enumerate() {
            let key = Key::new(*face, *upem, *em, *features, text).expect("a legal key");
            assert_eq!(
                cache.get(&key).copied(),
                Some(index as u32),
                "{what} read back somebody else's shaping"
            );
        }
    }

    /// A run past the bound is refused, and two runs that share a prefix past it
    /// are both refused rather than both truncated into one key.
    #[test]
    fn a_run_past_the_bound_is_refused_rather_than_truncated() {
        let mut long = [0x61u8; TEXT_BYTES_MAX + 1];
        let first = core::str::from_utf8(&long).expect("ascii");
        assert_eq!(
            Key::new(FACES[0].0, FACES[0].1, EMS_X64[0], FeatureSet::NONE, first),
            Err(NotCacheable::RunTooLong)
        );
        long[TEXT_BYTES_MAX] = 0x62;
        let second = core::str::from_utf8(&long).expect("ascii");
        assert_eq!(
            Key::new(FACES[0].0, FACES[0].1, EMS_X64[0], FeatureSet::NONE, second),
            Err(NotCacheable::RunTooLong),
            "two runs that differ only past the bound are two refusals, not one key"
        );
        let at_the_bound = core::str::from_utf8(&long[..TEXT_BYTES_MAX]).expect("ascii");
        assert!(
            Key::new(FACES[0].0, FACES[0].1, EMS_X64[0], FeatureSet::NONE, at_the_bound).is_ok(),
            "the bound is inclusive"
        );
    }

    /// A run with a zero byte in it is not the shorter run without it.
    ///
    /// The zero fill past a run's length is what makes a key a function of the
    /// run alone, and the obvious way to get that wrong is to let the fill be
    /// the terminator. A `&str` may contain `U+0000`, so it cannot be, and the
    /// length is in the key for that reason.
    #[test]
    fn a_run_with_a_zero_byte_is_not_the_run_without_it() {
        assert_eq!(key_of("ab"), key_of("ab"), "one run, built twice, is one key");
        assert_ne!(key_of("ab"), key_of("ab\u{0}"), "the fill is not a terminator");
        assert_ne!(key_of("ab"), key_of("ab "), "and a trailing space is a different run");
    }

    /// The table is a function of the key set, not of the order they arrived.
    ///
    /// Sorted residency is what buys this, and it is worth a test because the
    /// alternative — a table in arrival order — passes every count assertion in
    /// this file while making two callers that cached the same runs in different
    /// orders evict different entries.
    #[test]
    fn the_table_is_the_same_whichever_order_the_keys_arrived_in() {
        let runs = ["alpha", "beta", "gamma", "delta", "epsilon", "zeta", "eta", "theta"];
        let mut forward_slots = table::<16>();
        let mut backward_slots = table::<16>();
        let mut forward = Cache::new(&mut forward_slots);
        let mut backward = Cache::new(&mut backward_slots);
        for run in runs {
            forward.insert(&key_of(run), 0);
        }
        for run in runs.iter().rev() {
            backward.insert(&key_of(run), 0);
        }
        // Length first and then bytes, because `text_len` is declared before
        // `text` and `Ord` is derived. **That this is the order is not the
        // claim** — a different field order would give a different sequence and
        // nothing in the module would change. The claim is the two assertions
        // below: both arrival orders reach *this* sequence, whichever it is.
        let ordered = ["eta", "beta", "zeta", "alpha", "delta", "gamma", "theta", "epsilon"];
        let forward_keys: [&str; 8] =
            core::array::from_fn(|at| forward.keys().nth(at).expect("eight resident").text());
        let backward_keys: [&str; 8] =
            core::array::from_fn(|at| backward.keys().nth(at).expect("eight resident").text());
        assert_eq!(forward_keys, ordered, "the table is not in the key's order");
        assert_eq!(backward_keys, ordered, "arrival order reached a different table");
    }

    /// What goes when the table is full is the entry nothing has asked for
    /// longest — and a lookup counts as asking.
    #[test]
    fn the_least_recently_used_entry_is_the_one_that_goes() {
        let mut slots = table::<3>();
        let mut cache = Cache::new(&mut slots);
        assert_eq!(cache.insert(&key_of("first"), 1), Admission::Placed);
        assert_eq!(cache.insert(&key_of("second"), 2), Admission::Placed);
        assert_eq!(cache.insert(&key_of("third"), 3), Admission::Placed);
        // Asking for the oldest is what saves it. Without this line the answer
        // below would be `first` either way, and the test would pass under a
        // policy that evicted by age of insertion instead.
        assert_eq!(cache.get(&key_of("first")).copied(), Some(1));
        assert_eq!(cache.insert(&key_of("fourth"), 4), Admission::Evicted);
        assert_eq!(cache.counts().evictions, 1, "one eviction, counted");
        assert_eq!(cache.get(&key_of("second")).copied(), None, "the least recent went");
        assert_eq!(cache.get(&key_of("first")).copied(), Some(1));
        assert_eq!(cache.get(&key_of("third")).copied(), Some(3));
        assert_eq!(cache.get(&key_of("fourth")).copied(), Some(4));
        assert_eq!(cache.len(), 3, "residency is the slice the caller lent and not one more");
    }

    /// Every entry the table still holds can be found, and the table is in
    /// order.
    ///
    /// This is the assertion that **names** an out-of-order table, and it was
    /// added because a mutation exposed that nothing else did. Getting the
    /// insertion point wrong after an eviction below it puts a key where a
    /// bisecting lookup will not look; sometimes that is a panic inside the shift
    /// itself, which is a failure three layers down that says nothing about the
    /// cache, and sometimes it is nothing at all — a resident entry that is a
    /// permanent miss. The second kind moves the counts and gives no reason, so
    /// the reason is asserted here instead.
    #[test]
    fn every_resident_entry_is_still_findable_and_the_table_is_in_order() {
        let mut slots = table::<32>();
        let mut cache = Cache::new(&mut slots);
        let refused = workload(&mut cache, SEED);
        assert!(refused > 0, "the workload stopped producing refusals");
        assert_eq!(cache.len(), 32, "the workload stopped filling the table");
        let resident: [Key; 32] =
            core::array::from_fn(|at| *cache.keys().nth(at).expect("thirty-two resident"));
        for pair in resident.windows(2) {
            assert!(pair[0] < pair[1], "out of order: {:?} then {:?}", pair[0], pair[1]);
        }
        let before = cache.counts();
        for key in &resident {
            assert!(cache.get(key).is_some(), "a resident entry could not be found: {key:?}");
        }
        let after = cache.counts();
        assert_eq!(after.misses, before.misses, "a lookup for a key the table holds missed");
        assert_eq!(after.hits, before.hits + resident.len() as u64);
    }

    /// Overwriting an entry is not a new residency, and is counted apart.
    #[test]
    fn a_replacement_is_not_a_new_residency() {
        let mut slots = table::<4>();
        let mut cache = Cache::new(&mut slots);
        assert_eq!(cache.insert(&key_of("one"), 1), Admission::Placed);
        assert_eq!(cache.insert(&key_of("one"), 7), Admission::Replaced);
        assert_eq!(cache.len(), 1);
        assert_eq!(cache.get(&key_of("one")).copied(), Some(7));
        let counts = cache.counts();
        assert_eq!((counts.admissions, counts.replacements, counts.evictions), (2, 1, 0));
    }

    /// A cache with no slots is a legal cache. It answers rather than panicking,
    /// and the answer is counted.
    #[test]
    fn a_cache_with_no_slots_refuses_and_says_so() {
        let mut slots = table::<0>();
        let mut cache = Cache::new(&mut slots);
        assert_eq!(cache.insert(&key_of("anything"), 1), Admission::NoRoom);
        assert_eq!(cache.get(&key_of("anything")).copied(), None);
        assert_eq!(cache.counts().refused, 1);
        assert_eq!(cache.counts().misses, 1);
        assert_eq!(cache.len(), 0);
        assert!(cache.is_empty());
    }

    /// The residency bound moves the eviction count and leaves the lookup count
    /// alone — which is what makes the bound a policy rather than a correctness
    /// argument.
    #[test]
    fn the_bound_moves_evictions_and_not_lookups() {
        let mut small_slots = table::<8>();
        let mut large_slots = table::<128>();
        let mut small = Cache::new(&mut small_slots);
        let mut large = Cache::new(&mut large_slots);
        let small_refused = workload(&mut small, SEED);
        let large_refused = workload(&mut large, SEED);
        assert_eq!(small_refused, large_refused, "the bound decides nothing about naming");
        let (a, b) = (small.counts(), large.counts());
        assert_eq!(a.hits + a.misses, b.hits + b.misses, "the same lookups either way");
        assert!(a.evictions > b.evictions, "a smaller table threw more away: {a:?} against {b:?}");
        assert!(a.hits < b.hits, "and hit less often for it");
    }

    /// Every feature has its own bit, and a set is the union of them.
    #[test]
    fn a_feature_set_is_injective_on_the_set_it_holds() {
        let mut seen = 0u16;
        for feature in Feature::ALL {
            assert_eq!(seen & feature.bit(), 0, "{} shares a bit", feature.tag());
            seen |= feature.bit();
            assert!(FeatureSet::of(&[feature]).contains(feature));
        }
        assert_eq!(FeatureSet::of(&Feature::ALL).len(), Feature::ALL.len() as u32);
        assert_eq!(FeatureSet::of(&Feature::ALL).bits(), seen);
        assert!(FeatureSet::NONE.is_empty());
        let one_way = FeatureSet::of(&[Feature::Kerning, Feature::MarkPositioning]);
        let other_way = FeatureSet::of(&[Feature::MarkPositioning, Feature::Kerning]);
        assert_eq!(one_way, other_way, "a set is not a list");
    }

    /// A size no pen could be set at is not a key, and the refusal says which
    /// bound rather than saying only that something was wrong.
    #[test]
    fn a_size_no_pen_admits_is_not_a_key() {
        use crate::metric::NotAMetric;
        assert_eq!(
            Key::new(FACES[0].0, 0, EMS_X64[0], FeatureSet::NONE, "x"),
            Err(NotCacheable::Size(NotAMetric::UpemOutOfRange))
        );
        assert_eq!(
            Key::new(FACES[0].0, FACES[0].1, 0, FeatureSet::NONE, "x"),
            Err(NotCacheable::Size(NotAMetric::EmSizeOutOfRange))
        );
    }
}
