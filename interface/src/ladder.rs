// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The renderer fallback ladder: four rungs, decided before there is a machine
//! that cannot run the top one.
//!
//! # Why this is written now, with nothing to run it against
//!
//! Because the alternative is writing it later, and later means writing it in
//! front of hardware that has already failed. A ladder decided at that moment
//! is not a decision — it is a description of whatever the machine in the room
//! could do, with the reasoning supplied afterwards, and every system that has
//! ever shipped a "software fallback" arrived at one that way. `E3-B02`'s exit
//! asks for the ladder to be *exercised on hardware that cannot run the top
//! rung*, which is a test that can only be written against rungs that already
//! have names. RFC 0080 is the argument; this module is the vocabulary, and it
//! is deliberately the smallest thing that can hold it.
//!
//! # What a rung is, and what it is not
//!
//! A rung is a **renderer, chosen once, for as long as a compositor runs**. It
//! is not the per-frame effect degradation in section 10 of
//! `docs/design/ring-scene-boot.html`, which downgrades a blur inside a frame
//! that is already late and recovers on the next one. Those two mechanisms
//! share the word *fallback* and nothing else: one moves between renderers at a
//! component's start, the other moves between effects at a frame's deadline,
//! and a system that confused them would answer a missed frame by changing
//! rasterisers — which is the one response guaranteed to miss the next frame
//! too.
//!
//! RFC 0080 says a rung declares three things: what the machine must supply,
//! what it costs, and whether it changes the picture. **Two of the three are
//! types here and the third is not**, which is a gap worth stating rather than
//! leaving to be discovered. Expressing *what the machine must supply* needs a
//! vocabulary for what a backend reports about itself, and there is no backend;
//! inventing that vocabulary now would be guessing at what a GPU driver will
//! say, and a wrong guess frozen into this crate is worse than an absent one,
//! because the compositor would have to satisfy it. It becomes a type when
//! `E3-B02` lands something that answers the question, and until then the
//! requirement lives in RFC 0080's table, where it is prose somebody has to
//! read rather than a predicate something can evaluate.
//!
//! # Why each rung names a claim metric rather than carrying a number
//!
//! Because a cost written here would be a second copy of a number the registry
//! already owns, and a second copy is what `claims/README.md` exists to
//! prevent: the claim can go red, or move, and the constant in the source stays
//! confidently in place. So a rung carries the *name of the row* that prices it
//! — [`Rung::cost_metric`] — and `claims/0033-raster-cost-per-rung.toml` holds
//! the number, the threshold, the workload and the reproduction. No type and no
//! constant here states what a rung costs, and that is the intended amount.
//!
//! The names are the seam, so the names are checked against the file itself
//! rather than against a copy of it: the tests `include_str!` the claim and
//! compare its `[threshold]` keys to [`Rung::cost_metric`] in both directions.
//! That is a compile-time read, so it costs this crate no allocator, no `std`
//! and no filesystem at run time — and it means renaming a row in either file
//! alone is a build failure rather than a drift nobody notices until the day the
//! claim first runs. It reads the rows *in order*, so the ladder's order is the
//! claim's order and not a second opinion about it, and it reads one property of
//! the numbers: an allowance never tightens as the ladder descends. What it does
//! not read is whether any allowance is the *right* one. That needs a
//! distribution, which is `E3-B02`'s to produce.
//!
//! # The ordering is fidelity, and it is not a staircase of capability
//!
//! The rungs descend in what the picture is worth, not in what the machine must
//! have. The first three produce the **same image** by different arithmetic;
//! the floor produces a different one. That is why the floor is last despite
//! asking least of the CPU, and it is the single most load-bearing thing in
//! this module — a reader who reorders these variants by hardware requirement
//! has changed what the system is willing to put on a screen. That sentence was
//! held by nothing outside this file: `ALL` and [`Rung::below`] are two
//! hand-written lists checked against each other, so swapping two rungs in both
//! was green. The order is now checked against RFC 0080's table, which is where
//! `E3-D04`'s exit clause states it and is the one copy this module cannot edit
//! by accident. There is no rung every machine reaches, which RFC 0080 states as a decision rather than
//! conceding as a gap: a machine that satisfies no rung's requirement is
//! refused a compositor, because refusing is answerable and a compositor that
//! misses every frame is not.

/// The registry entry that prices every rung on this ladder.
///
/// Named here so that a reader who wants the numbers has one string to search
/// for, and so that the coupling between this module and `claims/` is a
/// declaration rather than a convention somebody has to notice.
///
/// It was a declaration nothing checked. This constant appeared in this file
/// only inside failure *messages*, so renaming the registry entry — the string
/// `ROUTES` and `cargo xtask claim` both key on — would have left it confidently
/// wrong while every test stayed green. `the_claim_named_here_is_the_file_read_here`
/// compares it to the claim's own `name` field.
pub const CLAIM: &str = "raster-cost-per-rung";

/// How many rungs there are.
///
/// Four is an argument and not a count — RFC 0080 refuses three and refuses
/// five, and the reasoning is there rather than here. What this constant is for
/// is [`Rung::ALL`]'s length.
///
/// **What the type system enforces about a fifth rung, stated exactly, because
/// this comment used to overstate it.** It said a fifth variant left out of
/// `ALL` fails to compile. It does not: `ALL` is a hand-written `[Self; 4]` and
/// stays a valid one, so a fifth variant with arms in the five `match`es below
/// compiles and is simply on no ladder — reachable by nothing, iterated by
/// nothing, and green. What the compiler does enforce is narrower and worth
/// having: every method here matches exhaustively, so a fifth variant with no
/// arm does not build, which makes *what it costs*, *how much of it runs on the
/// CPU*, *what it looks like* and *what is below it* four questions somebody has
/// to answer. `ALL`'s agreement with the enum is reached for one layer out, in
/// `all_is_every_rung_the_enum_has`, and that test states exactly how far its
/// reach goes — a test build rather than a crate build, and an arm somebody has
/// to write rather than an arm that has to be right.
pub const RUNGS: usize = 4;

/// Whether a rung draws the picture the scene describes, or an approximation
/// of it.
///
/// This is the distinction that orders the ladder, so it is a type rather than
/// a comment. Three rungs compute exact coverage and differ only in where the
/// arithmetic happens; the floor flattens paths to triangles and accepts
/// conflation where two shapes meet. A projection layer that could not tell
/// those apart would have no way to say *this machine is showing you something
/// slightly different*, and that sentence is owed to a user rather than
/// optional.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Fidelity {
    /// The same image the compute path would have produced, to the pixel.
    Exact,
    /// A visibly different image: sampled coverage, and seams where adjacent
    /// shapes meet. Cheap, correct in outline, wrong at the edges.
    Approximate,
}

/// How much of a frame's coverage arithmetic a rung puts on the CPU.
///
/// **This is not the third thing a rung declares.** It says nothing about what a
/// machine must supply — that is still RFC 0080's table and still prose, for the
/// reason the module comment gives. It is a property of the *renderer*, and it
/// is a type because `docs/design/ring-scene-boot.html` section 08 carries one
/// sentence that counts it: the narrowing of section 05's *the CPU raster
/// segment disappears entirely*. While that count lived only in prose, in three
/// files, the published one was wrong by a rung — it said three of four, and it
/// is two of four. A number in a design document that nothing can recompute is a
/// number that drifts on the first edit to the thing it describes, and this
/// ladder is the thing it describes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CpuCoverage {
    /// None of it: the device computes coverage. True of two rungs, by two
    /// different pieces of hardware — [`Rung::ComputePath`] reaches it with a
    /// compute shader, [`Rung::TessellatingFloor`] with a fixed-function
    /// triangle pipeline, and no machine is required to have both. The floor's CPU work is flattening paths into triangles, which
    /// is geometry and not coverage, and reading it as CPU rasterisation is the
    /// specific mistake this type exists to stop.
    None,
    /// The coarse raster's parallel prefix scan, and nothing else: the one stage
    /// that needs cooperating lanes, run on the processor that has none. Fine
    /// raster and present stay on the device.
    PrefixScan,
    /// All of it, with the device used for nothing but putting the finished
    /// image on the screen.
    Whole,
}

/// One rung of the fallback ladder, in descending order of fidelity.
///
/// The variants are the ladder. Their declaration order is `E3-D04`'s exit
/// clause and RFC 0080's table, and it is the order [`Rung::below`] walks — so
/// reordering them is not a cosmetic change, it is a change to what a machine
/// falls back *to*.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rung {
    /// The whole pipeline of section 08: encode on the CPU, then coarse
    /// raster, fine raster and present as compute on the GPU.
    ///
    /// This is the rung the design document argues for, and the only one whose
    /// target was derived somewhere other than the claim file: section 08's
    /// four stage costs, summed there and quoted into
    /// `claims/0033-raster-cost-per-rung.toml` with the tail allowance that sum
    /// does not carry. The number is not repeated here — it was, and a second
    /// copy of the registry's number in the module that argues against second
    /// copies is the stale constant this module exists to refuse. Every other
    /// rung exists because a machine cannot reach this one.
    ComputePath,
    /// The coarse raster's parallel prefix scan moves to the CPU; fine raster
    /// and present stay on the GPU.
    ///
    /// The rung for hardware that has compute shaders and cannot use them for
    /// the one stage that needs cooperating lanes. It is the rung most likely to
    /// be deleted, and RFC 0080 says on what observation — a comparison between
    /// this rung and [`Rung::CpuRaster`] *as measured on the machine that chose
    /// this one*, never between each rung and its own threshold, which are
    /// scaled from different arguments and say nothing about which renderer is
    /// faster. A middle rung that nothing lands on profitably is a seam being
    /// maintained for its own sake.
    Hybrid,
    /// Every stage on the CPU, with the GPU used for nothing but putting the
    /// finished image on the screen.
    ///
    /// Slow and exactly right. This is the rung that is worth having when a
    /// backend cannot be trusted rather than when it cannot be found, because
    /// the image is identical to the compute path's and only the clock differs.
    CpuRaster,
    /// Paths flattened to triangles and handed to a fixed-function pipeline.
    ///
    /// The floor: the only rung that changes what is on the screen, and the
    /// reason it is last rather than second despite being cheap. It is here so
    /// that a machine with a triangle rasteriser and no compute renders
    /// *something*.
    ///
    /// **What refutes it is a measurement on the hardware it is for, and its
    /// threshold is not that measurement.** `tessellated_us_x100` is bounded by
    /// section 09's latency chain — the same bound [`Rung::CpuRaster`] carries —
    /// because a machine running the floor still has to present a frame, and
    /// that is the only question a bound can ask on its own. The row used to be
    /// set at the *top* rung's cost so that a floor which was not cheaper
    /// refuted itself, which fires in the wrong direction: the floor exists for
    /// machines that cannot run rung 1, so that red could only ever be recorded
    /// on a machine where the floor is never selected, and a uniformly slow
    /// machine reddens both rows at once while the diagnosis reads the second as
    /// refutation. RFC 0080 now states the condition as a rung-to-rung
    /// comparison *on a machine that cannot start rung 1*: the floor at or above
    /// that machine's own `cpu_raster_us_x100` is a worse picture bought for no
    /// saving, and that is the rung failing to justify itself.
    TessellatingFloor,
}

impl Rung {
    /// Every rung, highest fidelity first.
    ///
    /// The order is the ladder's, not alphabetical and not by cost. A reader
    /// tempted to sort this by what each rung demands of the machine should
    /// read the module comment first: the ladder is not monotone in hardware
    /// and is not meant to be.
    pub const ALL: [Self; RUNGS] =
        [Self::ComputePath, Self::Hybrid, Self::CpuRaster, Self::TessellatingFloor];

    /// The word a log line, a state tree and a command line share.
    ///
    /// One spelling, in one place, because the rung a machine is on is a thing
    /// somebody will grep for across a boot log and a component's published
    /// tree, and two spellings of it is a question that cannot be answered by
    /// searching.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ComputePath => "compute-path",
            Self::Hybrid => "hybrid",
            Self::CpuRaster => "cpu-raster",
            Self::TessellatingFloor => "tessellating-floor",
        }
    }

    /// The row in [`CLAIM`] that states what this rung is allowed to cost.
    ///
    /// Every name ends in `_us_x100` — microseconds times one hundred — because
    /// RFC 0004 forbids a float in this tree and a frame cost is exactly the
    /// quantity somebody reaches for one to hold. The scale is in the name for
    /// the same reason: a bare `cost` is a number whose unit is obvious only to
    /// whoever wrote it down, and this one crosses from a renderer to a claim
    /// file to a design document.
    #[must_use]
    pub const fn cost_metric(self) -> &'static str {
        match self {
            Self::ComputePath => "compute_path_us_x100",
            Self::Hybrid => "hybrid_us_x100",
            Self::CpuRaster => "cpu_raster_us_x100",
            Self::TessellatingFloor => "tessellated_us_x100",
        }
    }

    /// How much of the frame's coverage arithmetic this rung puts on the CPU.
    ///
    /// The floor is the answer a reader gets wrong, so it is worth stating
    /// twice: the floor is [`CpuCoverage::None`]. Its triangles are rastered by
    /// fixed-function hardware and what the CPU does for it is flatten paths, so
    /// *the CPU raster segment returns below the top rung* is true of two of the
    /// three lower rungs and not of all three. Section 08's narrowing states
    /// that count, and `section_08_s_narrowing_counts_this_ladder` is where the
    /// two are held to each other.
    #[must_use]
    pub const fn cpu_coverage(self) -> CpuCoverage {
        match self {
            Self::ComputePath | Self::TessellatingFloor => CpuCoverage::None,
            Self::Hybrid => CpuCoverage::PrefixScan,
            Self::CpuRaster => CpuCoverage::Whole,
        }
    }

    /// Whether this rung draws the scene or an approximation of it.
    ///
    /// Three of four are [`Fidelity::Exact`], and the asymmetry is the whole
    /// shape of the ladder: descending through the first three costs time and
    /// changes nothing a user can see, and the last step changes the picture.
    /// A compositor that is willing to take the first three steps silently and
    /// unwilling to take the fourth one silently is behaving correctly, and
    /// this is the predicate it asks.
    #[must_use]
    pub const fn fidelity(self) -> Fidelity {
        match self {
            Self::ComputePath | Self::Hybrid | Self::CpuRaster => Fidelity::Exact,
            Self::TessellatingFloor => Fidelity::Approximate,
        }
    }

    /// The next rung down, or `None` at the floor.
    ///
    /// One step, deliberately, rather than *the best rung this machine can
    /// run*. A search would need every rung's requirement expressed as a
    /// predicate over a backend that does not exist yet, and inventing that
    /// vocabulary here would be guessing at what a GPU driver will report.
    /// Stepping needs nothing: whoever holds the backend tries a rung, fails to
    /// start it, and asks for the one below. `None` is a compositor that cannot
    /// run at all, which RFC 0080 makes a refusal rather than a degraded start.
    #[must_use]
    pub const fn below(self) -> Option<Self> {
        match self {
            Self::ComputePath => Some(Self::Hybrid),
            Self::Hybrid => Some(Self::CpuRaster),
            Self::CpuRaster => Some(Self::TessellatingFloor),
            Self::TessellatingFloor => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The claim file, read when this file is compiled.
    ///
    /// **What was here was a hand-copied array of the four metric names, and
    /// the reason given for copying them was false.** It said this crate is
    /// `#![no_std]` with no filesystem and no allocator, so there is nothing to
    /// read — true of run time and irrelevant, because `include_str!` reads at
    /// compile time and needs none of the three. And what the copied array
    /// compared was one list in this file against another list in this file,
    /// forty lines apart in one diff, while its comment claimed to be catching
    /// an edit made in another directory a week later. A test that cannot fail
    /// for the reason it states is worse than no test, because the exit it
    /// stands under reads as checked.
    ///
    /// The cost is real and is the intended one: touching the claim file
    /// rebuilds this crate. The ladder and the rows that price it are one
    /// decision written in two files — RFC 0080 — and a rebuild is the cheapest
    /// possible way for that to be a fact the toolchain knows.
    const CLAIM_FILE: &str = include_str!("../../claims/0033-raster-cost-per-rung.toml");

    /// RFC 0080, read when this file is compiled.
    ///
    /// Read for one thing only: the order of the rungs in its table. `ALL` and
    /// [`Rung::below`] are two hand-written lists, and every test here compared
    /// them *to each other* — so swapping two rungs in both left the whole suite
    /// green, and the module comment's claim that reordering the variants "is a
    /// change to what a machine falls back to" was guarded by nothing. The order
    /// is `E3-D04`'s exit clause and RFC 0080's table; the table is the copy that
    /// is not in this file.
    const RFC: &str =
        include_str!("../../docs/rfc/0080-the-fallback-ladder-is-decided-before-it-is-needed.md");

    /// The design document this ladder narrowed, read when this file is compiled.
    ///
    /// Also read for one thing: section 08's narrowing sentence counts how many
    /// rungs below the top one put coverage arithmetic back on the CPU, and a
    /// count in a published document that nothing recomputes is a count that
    /// goes stale on the first edit to the ladder. It went stale before it was
    /// published — it said three, and it is two. The cost is that editing the
    /// design document rebuilds this crate, which is the same cost the claim
    /// file already imposes and is paid for the same reason.
    const DESIGN: &str = include_str!("../../docs/design/ring-scene-boot.html");

    /// The clause of section 08's narrowing that this ladder decides.
    ///
    /// Held as a constant rather than written into an assertion so that the
    /// failure message can print the sentence a reader then has to go and fix.
    const NARROWING: &str =
        "two of the three rungs below it put coverage arithmetic back on the CPU";

    /// Every row of the claim's `[threshold]` table, as `(key, rest)`, in the
    /// order the file states them.
    ///
    /// The same walk `xtask`'s `threshold_rows` does, and this time actually the
    /// same: a `[` line *opens* the table when it reads `[threshold]` and closes
    /// it when it reads anything else, so a second `[threshold]` table further
    /// down the file is read by both. What was here stopped at the first
    /// following header — `skip_while` then `take_while` — so a row appended
    /// after `[hardware]` was gated on by `cargo xtask lint` and invisible to
    /// every assertion below, which is the drift these tests exist to catch read
    /// from the one end they could not see. The comment already claimed this
    /// walk was the same walk; now it is.
    ///
    /// It is a walk and not a TOML parser because a parser needs an allocator
    /// this crate does not have. If the claim ever stops being readable this
    /// way, `[threshold]` is missing or has moved, and every assertion below
    /// goes red rather than silently passing over nothing.
    fn threshold_rows(text: &str) -> impl Iterator<Item = (&str, &str)> {
        text.lines()
            .scan(false, |inside, line| {
                let trimmed = line.trim_start();
                if trimmed.starts_with('[') {
                    *inside = trimmed.trim_end() == "[threshold]";
                    return Some(None);
                }
                if !*inside || trimmed.starts_with('#') {
                    return Some(None);
                }
                Some(trimmed.split_once('=').map(|(key, rest)| (key.trim(), rest)))
            })
            .flatten()
            .filter(|(key, _)| !key.is_empty())
    }

    /// The `max` a row states, read the way `xtask`'s `stated` reads one.
    ///
    /// Digits are accumulated and `_` skipped rather than stripped out of an
    /// owned string, because this crate has no allocator — and skipped at all
    /// because `5_000` *is* 5000 to TOML and is not to `u64::from_str`. That
    /// exact defect once read `claims/0002`'s gate as no bound at all, and a
    /// second reader of these files repeating it would report a ladder whose
    /// allowances it could not see as one whose allowances were fine.
    fn stated_max(rest: &str) -> Option<u64> {
        let after = rest.split_once("max")?.1.trim_start().strip_prefix('=')?;
        let (mut value, mut digits) = (0u64, 0usize);
        for byte in after.trim_start().bytes() {
            match byte {
                b'0'..=b'9' => {
                    value = value * 10 + u64::from(byte - b'0');
                    digits += 1;
                }
                b'_' => {}
                _ => break,
            }
        }
        (digits > 0).then_some(value)
    }

    #[test]
    fn the_claim_named_here_is_the_file_read_here() {
        // `CLAIM` is what `ROUTES` and `cargo xtask claim` key on, and it used to
        // appear in this file only inside failure messages — a constant no
        // assertion compared to anything, in the module that argues against
        // second copies. Renaming the registry entry would have left it wrong
        // and green.
        let stated = CLAIM_FILE
            .lines()
            .map(str::trim_start)
            .filter_map(|line| line.strip_prefix("name"))
            .filter_map(|rest| rest.trim_start().strip_prefix('='))
            .map(|rest| rest.trim().trim_matches('"'))
            .next();
        assert_eq!(stated, Some(CLAIM), "this module prices its rungs in `{CLAIM}`");
    }

    #[test]
    fn the_claim_prices_every_rung_in_order_and_nothing_else() {
        // Both directions and the order, in one walk, because they are one
        // property: a fifth `[threshold]` row is a cost the registry gates on and
        // this ladder cannot name; a rung with no row is a rung nothing prices;
        // a row spelled twice is a rename that was started and not finished; and
        // a table whose rows are in a different order from `ALL` is the claim and
        // the module disagreeing about which rung is the top one while every
        // set-wise check stays green.
        let mut rows = threshold_rows(CLAIM_FILE).map(|(key, _)| key);
        for rung in Rung::ALL {
            let Some(key) = rows.next() else {
                panic!(
                    "{CLAIM} runs out of rows at {}, which prices itself as `{}`",
                    rung.name(),
                    rung.cost_metric()
                )
            };
            assert_eq!(key, rung.cost_metric(), "{CLAIM}'s rows are not this ladder's rungs");
        }
        assert_eq!(rows.next(), None, "{CLAIM} gates on a row no rung on this ladder claims");
    }

    #[test]
    fn no_rung_is_allowed_less_than_the_rung_above_it() {
        // **The direction a fallback ladder's allowances have to run, and the
        // one the floor's row was written against.** Every step down this ladder
        // is taken by a machine that has less than the machine above it had, so a
        // lower rung bounded more tightly than a higher one states a bound that
        // only a machine which never takes the step could meet — and its red can
        // then be recorded only where the rung is not selected. `tessellated_us_x100`
        // was 315 000 against `cpu_raster_us_x100`'s 840 000 for exactly that
        // reason: the floor was being refuted by the cost of a rung its own
        // hardware cannot run. This does not check that any allowance is right;
        // it checks that no allowance points at hardware the rung is not for.
        let mut above: Option<(Rung, u64)> = None;
        for rung in Rung::ALL {
            let row = threshold_rows(CLAIM_FILE).find(|(key, _)| *key == rung.cost_metric());
            let Some((_, rest)) = row else {
                panic!("{CLAIM} states no row for `{}`", rung.cost_metric())
            };
            let Some(allowance) = stated_max(rest) else {
                panic!("`{}` states no `max` this can read", rung.cost_metric())
            };
            if let Some((upper, allowed)) = above {
                assert!(
                    allowance >= allowed,
                    "{} allows {} and {} above it allows {}; this ladder tightens downward",
                    rung.name(),
                    allowance,
                    upper.name(),
                    allowed
                );
            }
            above = Some((rung, allowance));
        }
    }

    #[test]
    fn the_rfc_s_table_states_this_ladder_s_order() {
        // The module comment calls the declaration order the most load-bearing
        // thing here, and nothing outside this file held it: `ALL` and `below`
        // are two hand-written lists checked against each other, so swapping two
        // rungs in both was green. RFC 0080's table is the other copy, and it is
        // the one `E3-D04`'s exit clause names. First occurrence of each metric,
        // because every later mention in that document is prose about a rung the
        // table has already named.
        let mut previous: Option<(Rung, usize)> = None;
        for rung in Rung::ALL {
            let Some(at) = RFC.find(rung.cost_metric()) else {
                panic!("RFC 0080 never names `{}`", rung.cost_metric())
            };
            if let Some((upper, before)) = previous {
                assert!(at > before, "RFC 0080 puts {} above {}", rung.name(), upper.name());
            }
            previous = Some((rung, at));
        }
    }

    #[test]
    fn section_08_s_narrowing_counts_this_ladder() {
        // The one edit `E3-D04` made to the document `claims/0033` names as its
        // owner is a sentence that counts this ladder, and for two review rounds
        // it counted it wrong — three of the four rungs rastering on the CPU,
        // where two do. Both halves are asserted: the count the ladder produces,
        // and the sentence the document carries. Change either alone and this
        // goes red naming the other.
        let below_the_top =
            Rung::ALL.iter().skip(1).filter(|r| r.cpu_coverage() != CpuCoverage::None).count();
        assert_eq!(
            below_the_top, 2,
            "section 08 says `{NARROWING}`, and this ladder now has {below_the_top}"
        );
        assert_eq!(
            Rung::TessellatingFloor.cpu_coverage(),
            CpuCoverage::None,
            "the floor's CPU work is flattening, not rastering, and section 08 counts on it"
        );
        assert!(DESIGN.contains(NARROWING), "section 08 no longer says `{NARROWING}`");
    }

    #[test]
    fn no_two_rungs_share_a_cost() {
        // A ladder whose rungs report into one row is a ladder with one number,
        // and the claim's table would be green while saying nothing. Names and
        // metrics both, because a duplicated *name* makes a boot log ambiguous
        // about which renderer is running, which is the same defect read by a
        // human instead of by a threshold.
        for (i, a) in Rung::ALL.iter().enumerate() {
            for b in &Rung::ALL[i + 1..] {
                assert_ne!(a.cost_metric(), b.cost_metric(), "{} and {}", a.name(), b.name());
                assert_ne!(a.name(), b.name());
            }
        }
    }

    #[test]
    fn every_cost_states_its_scale() {
        // RFC 0004: no float, so every one of these is an integer, and an
        // integer whose unit is not in its name is the number a later reader
        // divides by the wrong thing.
        for rung in Rung::ALL {
            assert!(rung.cost_metric().ends_with("_us_x100"), "{}", rung.cost_metric());
        }
    }

    /// Whether two rungs are one rung, decidable while this file compiles.
    ///
    /// A match over the pair rather than the derived `PartialEq`, because the
    /// membership check below has to hold at compile time and a derived `eq` is
    /// not `const`. There is no wildcard arm and that is the load-bearing part:
    /// a fifth variant makes every pair that mentions it answer *false*, so the
    /// const assertion it is given goes red rather than passing by default.
    const fn same(a: Rung, b: Rung) -> bool {
        matches!(
            (a, b),
            (Rung::ComputePath, Rung::ComputePath)
                | (Rung::Hybrid, Rung::Hybrid)
                | (Rung::CpuRaster, Rung::CpuRaster)
                | (Rung::TessellatingFloor, Rung::TessellatingFloor)
        )
    }

    /// Whether [`Rung::ALL`] holds this rung.
    const fn on_ladder(rung: Rung) -> bool {
        let mut at = 0;
        while at < RUNGS {
            if same(Rung::ALL[at], rung) {
                return true;
            }
            at += 1;
        }
        false
    }

    #[test]
    fn all_is_every_rung_the_enum_has() {
        // **This is the closedness guard, and nothing else in this module is
        // one.** Every other test here either iterates `Rung::ALL` or walks
        // `below` from the top, and neither can see a variant `ALL` omits — so a
        // `Rung::Fifth` with arms in `name`, `cost_metric`, `cpu_coverage`,
        // `fidelity` and `below`, left out of the array, compiled and passed.
        // A rung on no ladder is reachable by no machine, which is the one
        // defect this file exists to make impossible.
        //
        // The guard cannot be a loop; it is this match. The match is exhaustive
        // over `Rung`, so a fifth variant does not build until somebody writes an
        // arm, and every arm is a `const` block, so the arm's assertion is
        // evaluated when this file is compiled rather than when a loop reaches
        // it. `ALL` stays a hand-written array, and what the compiler adds is
        // that no new variant can reach the ladder's tests without an edit at
        // this line.
        //
        // *The edit that makes this go red:* add `Rung::Fifth` to the enum with
        // arms in the five methods and leave `ALL` and `RUNGS` alone. With no arm
        // here the match is not exhaustive and the test build fails. Widening
        // `RUNGS` to 5 without extending `ALL` does not rescue it either: the
        // array's length is `RUNGS`, so that is also a build failure.
        //
        // *And what it does not force, because this comment used to say it did.*
        // It claimed an arm "copied from its neighbours" goes red at
        // const-evaluation. A literally copied one does not:
        // `Rung::Fifth => const { assert!(on_ladder(Rung::TessellatingFloor)) }`
        // compiles and passes, and so does `Rung::Fifth => ()`. Exhaustiveness
        // forces an arm to be *written* at the line that says every variant is on
        // the ladder; it does not force the arm to name its own variant. What
        // would force that is generating the enum and `ALL` from one macro list,
        // which this module has not done — so the reach of this guard is a diff
        // somebody has to make here, and that sentence is the whole of the claim
        // made for it.
        fn every_variant_is_on_ladder(rung: Rung) {
            match rung {
                Rung::ComputePath => const { assert!(on_ladder(Rung::ComputePath)) },
                Rung::Hybrid => const { assert!(on_ladder(Rung::Hybrid)) },
                Rung::CpuRaster => const { assert!(on_ladder(Rung::CpuRaster)) },
                Rung::TessellatingFloor => const { assert!(on_ladder(Rung::TessellatingFloor)) },
            }
        }

        for rung in Rung::ALL {
            every_variant_is_on_ladder(rung);
        }
    }

    #[test]
    fn the_ladder_descends_through_every_rung_and_stops() {
        // Bounded rather than `while let`, because the defect this test is
        // nearest to — a `below` that steps into a cycle — is the one an
        // unbounded walk cannot report. It hangs the suite instead of failing
        // it, and a hung suite is read as an infrastructure fault by whoever
        // sees it rather than as this file being wrong.
        let mut rung = Rung::ComputePath;
        let mut visited = 1;
        for _ in 0..RUNGS {
            let Some(next) = rung.below() else { break };
            rung = next;
            visited += 1;
        }
        assert_eq!(visited, RUNGS, "descent reached {visited} rung(s) of {RUNGS}");
        assert_eq!(rung, Rung::TessellatingFloor, "the descent ends somewhere but the floor");
        assert!(rung.below().is_none(), "the floor steps down, so this ladder has no bottom");
    }

    #[test]
    fn descending_agrees_with_the_declared_order() {
        // Two statements of the ladder — the variant order and the step — and
        // they are checked against each other because they are edited
        // separately. A new rung inserted into `ALL` without a `below` arm
        // would otherwise be a rung nothing can reach. What this cannot see is
        // the two lists being reordered together, which is green here and red
        // in `the_rfc_s_table_states_this_ladder_s_order`.
        for pair in Rung::ALL.windows(2) {
            let (upper, lower) = (pair[0], pair[1]);
            let step = upper.below();
            assert_eq!(step, Some(lower), "{} does not step to {}", upper.name(), lower.name());
        }
    }

    #[test]
    fn only_the_floor_changes_the_picture() {
        // The property the ordering rests on. If a second rung ever becomes
        // approximate, the ladder has stopped being *three ways to draw one
        // image, then a cheaper image* — and RFC 0080's argument for why the
        // floor is last has to be made again rather than assumed.
        let approximate: usize =
            Rung::ALL.iter().filter(|r| r.fidelity() == Fidelity::Approximate).count();
        assert_eq!(approximate, 1);
        assert_eq!(Rung::TessellatingFloor.fidelity(), Fidelity::Approximate);
    }
}
