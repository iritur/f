// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a backend says about itself, and which rung that sentence selects.
//!
//! # The column this file is
//!
//! RFC 0080 says a rung declares three things, and gives them a four-column
//! table. Two of the three are types in [`crate::ladder`] — the cost metric and
//! the fidelity — and the third, *what the machine must supply*, was left in
//! that table and nowhere else, on a reason worth quoting because this file has
//! to answer it: expressing it needs a vocabulary for what a backend reports
//! about itself, there is no backend, and *a wrong guess frozen into the crate
//! everything else depends on is worse than an absent one, because the
//! compositor would then have to satisfy it*.
//!
//! The answer is that what is written below is not a guess at what a GPU driver
//! will say. It is a **reading of four cells**, one capability per clause, and
//! the cells are read at compile time — `include_str!` on the RFC, in the tests
//! — so the reading cannot drift from the decision it reads. The vocabulary is
//! exactly as large as the table and not one word larger. That is the whole of
//! the discipline here, and it is what makes the guess the RFC feared
//! impossible: nothing below is a prediction about hardware, and every line of
//! it is refuted by an edit to one document.
//!
//! What it costs is that a real backend reporting something this list has no
//! word for is not a line added here. It is a row edited in RFC 0080's table and
//! *then* a line here, in that order, because the table is where the decision
//! lives and this file is only where it is evaluated. There is no
//! `Capability::Other`, for [`crate::node`]'s reason: a vocabulary with an
//! escape hatch is a vocabulary that stops being read.
//!
//! # Why there is no `Backend` type in a file called `backend`
//!
//! Because a backend is a thing that runs, and this crate depends on nothing,
//! runs nothing, and must stay deletable — `interface/Cargo.toml` argues that
//! and part III of `ring-scene-boot` is why. What crosses from a backend into
//! this crate is not the backend; it is the one sentence it says about itself,
//! and that sentence is a set. [`Capabilities`] is that set, [`select`] is what
//! reads it, and the driver, the device and the process it came from are all on
//! the other side of a boundary this crate never reaches across.
//!
//! # Reported means usable, which is how the second clause of row 2 is a type
//!
//! Row 2 reads *compute shaders; the scan of rung 1 may be unavailable **or
//! unusable***. Those are two different facts about a machine and one fact about
//! the ladder, and collapsing them is deliberate: a [`Capabilities`] is what a
//! backend undertakes to *do*, not an inventory of what it has. A driver
//! exposing a subgroup scan that miscompiles, hangs, or is correct and slower
//! than the CPU reports [`Capability::SubgroupScan`] **absent**, and rung 1 is
//! then never offered a machine that cannot run it.
//!
//! The reversal condition is precise, because this is the clause most likely to
//! be wanted back: if anything other than rung selection ever needs to tell
//! *absent* from *present and broken* — a bug report, a driver blocklist, a
//! diagnostic a user pastes into an issue — that is a **second set beside this
//! one**, not a third state inside it. A tri-state here would put a value into
//! every predicate below that neither satisfies nor refuses, and the first
//! reader to write `if !unusable` would have re-decided the ladder.
//!
//! # Selection is total, and refusing is one of its answers
//!
//! [`select`] returns `Result<Rung, Refused>`. The floor is not a default and
//! cannot become one here: RFC 0080 decides that *a machine that satisfies no
//! rung's requirement is refused a compositor rather than given the lowest one*,
//! on the grounds that a refusal is answerable and a compositor that misses
//! every frame is not. `E3-B02b` inherits that from this signature rather than
//! from a convention — there is no path through this module that produces a
//! [`Rung`] from a report that satisfies none, because [`Refused`] has no method
//! that returns one.
//!
//! It had one. `Refused::nearest` returned the rung with the fewest unmet
//! demands, which is genuinely the answerable part of a refusal and is also
//! `refused.nearest().0` away from being the silent floor this RFC forecloses,
//! written by somebody who wanted the program to start. It is deleted and
//! [`Refused::unmet`] is what is left: a caller that wants to rank the rungs
//! writes that loop, in its own file, under its own name. Removing the
//! possibility beat documenting it.
//!
//! # What is a compile error in this file
//!
//! A **fifth rung**. [`TABLE`] is `[(Rung, Requirement); RUNGS]`, `RUNGS` is
//! counted from `ladder!`'s list, and four initialisers do not make an array of
//! five. Not a test, and deliberately not: the tests below match exhaustively on
//! [`Rung`] because that is legible, and an exhaustive match is precisely the
//! guard this project has already been defeated through twice — `Rung::Fifth =>
//! ()` compiles, and so does an arm copied from its neighbour. The array length
//! is what fails.
//!
//! A **rung nothing can select**. If some rung above asks for a subset of what a
//! lower rung asks for, every machine that satisfies the lower one is caught by
//! the higher one first and the lower rung is unreachable — a renderer to
//! maintain, a claim row to keep green, and no machine that ever runs it. That
//! is checked in a `const` block, so it is a build failure and not a red test,
//! and it is exact rather than conservative: every subset of [`Capability::ALL`]
//! is a report some backend may make, so a rung's own required set is the
//! minimal witness that selects it, and *no rung above asks for a subset* is the
//! whole of reachability. The day a report type forbids some combinations — a
//! backend that must always claim a CPU, say — that argument needs redoing.
//!
//! A **table out of the ladder's order**. [`TABLE`] is indexed by
//! [`Rung::index`], and a `const` block asserts each row carries the rung whose
//! position it occupies. Two rows swapped without their rungs is the one way
//! that index can lie, and it does not compile.
//!
//! # The finding this encoding produced, stated rather than buried
//!
//! Rung 3 asks for *a CPU, and any way at all to put a finished image on the
//! screen*, which nearly every machine with a display satisfies — so the floor
//! is selected only by a backend that offers a triangle pipeline and **no way to
//! put a finished image on the screen**. That is a narrow machine, and it is
//! what the table says: the ladder descends in fidelity, so an exact CPU raster
//! outranks an approximate floor wherever both can start, and the floor is what
//! is left when the exact one cannot start at all.
//!
//! This is a live reversal condition rather than a curiosity, and `E3-B02l` is
//! the task that settles it. If every real fixed-function pipeline also blits —
//! if a textured quad is *any way at all* — then no backend selects rung 4, and
//! either row 3's clause is wrong or the floor is the seam maintained for its
//! own sake that RFC 0080's reversal section describes. The refutation is a
//! machine, not an argument, and
//! `a_triangle_pipeline_that_can_present_takes_the_exact_rung` is the sentence
//! it would refute.
//!
//! # What this file does not add
//!
//! Row 4 does not ask for a CPU, and the floor flattens paths on one —
//! [`crate::ladder::CpuCoverage::None`]'s own comment says flattening is on the
//! CPU at every rung. The demand is still not here. A predicate that asks for
//! more than its cell asks for has stopped being a reading of the table and
//! become a second decision, made in the file least entitled to make it; and the
//! failure it produces is a backend refused for a requirement no document
//! states, which is the hardest kind of refusal to argue with. If `E3-B02` meets
//! a machine refused that way, the invented demand is the defect.
//!
//! Nothing here observes a clock, a source of randomness or an ordering.
//! [`select`] is a pure function of its argument — a `const fn` over a fixed
//! array — so RFC 0004's substrate is not needed and this crate keeps its
//! promise to depend on nothing.

use crate::ladder::{RUNGS, Rung};

/// The reported vocabulary, written once so that it cannot be written twice.
///
/// One line per capability, carrying the variant, the word a log line spells it
/// with, and the words RFC 0080's table uses for it. [`Capability`],
/// [`CAPABILITY_COUNT`], [`Capability::ALL`] and every accessor are emitted from
/// it.
///
/// # Why a macro, which is this crate's third
///
/// For `vocabulary!`'s reason in [`crate::node`] and `ladder!`'s in
/// [`crate::ladder`], and there is no new argument to make: a capability absent
/// from `ALL` is one no set iterates, no refusal names and no test sees, and
/// both guards written against that shape elsewhere in this crate were defeated
/// — an array cannot see what it omits, and an exhaustive `match` demands an arm
/// rather than an arm that says anything. One list, read four ways, and the
/// second place to write a capability does not exist.
///
/// The `phrase` is the load-bearing field and the reason this is not simply
/// `ladder!` again. It is the words the RFC's table spends on the capability,
/// and the tests read the table and require that **every word of every cell is
/// accounted for by some phrase** — which is how *no clause of that table is
/// left in a paragraph* becomes something the build checks rather than something
/// a reviewer counted once.
macro_rules! capability {
    (
        $(
            $(#[$about:meta])*
            $variant:ident, $spelling:literal, $phrase:literal,
        )*
    ) => {
        /// How many capabilities the table asks about.
        ///
        /// Counted from the list rather than written down, for `Role::COUNT`'s
        /// reason: a number that is derived cannot disagree with what it is
        /// derived from. It is also the width [`Capabilities`] needs, and the
        /// `const` block under that type turns *more capabilities than a `u32`
        /// holds* into a build failure rather than a set that silently drops the
        /// thirty-third.
        pub const CAPABILITY_COUNT: usize = [$(stringify!($variant)),*].len();

        /// One thing a backend says it will do.
        ///
        /// The list is RFC 0080's third column and nothing else: every variant
        /// below is a clause of one of four table cells, and a clause of those
        /// cells that is not a variant fails
        /// `no_clause_of_the_column_is_left_in_a_paragraph`. There is no
        /// `Other`, no variant carrying a string and no `#[non_exhaustive]` — a
        /// backend with a capability this list has no word for is a row edited in
        /// RFC 0080 first.
        ///
        /// Emitted from the [`capability!`] invocation that declares them, which
        /// is the only place a capability is written.
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        pub enum Capability {
            $($(#[$about])* $variant,)*
        }

        impl Capability {
            /// Every capability, in the order the table first asks for one.
            ///
            /// Not alphabetical and not by importance: reading RFC 0080's cells
            /// top to bottom and left to right produces exactly this sequence,
            /// and `the_capability_list_is_the_order_the_table_first_asks_for_it`
            /// holds it there. A new capability therefore has one correct place
            /// in this list — where its clause first appears — rather than being
            /// appended wherever the diff is smallest.
            ///
            /// Emitted from the same list as the enum, so there is no way to
            /// write a capability this array does not get.
            pub const ALL: [Self; CAPABILITY_COUNT] = [$(Self::$variant),*];

            /// This capability's bit position in a [`Capabilities`].
            ///
            /// The enum's own discriminant, which is the position of the line
            /// that declared it, which is its index in [`ALL`](Self::ALL): one
            /// list, read three ways.
            #[must_use]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The word a refusal, a log line and a state tree share.
            ///
            /// One spelling, in one place, because *which capability was
            /// missing* is the whole content of a refused compositor's
            /// explanation, and a machine's owner will grep for it across a boot
            /// log and a bug report.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }

            /// The words RFC 0080's table spends on this capability.
            ///
            /// The seam between the predicate and the decision. It is prose on
            /// purpose — it has to match a cell, and the cell is written for a
            /// reader — and it is checked against the document itself rather
            /// than against a copy of it, in both directions: a phrase in no
            /// cell is a capability this ladder does not ask for, and a cell word
            /// in no phrase is a clause with no predicate.
            #[must_use]
            pub const fn phrase(self) -> &'static str {
                match self {
                    $(Self::$variant => $phrase,)*
                }
            }

            /// The capability of that name, if the vocabulary has one.
            ///
            /// Derived from [`ALL`](Self::ALL) rather than written as a second
            /// match, which is what makes the negative answer structural: there
            /// is no name this accepts that is not already a capability, so a
            /// backend cannot widen the vocabulary by reporting a string.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|capability| capability.name() == name)
            }
        }
    };
}

capability! {
    /// Programmable shaders that are not part of the draw pipeline, which is
    /// what section 08's encode, coarse raster, fine raster and present stages
    /// are written as.
    ///
    /// Rows 1 and 2 both open with it, and it is the single capability that
    /// separates the two exact GPU rungs from the two rungs below them.
    ComputeShaders, "compute-shaders", "compute shaders",

    /// Buffers a compute shader can both read and write, which is how one stage
    /// of section 08's pipeline hands its output to the next without a round
    /// trip through the CPU.
    ///
    /// Asked for by row 1 alone. Row 2 does not name it and this file does not
    /// add it: see the module comment on demands the table does not make.
    StorageBuffers, "storage-buffers", "storage buffers",

    /// A prefix scan across cooperating lanes.
    ///
    /// The one stage of the pipeline that depends on lanes cooperating, and
    /// therefore the one a GPU can fail at while succeeding at everything else —
    /// RFC 0080 keeps the hybrid rung for precisely this failure and says most
    /// of the installed base older than about five years exhibits it. Reported
    /// when it is *usable*: a scan that is present and broken is a scan this set
    /// does not carry.
    SubgroupScan, "subgroup-scan", "prefix scan across cooperating lanes",

    /// A CPU.
    ///
    /// Row 3's first clause, and trivially true of anything that can run the
    /// code reporting it — which is not a reason to drop it. A clause dropped
    /// for being easy is a clause that has quietly stopped being a requirement,
    /// and rung 3's predicate would then be a single term whose failure mode
    /// nobody has looked at. It is also what makes the ladder's
    /// non-monotonicity visible: rung 3 asks for two things rung 1 asks for
    /// neither of.
    Cpu, "cpu", "CPU",

    /// Any way at all to put a finished image on the screen.
    ///
    /// Deliberately the weakest thing the table asks for anywhere: a blit, an
    /// upload, a textured quad, a framebuffer somebody else scans out. It is
    /// what rung 3 needs beyond a CPU and the only thing it needs, and it is
    /// therefore also what the floor's reachability turns on — see the module
    /// comment.
    ImagePresent, "image-present", "any way at all to put a finished image on the screen",

    /// A fixed-function triangle pipeline.
    ///
    /// Row 4, whole. The floor flattens paths to triangles and hands them to
    /// hardware that rasters them, which is why the floor is
    /// [`crate::ladder::CpuCoverage::None`] and why it is last anyway: the
    /// picture it draws is not the picture the scene describes.
    TrianglePipeline, "triangle-pipeline", "fixed-function triangle pipeline",
}

/// A backend's whole sentence about itself: the set of things it will do.
///
/// A set rather than a struct of `bool`s because every predicate below is a
/// subset question and a bitset answers one in an instruction, and because a
/// struct of named flags would be a second place each capability is written —
/// the defect [`capability!`] exists to remove, reintroduced one type later.
///
/// Absence is the only negative. There is no *unknown* and no *broken*; the
/// module comment says why, and says what a second set beside this one would be
/// for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Capabilities {
    /// One bit per [`Capability`], at [`Capability::index`].
    bits: u32,
}

// A `u32` holds thirty-two capabilities and this vocabulary has six. The day it
// has thirty-three, this fails to build rather than dropping the bits past the
// end — which is the failure that would be invisible, because a dropped bit
// reads as a backend that did not report the capability, and a refusal naming it
// looks exactly like a machine that does not have it.
const _: () = assert!(
    CAPABILITY_COUNT <= u32::BITS as usize,
    "more capabilities than a `u32` holds; widen `Capabilities::bits`"
);

impl Capabilities {
    /// A backend that reports nothing.
    ///
    /// The report a compositor gets from a device it could not open, and the one
    /// [`select`] must refuse rather than floor.
    pub const NONE: Self = Self { bits: 0 };

    /// The set of exactly these capabilities.
    #[must_use]
    pub const fn of(reported: &[Capability]) -> Self {
        let mut bits = 0;
        let mut at = 0;
        while at < reported.len() {
            bits |= 1 << reported[at].index();
            at += 1;
        }
        Self { bits }
    }

    /// This set, and `capability` as well.
    #[must_use]
    pub const fn with(self, capability: Capability) -> Self {
        Self { bits: self.bits | 1 << capability.index() }
    }

    /// Does the backend report `capability`?
    #[must_use]
    pub const fn has(self, capability: Capability) -> bool {
        self.bits & 1 << capability.index() != 0
    }

    /// Does this set hold everything in `wanted`?
    ///
    /// The subset question every rung's predicate is. Note the direction, which
    /// the name is chosen to fix: `reported.includes(required)`, never the other
    /// way round, and getting it backwards would offer the top rung to a machine
    /// that reported one of its three demands.
    #[must_use]
    pub const fn includes(self, wanted: Self) -> bool {
        self.bits & wanted.bits == wanted.bits
    }

    /// This set with everything in `other` taken out.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self { bits: self.bits & !other.bits }
    }

    /// Does the backend report nothing at all?
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.bits == 0
    }

    /// How many capabilities are in the set.
    #[must_use]
    pub const fn count(self) -> u32 {
        self.bits.count_ones()
    }

    /// The capabilities, in [`Capability::ALL`] order.
    ///
    /// Ordered by the vocabulary rather than by insertion, so that two refusals
    /// naming the same missing capabilities print them in the same order. RFC
    /// 0004's iteration-order rule is satisfied by construction: there is no
    /// map, no hash and no seed here — the order is a fixed array's.
    pub fn iter(self) -> impl Iterator<Item = Capability> {
        Capability::ALL.into_iter().filter(move |capability| self.has(*capability))
    }
}

/// One rung's *what the machine must supply* column, as a predicate.
///
/// Two sets, and only the first decides anything. `required` is the conjunction
/// [`admits`](Self::admits) evaluates; `tolerated` is the clause a row spends on
/// what it explicitly does **not** require, which row 2 is the whole reason for.
///
/// # Why a tolerated set that no admission reads
///
/// Because *the scan of rung 1 may be unavailable or unusable* is a clause, and
/// a clause encoded as the absence of a line is a clause somebody deletes by
/// accident. Making it a set costs one field and buys two `const` assertions
/// that state the clause exactly: a tolerated capability must be one the rung
/// **above** demanded, and it must be one this rung admits a machine without.
/// Both are build failures, so the sentence *this is the rung for hardware whose
/// scan does not work* cannot quietly stop being true.
///
/// It deliberately takes no part in [`admits`](Self::admits). A second set that
/// could also refuse a machine would be a second predicate per rung, and the two
/// would eventually disagree — which is the shape this crate keeps deleting.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Requirement {
    /// Everything the machine must supply for this rung to start.
    required: Capabilities,
    /// What the row says the machine may be without, having been asked for it
    /// one rung up.
    tolerated: Capabilities,
}

impl Requirement {
    /// A row of the table.
    const fn new(required: Capabilities, tolerated: Capabilities) -> Self {
        Self { required, tolerated }
    }

    /// Everything the machine must supply for this rung to start.
    #[must_use]
    pub const fn required(self) -> Capabilities {
        self.required
    }

    /// What this row says the machine may be without.
    ///
    /// Empty for three of the four rungs, and that is not a deficiency: a row
    /// that drops no demand of the row above it has nothing to tolerate.
    #[must_use]
    pub const fn tolerated(self) -> Capabilities {
        self.tolerated
    }

    /// May a backend reporting `reported` start this rung?
    ///
    /// The predicate, whole. A conjunction and nothing else — no weights, no
    /// *nearly*, no capability that half counts — because a rung either starts
    /// or does not, and a compositor that started a rung it half satisfied is
    /// the frame-missing outcome RFC 0080 refuses in favour of an answerable no.
    #[must_use]
    pub const fn admits(self, reported: Capabilities) -> bool {
        reported.includes(self.required)
    }

    /// What this rung asked for and `reported` did not supply.
    ///
    /// Empty exactly when [`admits`](Self::admits) is true, which is what makes
    /// a refusal answerable: the set this returns is the list of things to go
    /// and get.
    #[must_use]
    pub const fn unmet_by(self, reported: Capabilities) -> Capabilities {
        self.required.without(reported)
    }
}

/// RFC 0080's third column, one row per rung, in the ladder's order.
///
/// Indexed by [`Rung::index`], which is what makes a fifth rung a build failure
/// here: `RUNGS` is counted from `ladder!`'s list, and four initialisers are not
/// an array of five. That is the guard — not the exhaustive matches in the tests
/// below, which this project has been defeated through twice.
///
/// Each row is a reading of one cell, and the comment above it says which clause
/// became which capability. The cells themselves are not copied into this file:
/// a second copy of the table would be a second thing to keep agreeing with the
/// decision, and the tests read the document instead.
const TABLE: [(Rung, Requirement); RUNGS] = [
    // `compute shaders, storage buffers, and a prefix scan across cooperating
    // lanes` — three clauses, three demands, and nothing tolerated: this is the
    // top of the ladder, so there is no row above it to have dropped anything.
    (
        Rung::ComputePath,
        Requirement::new(
            Capabilities::of(&[
                Capability::ComputeShaders,
                Capability::StorageBuffers,
                Capability::SubgroupScan,
            ]),
            Capabilities::NONE,
        ),
    ),
    // `compute shaders; the scan of rung 1 may be unavailable or unusable` — one
    // demand and one explicit tolerance. The tolerance is why this rung exists
    // at all, and the `const` blocks below require it to be a demand rung 1 made
    // and one this rung admits a machine without.
    (
        Rung::Hybrid,
        Requirement::new(
            Capabilities::of(&[Capability::ComputeShaders]),
            Capabilities::of(&[Capability::SubgroupScan]),
        ),
    ),
    // `a CPU, and any way at all to put a finished image on the screen` — two
    // clauses, neither of which rung 1 or rung 2 asks for. This row is where the
    // ladder stops being monotone in hardware, which RFC 0080 states as a
    // property rather than conceding as a gap.
    (
        Rung::CpuRaster,
        Requirement::new(
            Capabilities::of(&[Capability::Cpu, Capability::ImagePresent]),
            Capabilities::NONE,
        ),
    ),
    // `a fixed-function triangle pipeline` — one clause, and pointedly not `and
    // a CPU` even though the floor flattens paths on one. The module comment
    // says why the missing demand stays missing.
    (
        Rung::TessellatingFloor,
        Requirement::new(Capabilities::of(&[Capability::TrianglePipeline]), Capabilities::NONE),
    ),
];

// The table is the ladder, in the ladder's order, and each row's demands and
// tolerances are disjoint.
//
// Indexing by `Rung::index` becomes a lie the moment two rows are swapped
// without their rungs, and it is the kind of lie nothing downstream can detect:
// every rung would still get *a* requirement. A row that both requires and
// tolerates the same capability is the same defect read from the other side — a
// predicate refusing a machine the row says it accepts.
const _: () = {
    let mut at = 0;
    while at < RUNGS {
        assert!(TABLE[at].0.index() == at, "`TABLE` is not the ladder in the ladder's order");
        let row = TABLE[at].1;
        assert!(
            row.required.bits & row.tolerated.bits == 0,
            "a row both requires a capability and says the machine may be without it"
        );
        at += 1;
    }
};

// A tolerated capability is exactly a demand the rung above made and this rung
// drops.
//
// This is *the scan of rung 1 may be unavailable or unusable*, written as
// arithmetic. Two halves: the capability was asked for one rung up, and a
// machine meeting the rung above's demands except for it lands here. Without the
// second half `tolerated` would be decoration — a set naming a capability some
// unrelated clause still refused — and the sentence the hybrid exists for would
// be false while this file said it.
//
// The top rung is the case the loop below cannot state, so it is stated first:
// there is no row above it, so there is nothing it can be tolerating the absence
// of, and `every_capability_a_rung_requires_is_named_by_its_row` looks a
// tolerance up in the row above without checking that one exists.
const _: () = {
    assert!(
        TABLE[0].1.tolerated.is_empty(),
        "the top rung tolerates the absence of something no row above it asked for"
    );
    let mut at = 1;
    while at < RUNGS {
        let above = TABLE[at - 1].1;
        let here = TABLE[at].1;
        assert!(
            here.tolerated.bits & above.required.bits == here.tolerated.bits,
            "a rung tolerates the absence of something the rung above it never asked for"
        );
        if !here.tolerated.is_empty() {
            let deprived = above.required.without(here.tolerated);
            assert!(
                here.admits(deprived),
                "a rung does not admit the machine its own tolerance clause describes"
            );
            assert!(
                !above.admits(deprived),
                "the tolerated capability is not what separates this rung from the one above"
            );
        }
        at += 1;
    }
};

// No rung on this ladder is unreachable.
//
// If a rung above asks for a subset of what a lower rung asks for, every machine
// satisfying the lower one is caught by the higher one first and the lower rung
// is never selected — a renderer to maintain, a row in `claims/0033` to keep
// green, and no machine that ever runs it. The check is exact rather than
// conservative: every subset of `Capability::ALL` is a report some backend may
// make, so a rung's own required set is the minimal witness that selects it, and
// *no rung above asks for a subset* is the whole of reachability.
// `the_minimum_machine_for_a_rung_selects_it` exercises that witness for each
// rung; this is the clause that makes it impossible to write one that has none.
const _: () = {
    let mut lower = 0;
    while lower < RUNGS {
        let mut upper = 0;
        while upper < lower {
            assert!(
                !TABLE[lower].1.required.includes(TABLE[upper].1.required),
                "a rung above asks for no more than this one, so no machine ever reaches it"
            );
            upper += 1;
        }
        lower += 1;
    }
};

/// What `rung` requires of a machine.
///
/// The accessor [`crate::ladder`] does not have, kept here because that module
/// may not depend on this one and because RFC 0080 left this column out of it on
/// purpose. Total by construction: [`TABLE`] has one row per rung and a `const`
/// block holds each row at its rung's index, so there is no rung this answers
/// for by default and none it cannot answer for.
#[must_use]
pub const fn requirement(rung: Rung) -> Requirement {
    TABLE[rung.index()].1
}

/// A backend that satisfies no rung, and what each rung wanted from it.
///
/// Named rather than unit, and carrying the report rather than a message,
/// because RFC 0080's argument for refusing is that *a refusal is answerable*: a
/// supervisor holding one of these can tell a machine's owner which capability
/// would have bought which rung. A refusal that only says no is the outcome that
/// argument was made against.
///
/// There is deliberately no method here that returns a [`Rung`]. The module
/// comment records the one that used to be, and why it went.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Refused {
    /// What the backend said about itself.
    reported: Capabilities,
}

impl Refused {
    /// What the backend said about itself.
    #[must_use]
    pub const fn reported(self) -> Capabilities {
        self.reported
    }

    /// What `rung` asked for and this backend did not report.
    ///
    /// Never empty for any rung — a rung with nothing unmet would have been
    /// selected — which is what `a_backend_that_satisfies_no_rung_is_refused`
    /// asserts of all four at once.
    #[must_use]
    pub const fn unmet(self, rung: Rung) -> Capabilities {
        requirement(rung).unmet_by(self.reported)
    }
}

/// The rung this backend starts on, or a refusal.
///
/// The ladder is walked from the top and the first rung whose predicate admits
/// the report is the answer, which is [`Rung::ALL`]'s order and therefore
/// descending fidelity: a machine that can draw the scene exactly is never given
/// an approximation of it because the approximation was cheaper. RFC 0080 argues
/// that ordering at length, and this function is the only place it is acted on.
///
/// Chosen once, by the compositor, at a start. Nothing here can be called again
/// to promote a running compositor to a better rung — that is RFC 0080's
/// foreclosure and RFC 0008's *restart is the supervisor's act* — because
/// nothing here holds any state to promote. A backend that comes back after a
/// driver restart is a new compositor with a new report.
///
/// # Errors
///
/// [`Refused`] when the report satisfies no rung's predicate, which RFC 0080
/// makes a refused compositor rather than the floor. The error carries the
/// report, so the caller can say what would have bought what.
pub const fn select(reported: Capabilities) -> Result<Rung, Refused> {
    let mut at = 0;
    while at < RUNGS {
        let (rung, requirement) = TABLE[at];
        if requirement.admits(reported) {
            return Ok(rung);
        }
        at += 1;
    }
    Err(Refused { reported })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ladder::Fidelity;

    /// RFC 0080, read when this file is compiled.
    ///
    /// The same `include_str!` [`crate::ladder`]'s tests make, for the same
    /// reason and against a different column: the decision lives in that
    /// document, this module evaluates it, and a rebuild when the document
    /// changes is the cheapest way for that to be a fact the toolchain knows
    /// rather than a convention a reviewer applies.
    const RFC: &str =
        include_str!("../../docs/rfc/0080-the-fallback-ladder-is-decided-before-it-is-needed.md");

    /// Words that are grammar rather than clauses.
    ///
    /// Deliberately the shortest list that lets the four cells parse, and it
    /// stays that way: every word added here is a word the coverage test stops
    /// asking about, so a long list is how *no clause is left in a paragraph*
    /// would quietly stop being true. A new connective in the table is a line in
    /// this array, in a diff, with somebody reading it.
    const CONNECTIVES: [&str; 6] = ["a", "an", "and", "the", "of", "or"];

    /// The grammar a row uses to say it does *not* require something.
    ///
    /// Also minimal, and it carries an obligation rather than an exemption: a
    /// cell using any of these words must belong to a rung whose
    /// [`Requirement::tolerated`] is non-empty, so the words cannot be used to
    /// wave a clause past the coverage test. `need`, `not` and `optional` are
    /// absent on purpose — the day the table says one of them, this goes red and
    /// somebody decides what it meant.
    const TOLERANCE: [&str; 4] = ["may", "be", "unavailable", "unusable"];

    /// The one markdown table in RFC 0080, as `[number, rung, supply, cost,
    /// picture]`, data rows only.
    ///
    /// A walk rather than a parser, for [`crate::ladder`]'s reason: a parser
    /// needs an allocator this crate does not have. A row is a line of five
    /// pipe-delimited cells; the header and the `---` rule are dropped by the
    /// last filter. If the document ever stops being readable this way, the
    /// assertions below panic on a missing row rather than passing over nothing.
    fn table_rows() -> impl Iterator<Item = [&'static str; 5]> {
        RFC.lines()
            .filter(|line| line.trim_start().starts_with('|'))
            .filter_map(|line| {
                let mut cells = line.trim().trim_matches('|').split('|').map(str::trim);
                let row =
                    [cells.next()?, cells.next()?, cells.next()?, cells.next()?, cells.next()?];
                cells.next().is_none().then_some(row)
            })
            .filter(|row| row[0] != "#" && !row[0].starts_with("---"))
    }

    /// The table row that prices `rung`, found by the metric name — the same key
    /// `the_rfc_s_table_states_this_ladder_s_order` uses one module over.
    fn row_for(rung: Rung) -> [&'static str; 5] {
        let Some(row) = table_rows().find(|row| row[3].contains(rung.cost_metric())) else {
            panic!("RFC 0080's table has no row for `{}`", rung.cost_metric())
        };
        row
    }

    /// Where `needle` first occurs in `haystack`, ignoring ASCII case.
    ///
    /// Ignoring case because the table writes `a CPU` and a capability's name is
    /// spelled for a log line. Every byte in both is ASCII, so this is a byte
    /// walk and not a Unicode one, and a non-ASCII phrase would make it wrong
    /// rather than slow — which is the reversal condition, and the day the table
    /// grows one is the day this needs `char_indices`.
    fn find_ignoring_case(haystack: &str, needle: &str) -> Option<usize> {
        let (hay, pin) = (haystack.as_bytes(), needle.as_bytes());
        if pin.is_empty() || pin.len() > hay.len() {
            return None;
        }
        (0..=hay.len() - pin.len()).find(|&at| hay[at..at + pin.len()].eq_ignore_ascii_case(pin))
    }

    /// The words of `text`, with punctuation dropped and hyphens kept.
    ///
    /// Hyphens kept because `fixed-function` is one word in the cell and one
    /// word in the phrase, and splitting it would let half a clause pass as a
    /// connective.
    fn words(text: &str) -> impl Iterator<Item = &str> {
        text.split(|c: char| !c.is_alphanumeric() && c != '-').filter(|word| !word.is_empty())
    }

    /// Is `word` part of what some capability of `requirement` is called?
    fn spoken_for(word: &str, requirement: Requirement) -> bool {
        requirement.required().iter().chain(requirement.tolerated().iter()).any(|capability| {
            words(capability.phrase()).any(|part| part.eq_ignore_ascii_case(word))
        })
    }

    /// Is `word` a reference to another row, as row 2's `the scan of rung 1` is?
    ///
    /// Derived from `RUNGS` rather than matching `1`, so a ladder with a fifth
    /// rung would not need this edited — though it would never get here, since
    /// `TABLE` would not compile.
    fn is_a_rung_reference(word: &str) -> bool {
        word.eq_ignore_ascii_case("rung")
            || matches!(word.parse::<usize>(), Ok(number) if (1..=RUNGS).contains(&number))
    }

    /// The machine each rung was written for.
    ///
    /// Four synthetic backends, one per rung, written as plausible parts rather
    /// than as minimal witnesses — the minimal ones are derived from the
    /// requirements in `the_minimum_machine_for_a_rung_selects_it`, and a test
    /// that only ever showed the minimum would not notice a predicate refusing a
    /// machine for having *more* than it needed.
    ///
    /// **The exhaustive match is legible and is not the guard.** A fifth rung
    /// fails to build at `TABLE` long before it reaches here, and an arm copied
    /// from its neighbour is caught by the caller rather than by the compiler:
    /// each machine is asserted to select the rung it was written under, so a
    /// `Rung::Fifth` arm carrying the floor's machine selects the floor and goes
    /// red.
    fn machine_for(rung: Rung) -> (&'static str, Capabilities) {
        match rung {
            // Everything the table asks for anywhere: a current discrete part.
            Rung::ComputePath => {
                ("a part that reports the whole table", Capabilities::of(&Capability::ALL))
            }
            // Compute shaders and storage buffers, and a subgroup scan either
            // absent or known not to work — RFC 0080 says this is most of the
            // installed base older than about five years.
            Rung::Hybrid => (
                "compute shaders with no usable scan",
                Capabilities::of(&[
                    Capability::ComputeShaders,
                    Capability::StorageBuffers,
                    Capability::Cpu,
                    Capability::ImagePresent,
                    Capability::TrianglePipeline,
                ]),
            ),
            // A CPU and a framebuffer somebody else scans out: no GPU of any
            // kind, which is the machine this project actually has.
            Rung::CpuRaster => (
                "a CPU and a framebuffer",
                Capabilities::of(&[Capability::Cpu, Capability::ImagePresent]),
            ),
            // Triangles and no way to put a finished image on the screen. The
            // narrowness is the finding rather than an oversight: see
            // `a_triangle_pipeline_that_can_present_takes_the_exact_rung`.
            Rung::TessellatingFloor => (
                "a geometry-only fixed-function pipeline",
                Capabilities::of(&[Capability::Cpu, Capability::TrianglePipeline]),
            ),
        }
    }

    #[test]
    fn the_rfc_s_table_is_this_module_s_table() {
        // Before any clause is read, the two have to be the same four rows in
        // the same order. A fifth row in RFC 0080 with no rung here is a
        // requirement nothing evaluates, and it is invisible to every test that
        // walks `Rung::ALL` — because walking the ladder is exactly what it is
        // absent from.
        assert_eq!(
            table_rows().count(),
            RUNGS,
            "RFC 0080's table and this ladder disagree about how many rungs there are"
        );
        for rung in Rung::ALL {
            let row = row_for(rung);
            assert_eq!(
                row[0].parse::<usize>().ok(),
                Some(rung.index() + 1),
                "RFC 0080 numbers `{}` differently from its position on the ladder",
                rung.name()
            );
        }
    }

    #[test]
    fn every_capability_a_rung_requires_is_named_by_its_row() {
        // One direction: a demand this module makes that the table does not. It
        // is the direction the RFC feared — a vocabulary invented here that a
        // compositor would then have to satisfy — and the failure it prevents is
        // a backend refused for a requirement no document states.
        //
        // A tolerated capability is looked up in the row **above**, and this is
        // the table teaching the test rather than the other way round: row 2
        // says `the scan of rung 1`, in a short form, because the full phrase is
        // written one row up and a document does not spell a thing twice. So the
        // clause is checked where it is spelled — which is a stricter reading
        // than *somewhere in this row*, since it also holds the reference to
        // rung 1 to being a reference to rung 1. That the short form appears
        // here at all is `no_clause_of_the_column_is_left_in_a_paragraph`'s
        // business, and it covers `scan` out of this capability's own phrase.
        for rung in Rung::ALL {
            let requirement = requirement(rung);
            for capability in requirement.required().iter() {
                assert!(
                    find_ignoring_case(row_for(rung)[2], capability.phrase()).is_some(),
                    "this module asks {} for `{}`, and RFC 0080's row does not say it",
                    rung.name(),
                    capability.phrase()
                );
            }
            for capability in requirement.tolerated().iter() {
                let above = Rung::ALL[rung.index() - 1];
                assert!(
                    find_ignoring_case(row_for(above)[2], capability.phrase()).is_some(),
                    "{} tolerates `{}` being absent, and {} above it never asked for it",
                    rung.name(),
                    capability.phrase(),
                    above.name()
                );
            }
        }
    }

    #[test]
    fn no_clause_of_the_column_is_left_in_a_paragraph() {
        // **The other direction, and the one the exit turns on.** Every word of
        // every cell has to be accounted for: by a capability the row requires,
        // by one it explicitly tolerates, by a connective, or by a reference to
        // another row. A clause of the table with no predicate is what this
        // fails on, and the allow-lists are deliberately short because every
        // word in them is a word this stops asking about.
        //
        // *The edit that makes it red:* drop `Capability::StorageBuffers` from
        // rung 1's row in `TABLE`, and `storage` and `buffers` are suddenly two
        // words of RFC 0080 that this module does not evaluate.
        for rung in Rung::ALL {
            let cell = row_for(rung)[2];
            let requirement = requirement(rung);
            for word in words(cell) {
                if spoken_for(word, requirement) || is_a_rung_reference(word) {
                    continue;
                }
                if CONNECTIVES.iter().any(|known| known.eq_ignore_ascii_case(word)) {
                    continue;
                }
                if TOLERANCE.iter().any(|known| known.eq_ignore_ascii_case(word)) {
                    assert!(
                        !requirement.tolerated().is_empty(),
                        "{}'s row says `{word}` and this module tolerates nothing for it",
                        rung.name()
                    );
                    continue;
                }
                panic!(
                    "RFC 0080's row for {} says `{word}`, and no capability here reads it",
                    rung.name()
                );
            }
        }
    }

    #[test]
    fn the_capability_list_is_the_order_the_table_first_asks_for_it() {
        // `Capability::ALL`'s order is a claim about the document: read the
        // cells top to bottom, left to right, and this is the sequence. It gives
        // a new capability one correct place — where its clause first appears —
        // rather than the end of the list, which is where a capability goes when
        // nobody has decided anything. It is also the cheapest possible check
        // that every capability is in some cell at all.
        let mut previous: Option<(Capability, (usize, usize))> = None;
        for capability in Capability::ALL {
            let mut first = None;
            for (index, row) in table_rows().enumerate() {
                if let Some(at) = find_ignoring_case(row[2], capability.phrase()) {
                    first = Some((index, at));
                    break;
                }
            }
            let Some(at) = first else {
                panic!("no row of RFC 0080's table says `{}`", capability.phrase())
            };
            if let Some((earlier, before)) = previous {
                assert!(
                    at > before,
                    "the table asks for {} before {}, and this list is the other way round",
                    capability.name(),
                    earlier.name()
                );
            }
            previous = Some((capability, at));
        }
    }

    #[test]
    fn four_synthetic_backends_select_the_four_rungs() {
        // The exit's sentence, run. Four machines, four rungs, one to four, each
        // machine asserted against the rung it was written under — which is what
        // makes `machine_for`'s exhaustive match more than a convention with a
        // `#[test]` on it: an arm copied from its neighbour selects the
        // neighbour's rung and fails here.
        for rung in Rung::ALL {
            let (machine, reports) = machine_for(rung);
            assert_eq!(
                select(reports),
                Ok(rung),
                "rung {}: `{machine}` does not select {}",
                rung.index() + 1,
                rung.name()
            );
        }
    }

    #[test]
    fn the_minimum_machine_for_a_rung_selects_it() {
        // The derived half. A rung's own required set is the minimal report that
        // satisfies it, so if any rung above admitted that report the rung would
        // be unreachable — which the `const` block above already makes a build
        // failure. This is the same property read from the other end, and it
        // costs nothing to keep both: the `const` block says *no rung is
        // unreachable*, and this exhibits the machine that reaches each one.
        for rung in Rung::ALL {
            assert_eq!(
                select(requirement(rung).required()),
                Ok(rung),
                "the minimum machine for {} selects something else",
                rung.name()
            );
        }
    }

    #[test]
    fn a_backend_that_satisfies_no_rung_is_refused() {
        // RFC 0080's decision, and the one `E3-B02b` inherits from `select`'s
        // signature: a machine that satisfies no rung is refused a compositor
        // rather than handed the floor. Two reports, and the second is the one
        // that matters — a machine with a CPU and no route to a screen at all is
        // not exotic, it is a headless box, and *the floor by default* would
        // have it flattening paths for a pipeline it does not have.
        for reported in [Capabilities::NONE, Capabilities::of(&[Capability::Cpu])] {
            let Err(refused) = select(reported) else {
                panic!("a backend reporting {} capabilities started a compositor", reported.count())
            };
            assert_eq!(refused.reported(), reported);
            for rung in Rung::ALL {
                assert!(
                    !refused.unmet(rung).is_empty(),
                    "{} is refused with nothing unmet, so it should have been selected",
                    rung.name()
                );
            }
        }
    }

    #[test]
    fn a_triangle_pipeline_that_can_present_takes_the_exact_rung() {
        // **The finding, made observable.** Rung 3 asks so little that a machine
        // with triangles *and* any way to put a finished image on the screen
        // takes the exact CPU raster rather than the approximate floor. That is
        // the fidelity ordering working as RFC 0080 argues it should — the floor
        // changes the picture, so it is last — and it is also why the floor's
        // reachable machine is a narrow one.
        //
        // If `E3-B02l` finds that every real fixed-function pipeline can also
        // blit, this assertion is still true and the floor is a rung nothing
        // selects; the answer then is a row edited in RFC 0080, not a predicate
        // edited here.
        let blitter = Capabilities::of(&[
            Capability::Cpu,
            Capability::ImagePresent,
            Capability::TrianglePipeline,
        ]);
        assert_eq!(select(blitter), Ok(Rung::CpuRaster));
        assert_eq!(Rung::CpuRaster.fidelity(), Fidelity::Exact);
    }

    #[test]
    fn the_ladder_is_not_monotone_in_what_the_machine_must_supply() {
        // RFC 0080: *no total order over hardware is ever computed, because none
        // exists.* Said here as an existence claim over the predicates — some
        // lower rung asks for something no rung above it asks for — because that
        // sentence is the reason `Rung::below` is one step rather than a search,
        // and a ladder which quietly became a staircase of capability would make
        // that design look like an oversight.
        let mut found = false;
        for (lower, rung) in Rung::ALL.into_iter().enumerate() {
            for upper in 0..lower {
                let above = requirement(Rung::ALL[upper]).required();
                if !requirement(rung).required().without(above).is_empty() {
                    found = true;
                }
            }
        }
        assert!(found, "every rung asks for a subset of what the rungs above ask for");
    }

    #[test]
    fn a_capability_is_one_word_in_one_place() {
        // Two capabilities spelled the same make `from_name` answer with the
        // earlier one, and a refusal naming the later one unreadable. Phrases
        // too: two capabilities whose table words are the same phrase are one
        // clause counted twice, and the coverage test above would be satisfied
        // by either of them.
        for (at, one) in Capability::ALL.iter().enumerate() {
            for other in &Capability::ALL[at + 1..] {
                assert_ne!(one.name(), other.name());
                assert_ne!(one.phrase(), other.phrase());
            }
            assert_eq!(Capability::from_name(one.name()), Some(*one));
        }
        assert_eq!(Capability::from_name("ray-tracing"), None);
    }

    #[test]
    fn a_set_reports_what_was_put_in_it_and_nothing_else() {
        // The bitset every predicate above rests on, and which nothing else in
        // this module would notice being wrong: a `with` that set the wrong bit
        // would give a plausible rung to the wrong machine.
        let mut set = Capabilities::NONE;
        assert!(set.is_empty());
        for capability in Capability::ALL {
            set = set.with(capability);
            assert!(set.has(capability));
        }
        assert_eq!(set.count() as usize, CAPABILITY_COUNT);
        assert!(set.includes(Capabilities::of(&Capability::ALL)));
        assert_eq!(set.without(set), Capabilities::NONE);
        assert!(set.iter().eq(Capability::ALL));
    }
}
