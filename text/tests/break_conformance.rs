// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E3-B03f`'s exit: UAX #29's and UAX #14's own conformance files, run over
//! `f_text::grapheme` and `f_text::line`, and the text path's breaks held to
//! the clusters over both of them.
//!
//! # The levels, named here rather than implied by the code
//!
//! **`GraphemeBreakTest.txt`: extended grapheme clusters, untailored — every
//! rule of UAX #29 section 3.1.1, GB1 to GB999, GB9c included.**
//! **`LineBreakTest.txt`: UAX #14's default line breaking algorithm,
//! untailored — LB1's default resolution, then LB2 to LB31.** Both are printed
//! on every run as [`Corpus::level`] spells them. *Default* is each file's own
//! word for what it tests (`Default Grapheme_Cluster_Break Test`, `Default
//! Line_Break Test`), and neither header puts any rule out of scope — unlike
//! UAX #9's two files, which name P1, L3 and L4 — so [`OUTSIDE_THE_LEVEL`] is
//! empty and an exclusion has nothing it may cite. A case that fails is
//! therefore a defect or a renamed level, never an entry in [`EXCLUDED`].
//!
//! # What a case is held to
//!
//! Its marks, position by position; for `LineBreakTest.txt`, also which of
//! its breaks are **mandatory**, because the file writes UAX #14's `!` as `÷`
//! and an algorithm that allowed a line to run on past a line separator would
//! otherwise pass it whole. That half is derived by [`mandatory`] from the
//! `Line_Break` property as LB3 to LB5 state it, not from `f_text::line`'s
//! answer. And the rule: every position the file labels is decided by the rule
//! of this tree's module that [`ARMS`] maps the label to, so the right answer
//! for the wrong reason is red.
//!
//! # Which arms the files reach
//!
//! Each file's comment names, in brackets, the rule that decides each position
//! — `[25.14]` is the fourteenth part of LB25. [`ARMS`] is every such label
//! the specification's text has, with its rule and its part in words, and the
//! run prints how many positions each file gives each label. An arm neither
//! file reaches is red unless it names a unit test in `text/src` that argues
//! it from the rule's text; so is a label the file uses that [`ARMS`] does not
//! know, and a name that is not a test. The labels only go as fine as the
//! file's numbering: an alternative inside one part — LB15b's list of what may
//! follow a closing quote — is not a label of its own, and the unit tests an
//! arm names are where those are argued one by one.
//!
//! # What is excluded, and where a reader finds it
//!
//! [`EXCLUDED`], and it is empty. It is checked data, as in
//! `bidi_conformance.rs`: a failing case not listed is red, a listed case that
//! passes is red, a listed case the file does not hold is red, and an entry
//! citing a rule the level includes — which, with nothing outside the level,
//! is any entry — is red. A decoy entry is fed to those checks on every run,
//! because over an empty list they could not otherwise fail.
//!
//! # No break falls inside a cluster: what that sentence is taken to mean
//!
//! **The text path's line-break opportunities are UAX #14 opportunities that
//! are also UAX #29 extended grapheme cluster boundaries, and no others; and
//! the ones it calls mandatory are LB3 to LB5's.** Held over every case of both
//! files and over the argued eight of `f_text::corpus`, by four checks on each
//! text: every position `f_text::line::opportunities` yields is a cluster
//! boundary; every one is an opportunity of the untailored algorithm; every
//! untailored opportunity at a cluster boundary is yielded, so the filter
//! removes what it must and nothing else; and its `mandatory` is [`mandatory`]'s.
//! The clusters are `f_text::grapheme`'s, which the first file holds to the
//! specification.
//!
//! It is not vacuous, and the run says so: the untailored algorithm puts
//! [`INSIDE_A_CLUSTER`] of `LineBreakTest.txt`'s own opportunities inside a
//! cluster, printed by rule with instances by line. The assertion's silence is
//! controlled three ways, and all three go through the one [`Run::hold`] the
//! text path goes through and report through its one branch — so a report
//! silenced or narrowed there is red, and not only an assertion disarmed
//! beneath it. The untailored breaks of every case of that file are sent in
//! place of the text path, and what `hold` reports for them is drained and
//! counted: it must be exactly [`INSIDE_A_CLUSTER`] positions inside a
//! cluster and nothing else. The count of texts held is made inside `hold`,
//! after the judgement, for a path that reached the end of the text, and must
//! equal the cases run, so a run that never consumed the text path is red.
//! And five wrong answers are sent on every run — a break inside a cluster, a
//! break UAX #14 does not make, a lost opportunity, a mandatory break not
//! called mandatory, and a break called mandatory that is not — and each must
//! draw exactly one finding with exactly its fault, and the right answer none.
//!
//! The same holds for the rule labels: each position counts towards its
//! label's line in the coverage table only when the rule this tree applied
//! there is the one the label names, and the comparison is fed a wrong rule
//! and an unknown label on every run and must refuse both.
//!
//! # The obligations `bidi_conformance.rs` carries, carried here
//!
//! A run of zero is red; each file must yield the cases [`Corpus::cases`]
//! pins and the data lines its own `# Lines:` states; a line that does not
//! parse is a failure; a missing file fails by name with the command that
//! restores it (RFC 0114). `f-text` is in `cargo xtask test-host` on the arm
//! runner, and every answer here is a boolean per position.
//!
//! # A set paragraph ends its lines only where the text path offers
//!
//! `f_text::paragraph` promises that every line it sets ends at a position
//! `f_text::line::opportunities` offers over the *whole* text (`Line::end`),
//! and the promise is where a paragraph split can go wrong — a CR LF split in
//! two, or a slice's end offered by LB3 where the text has no break. So every
//! case's text of both files is also set, twice, with a seat of half an em a
//! scalar: in a measure of one pixel, where no segment fits and **the line
//! ends must be exactly the opportunities**; and in one no line fills, where
//! **every line end must be offered and every mandatory one taken**. The lines
//! must tile the text in both. The one refusal allowed is
//! `Unset::SeparatorMidLine`, for a text whose U+001C to U+001E ends a
//! paragraph where UAX #14 offers nothing, and it is counted. This is here and
//! not in a file of its own because this file is already the `IMPORT_READERS`
//! row that may read both corpora (RFC 0114), and a second reader of the same
//! files would be a second row for the same bytes.
//!
//! ```text
//! cargo test -p f-text --test break_conformance
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::exit;

use f_text::bidi::BaseDirection;
use f_text::corpus::Script;
use f_text::grapheme::{Clusters, cluster_boundaries};
use f_text::line::{Decision, Outcome, decisions, opportunities};
use f_text::line_break::{LineBreak, UNICODE_VERSION};
use f_text::metric::{Advance, NotAMetric, Px, Scale};
use f_text::paragraph::{Setting, Unset};
use f_text::property::line_break;

/// The name this target answers to when cargo's harness protocol asks.
const TEST_NAME: &str = "break_conformance";

/// The import both files are in, relative to the workspace root.
const CORPUS_DIR: &str = "third_party/unicode";

/// Failures printed in full before the rest are only counted. Unit: cases.
const SHOWN: usize = 20;

/// Instances of an opportunity inside a cluster printed on every run, per
/// rule that allowed it. Unit: positions.
const INSTANCES: usize = 2;

/// `LineBreakTest.txt`'s opportunities that fall inside an extended grapheme
/// cluster. Unit: positions.
///
/// A number about the specification's two algorithms rather than about this
/// implementation, written down so that it is read on every run: it is the
/// reason `f_text::line::opportunities` filters, and the control that the
/// within-cluster check can find what it looks for. A Unicode version may move
/// it, and moving it is a line of the diff that moves the version.
const INSIDE_A_CLUSTER: usize = 1388;

/// The two modules' sources, read for the names of the tests [`ARMS`] cites —
/// a citation of a test that does not exist is a citation of nothing.
const SOURCES: [(&str, &str); 2] =
    [("line", include_str!("../src/line.rs")), ("grapheme", include_str!("../src/grapheme.rs"))];

/// The two files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Corpus {
    /// UAX #29's extended grapheme cluster boundaries.
    GraphemeBreakTest,
    /// UAX #14's line-break opportunities.
    LineBreakTest,
}

impl Corpus {
    const ALL: [Self; 2] = [Self::GraphemeBreakTest, Self::LineBreakTest];

    const fn file(self) -> &'static str {
        match self {
            Self::GraphemeBreakTest => "auxiliary/GraphemeBreakTest.txt",
            Self::LineBreakTest => "auxiliary/LineBreakTest.txt",
        }
    }

    /// The level this run holds the implementation to, in words, as printed.
    const fn level(self) -> &'static str {
        match self {
            Self::GraphemeBreakTest => {
                "extended grapheme clusters (UAX #29 section 3.1.1), untailored: GB1-GB5, \
                 GB6-GB8, GB9, GB9a, GB9b, GB9c, GB11, GB12-GB13, GB999"
            }
            Self::LineBreakTest => {
                "the default line breaking algorithm (UAX #14 section 6), untailored: LB1's \
                 default resolution, then LB2-LB31 with LB8a, LB12a, LB15a-LB15d, LB19a, LB20a, \
                 LB21a, LB21b, LB23a, LB28a, LB30a and LB30b; mandatory breaks by LB3-LB5"
            }
        }
    }

    /// The cases the file holds, as this run counts them: one per data line.
    /// Unit: cases.
    ///
    /// Written down for `bidi_conformance.rs`'s reason: a line deleted from
    /// the file is red here and not only in `cargo xtask lint`'s hash check.
    /// Each file also states its own count, `# Lines:`, and is held to it.
    const fn cases(self) -> usize {
        match self {
            Self::GraphemeBreakTest => 766,
            Self::LineBreakTest => 19_338,
        }
    }
}

/// One arm of a rule as a conformance file labels it.
struct Arm {
    corpus: Corpus,
    /// The label the file's comment prints in brackets, as it prints it —
    /// `25.1` is the tenth part of LB25, not the first.
    label: &'static str,
    /// The rule of `f_text::line` or `f_text::grapheme` that must decide every
    /// position the file gives this label, by its name there.
    rule: &'static str,
    /// The part, in the specification's notation.
    part: &'static str,
    /// Unit tests, as `module::name`, that argue this arm — or alternatives
    /// inside it the file does not reach — from the rule's text. Required when
    /// the file gives the label no position.
    argued: &'static [&'static str],
}

const fn lb(
    label: &'static str,
    rule: &'static str,
    part: &'static str,
    argued: &'static [&'static str],
) -> Arm {
    Arm { corpus: Corpus::LineBreakTest, label, rule, part, argued }
}

const fn gb(
    label: &'static str,
    rule: &'static str,
    part: &'static str,
    argued: &'static [&'static str],
) -> Arm {
    Arm { corpus: Corpus::GraphemeBreakTest, label, rule, part, argued }
}

const MANDATORY: &str = "line::a_hard_line_break_is_mandatory";
const LB15A: &str = "line::lb15a_holds_after_an_opening_quote_in_every_context";
const LB15B: &str = "line::lb15b_holds_before_a_closing_quote_before_every_follower";
const LB19A: &str = "line::lb19a_holds_after_a_quote_before_a_character_that_is_not_east_asian";
const LB20A: &str = "line::lb20a_holds_after_a_word_initial_hyphen_in_every_context";
const LB21A: &str = "line::lb21a_holds_after_a_hebrew_hyphen_of_either_class";
const LB25_OPEN: &str = "line::lb25_looks_past_an_opening_bracket_and_a_decimal_mark";
const LB25_CLOSED: &str = "line::lb25_ends_a_number_at_its_closing_bracket";
const LB28A: &str =
    "line::lb28a_holds_after_a_virama_before_a_consonant_and_not_before_an_aksara_start";
const LB30: &str = "line::lb30_lets_a_halfwidth_bracket_break_after_a_letter";
const GB9C: &str = "grapheme::gb9c_ends_at_a_scalar_outside_the_conjunct";
const GB11: &str = "grapheme::gb11_needs_the_joiner_directly_before_the_pictograph";

/// Every arm the two files can label, in the specification's order.
///
/// The files print only the labels they use, so a label with no position is
/// placed here by the order of the parts in the rule's text: LB25's fifteen
/// parts are 25.01 to 25.15 in the order UAX #14 writes them, which is how
/// 25.01, 25.02, 25.07, 25.08 and 25.11 — the five the file never uses — get
/// their numbers. Where the file's numbering skips (23.02 with no 23.01) the
/// skip is the file's, and nothing is invented to fill it. 30.13 is the one
/// label on a break a rule does not state: `(RI RI)* RI ÷ RI`, LB30a's
/// complement, which this tree's LB31 decides.
///
/// *What would reverse a row:* a Unicode version whose file labels a position
/// this table does not name, which is red, or maps a label to another rule.
const ARMS: &[Arm] = &[
    lb("0.3", "LB3", "! eot", &[]),
    lb("4.0", "LB4", "BK !", &[MANDATORY]),
    lb("5.01", "LB5", "CR × LF", &[MANDATORY]),
    lb("5.02", "LB5", "CR !", &[MANDATORY]),
    lb("5.03", "LB5", "LF !", &[MANDATORY]),
    lb("5.04", "LB5", "NL !", &[MANDATORY]),
    lb("6.0", "LB6", "× (BK | CR | LF | NL)", &[MANDATORY]),
    lb("7.01", "LB7", "× SP", &[]),
    lb("7.02", "LB7", "× ZW", &[]),
    lb("8.0", "LB8", "ZW SP* ÷", &[]),
    lb("8.1", "LB8a", "ZWJ ×", &[]),
    lb("9.0", "LB9", "X (CM | ZWJ)* is X", &[]),
    lb("11.01", "LB11", "× WJ", &[]),
    lb("11.02", "LB11", "WJ ×", &[]),
    lb("12.0", "LB12", "GL ×", &[]),
    lb("12.1", "LB12a", "[^SP BA HY HH] × GL", &[]),
    lb("13.01", "LB13", "× EX", &[]),
    lb("13.02", "LB13", "× CL", &[]),
    lb("13.03", "LB13", "× CP", &[]),
    lb("13.04", "LB13", "× SY", &[]),
    lb("14.0", "LB14", "OP SP* ×", &[]),
    lb(
        "15.11",
        "LB15a",
        "(sot | BK | CR | LF | NL | OP | QU | GL | SP | ZW) [Pi&QU] SP* ×",
        &[LB15A],
    ),
    lb(
        "15.21",
        "LB15b",
        "× [Pf&QU] (SP | GL | WJ | CL | QU | CP | EX | IS | SY | BK | CR | LF | NL | ZW | eot)",
        &[LB15B],
    ),
    lb("15.3", "LB15c", "SP ÷ IS NU", &[]),
    lb("15.4", "LB15d", "× IS", &[]),
    lb("16.0", "LB16", "(CL | CP) SP* × NS", &[]),
    lb("17.0", "LB17", "B2 SP* × B2", &[]),
    lb("18.0", "LB18", "SP ÷", &[LB15A, LB15B]),
    lb("19.01", "LB19", "× [QU - Pi]", &[]),
    lb("19.02", "LB19", "[QU - Pf] ×", &[]),
    lb("19.1", "LB19a", "[^$EastAsian] × QU", &[]),
    lb("19.11", "LB19a", "× QU ([^$EastAsian] | eot)", &[]),
    lb("19.12", "LB19a", "QU × [^$EastAsian]", &[LB19A]),
    lb("19.13", "LB19a", "(sot | [^$EastAsian]) QU ×", &[]),
    lb("20.01", "LB20", "÷ CB", &[]),
    lb("20.02", "LB20", "CB ÷", &[]),
    lb(
        "20.1",
        "LB20a",
        "(sot | BK | CR | LF | NL | SP | ZW | CB | GL) (HY | HH) × (AL | HL)",
        &[LB20A],
    ),
    lb("21.01", "LB21", "× BA", &[]),
    lb("21.02", "LB21", "× HH", &[]),
    lb("21.03", "LB21", "× HY", &[]),
    lb("21.04", "LB21", "× NS", &[]),
    lb("21.05", "LB21", "BB ×", &[]),
    lb("21.1", "LB21a", "HL (HY | HH) × [^HL]", &[LB21A]),
    lb("21.2", "LB21b", "SY × HL", &[]),
    lb("22.0", "LB22", "× IN", &[]),
    lb("23.02", "LB23", "(AL | HL) × NU", &[]),
    lb("23.03", "LB23", "NU × (AL | HL)", &[]),
    lb("23.12", "LB23a", "PR × (ID | EB | EM)", &[]),
    lb("23.13", "LB23a", "(ID | EB | EM) × PO", &[]),
    lb("24.02", "LB24", "(PR | PO) × (AL | HL)", &[]),
    lb("24.03", "LB24", "(AL | HL) × (PR | PO)", &[]),
    lb("25.01", "LB25", "NU (SY | IS)* CL × PO", &[LB25_CLOSED]),
    lb("25.02", "LB25", "NU (SY | IS)* CP × PO", &[LB25_CLOSED]),
    lb("25.03", "LB25", "NU (SY | IS)* CL × PR", &[]),
    lb("25.04", "LB25", "NU (SY | IS)* CP × PR", &[LB25_CLOSED]),
    lb("25.05", "LB25", "NU (SY | IS)* × PO", &[]),
    lb("25.06", "LB25", "NU (SY | IS)* × PR", &[]),
    lb("25.07", "LB25", "PO × OP NU", &[LB25_OPEN]),
    lb("25.08", "LB25", "PO × OP IS NU", &[LB25_OPEN]),
    lb("25.09", "LB25", "PO × NU", &[]),
    lb("25.1", "LB25", "PR × OP NU", &[LB25_OPEN]),
    lb("25.11", "LB25", "PR × OP IS NU", &[LB25_OPEN]),
    lb("25.12", "LB25", "PR × NU", &[]),
    lb("25.13", "LB25", "HY × NU", &[]),
    lb("25.14", "LB25", "IS × NU", &[]),
    lb("25.15", "LB25", "NU (SY | IS)* × NU", &[LB25_CLOSED]),
    lb("26.01", "LB26", "JL × (JL | JV | H2 | H3)", &[]),
    lb("26.02", "LB26", "(JV | H2) × (JV | JT)", &[]),
    lb("26.03", "LB26", "(JT | H3) × JT", &[]),
    lb("27.01", "LB27", "(JL | JV | JT | H2 | H3) × PO", &[]),
    lb("27.02", "LB27", "PR × (JL | JV | JT | H2 | H3)", &[]),
    lb("28.0", "LB28", "(AL | HL) × (AL | HL)", &[]),
    lb("28.11", "LB28a", "AP × (AK | ◌ | AS)", &[]),
    lb("28.12", "LB28a", "(AK | ◌ | AS) × (VF | VI)", &[]),
    lb("28.13", "LB28a", "(AK | ◌ | AS) VI × (AK | ◌)", &[LB28A]),
    lb("28.14", "LB28a", "(AK | ◌ | AS) × (AK | ◌ | AS) VF", &[]),
    lb("29.0", "LB29", "IS × (AL | HL)", &[]),
    lb("30.01", "LB30", "(AL | HL | NU) × [OP - $EastAsian]", &[LB30]),
    lb("30.02", "LB30", "[CP - $EastAsian] × (AL | HL | NU)", &[]),
    lb("30.11", "LB30a", "sot (RI RI)* RI × RI", &[]),
    lb("30.12", "LB30a", "[^RI] (RI RI)* RI × RI", &[]),
    lb("30.13", "LB31", "(RI RI)* RI ÷ RI", &[]),
    lb("30.21", "LB30b", "EB × EM", &[]),
    lb("30.22", "LB30b", "[ExtPict&Cn] × EM", &[]),
    lb("999.0", "LB31", "ALL ÷; ÷ ALL", &[LB19A, LB20A, LB25_OPEN, LB25_CLOSED, LB28A, LB30]),
    gb("0.2", "GB1", "sot ÷", &[]),
    gb("0.3", "GB2", "÷ eot", &[]),
    gb("3.0", "GB3", "CR × LF", &[]),
    gb("4.0", "GB4", "(Control | CR | LF) ÷", &[]),
    gb("5.0", "GB5", "÷ (Control | CR | LF)", &[]),
    gb("6.0", "GB6", "L × (L | V | LV | LVT)", &[]),
    gb("7.0", "GB7", "(LV | V) × (V | T)", &[]),
    gb("8.0", "GB8", "(LVT | T) × T", &[]),
    gb("9.0", "GB9", "× (Extend | ZWJ)", &[]),
    gb("9.1", "GB9a", "× SpacingMark", &[]),
    gb("9.2", "GB9b", "Prepend ×", &[]),
    gb(
        "9.3",
        "GB9c",
        "InCB=Consonant [Extend Linker]* Linker [Extend Linker]* × Consonant",
        &[GB9C],
    ),
    gb("11.0", "GB11", "ExtPict Extend* ZWJ × ExtPict", &[GB11]),
    gb("12.0", "GB12", "sot (RI RI)* RI × RI", &[]),
    gb("13.0", "GB13", "[^RI] (RI RI)* RI × RI", &[]),
    gb("999.0", "GB999", "Any ÷ Any", &[GB9C, GB11]),
];

/// The rules the named levels do not include, per file, each with why no case
/// reaches it. Empty, because neither file's header puts a rule out of scope.
///
/// *What would reverse the emptiness:* a Unicode version whose file says in its
/// header that some rule is untested or tailored. Then that rule, and only it,
/// is a row here.
const OUTSIDE_THE_LEVEL: &[(Corpus, &str, &str)] = &[];

/// One case this run does not hold to the named level.
#[derive(Clone, Copy)]
struct Exclusion {
    corpus: Corpus,
    /// The case's line in the file. Unit: lines, counted from one.
    line: usize,
    /// A rule from [`OUTSIDE_THE_LEVEL`] for the same file.
    rule: &'static str,
    /// Why this case needs that rule, in a sentence.
    why: &'static str,
}

/// Every case excluded from the named levels. Empty: every case in both files
/// passes, and with nothing outside either level, an entry here would have no
/// rule to cite. Adding one is renaming a level, which is a diff to this file's
/// first section, to `TODO.md` and to the module that fails.
const EXCLUDED: &[Exclusion] = &[];

/// One line of either file: the scalars, at each of the `n + 1` positions
/// whether the file puts a boundary there (`÷`) or not (`×`), and the label of
/// the rule its comment says decides the position.
struct Case {
    line: usize,
    text: Vec<char>,
    breaks: Vec<bool>,
    labels: Vec<String>,
}

/// What a file came to.
#[derive(Default)]
struct Tally {
    lines: usize,
    ran: usize,
    passed: usize,
    excluded: usize,
    failed: usize,
    /// Texts whose text path the within-cluster assertion consumed to the end,
    /// counted inside [`Run::hold`]. Unit: texts.
    held: usize,
}

/// Whose count a text held by [`Run::hold`] adds to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Holder {
    /// A case of one file: the text path.
    File(Corpus),
    /// One of the argued eight: the text path.
    Entry,
    /// A control — the untailored breaks of a case of `LineBreakTest.txt`, or
    /// a made-up wrong answer — whose findings the caller drains and counts.
    /// It adds to no count of texts held.
    Control,
}

/// One thing the run found wrong, as printed.
enum Finding {
    /// Said in words where it was found.
    Said(String),
    /// The within-cluster assertion's, from [`Run::hold`]'s one reporting
    /// branch, kept whole rather than as a sentence so that a control which
    /// drains it counts what the assertion found: the text's name, the
    /// verdict, and the path judged, spelled.
    Within { name: String, verdict: Verdict, spelled: String },
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Said(text) => f.write_str(text),
            Self::Within { name, verdict, spelled } => {
                write!(f, "{name}: {}\n    {spelled}", verdict.describe())
            }
        }
    }
}

/// A text path as the within-cluster assertion judges it, one entry per
/// position.
struct Path {
    /// It offers a break here.
    breaks: Vec<bool>,
    /// It calls the break here mandatory.
    hard: Vec<bool>,
    /// It offered the end of the text, which LB3 makes every text path's
    /// last position: the iterator was consumed to the end.
    ended: bool,
}

impl Path {
    /// `f_text::line::opportunities`, consumed.
    fn offered(text: &[char]) -> Self {
        let n = text.len();
        let mut path = Self { breaks: vec![false; n + 1], hard: vec![false; n + 1], ended: false };
        for o in opportunities(text) {
            path.breaks[o.at] = true;
            path.hard[o.at] = o.mandatory;
            path.ended |= o.at == text.len();
        }
        path
    }

    /// The untailored breaks, sent where a text path goes: the corpus-wide
    /// control's answer.
    fn untailored(line: &Untailored) -> Self {
        Self { breaks: line.breaks.clone(), hard: line.hard.clone(), ended: true }
    }

    /// A made-up answer over `n` scalars: breaks at `breaks`, mandatory at
    /// `hard`.
    fn made_up(n: usize, breaks: &[usize], hard: &[usize]) -> Self {
        let mut path = Self { breaks: vec![false; n + 1], hard: vec![false; n + 1], ended: true };
        for &k in breaks {
            path.breaks[k] = true;
        }
        for &k in hard {
            path.hard[k] = true;
        }
        path
    }
}

/// What a labelled position's label says of the rule this tree applied there.
enum Attribution<'a> {
    /// The label names that rule: the position witnesses its arm.
    Held,
    /// The label names another rule, which is this arm's.
    Misattributed(&'a Arm),
    /// No arm has the label.
    Unknown,
}

/// Whether `here`, the rule this tree's module applied at a position, is the
/// rule `arms` maps the file's `label` for it to.
fn attribute<'a>(arms: &'a [Arm], corpus: Corpus, label: &str, here: &str) -> Attribution<'a> {
    match arms.iter().find(|a| a.corpus == corpus && a.label == label) {
        None => Attribution::Unknown,
        Some(arm) if arm.rule == here => Attribution::Held,
        Some(arm) => Attribution::Misattributed(arm),
    }
}

/// The control for [`attribute`]: a label with the rule it names, the same
/// label with another rule, and a label no arm has. Returns whether each is
/// answered as it must be.
fn attribution_is_armed() -> bool {
    let lb = Corpus::LineBreakTest;
    matches!(attribute(ARMS, lb, "25.14", "LB25"), Attribution::Held)
        && matches!(attribute(ARMS, lb, "25.14", "LB31"), Attribution::Misattributed(arm) if arm.rule == "LB25")
        && matches!(attribute(ARMS, lb, "25.99", "LB25"), Attribution::Unknown)
}

/// One opportunity inside a cluster, as printed: its line, its position,
/// and the scalars either side of it.
type Instance = (usize, usize, char, char);

/// Everything the run found.
struct Run {
    tallies: BTreeMap<Corpus, Tally>,
    findings: Vec<Finding>,
    failures_shown: usize,
    matched: Vec<bool>,
    /// `LineBreakTest.txt`'s opportunities inside a cluster, as the file marks
    /// them: (line, position).
    inside_by_file: Vec<(usize, usize)>,
    /// The same, as `f_text::line::decisions` makes them. Unit: positions.
    inside_by_decisions: usize,
    /// The same, as [`Run::hold`] reports them when sent the untailored
    /// breaks, drained from its findings and counted. Unit: positions.
    inside_by_hold: usize,
    /// The same, by the rule that allowed each: how many, and the first few
    /// as [`Instance`]s.
    inside_by_rule: BTreeMap<String, (usize, Vec<Instance>)>,
    /// The corpus entries [`Run::hold`] consumed to the end. Unit: texts.
    entries_held: usize,
    /// Positions per file and label. Unit: positions.
    witnesses: BTreeMap<(Corpus, String), usize>,
    /// Labels a file uses that [`ARMS`] does not name, with how often.
    unknown: BTreeMap<(Corpus, String), usize>,
    /// Positions decided by a rule other than the one [`ARMS`] maps the
    /// file's label to. Unit: positions.
    misattributed: usize,
    /// Mandatory breaks in `LineBreakTest.txt` by LB3 to LB5 from the
    /// property, by rule. Unit: positions.
    mandatory_by_rule: BTreeMap<&'static str, usize>,
    /// Case texts set as a paragraph whose line ends held, and texts refused
    /// as `Unset::SeparatorMidLine`, per file. Unit: texts.
    set: BTreeMap<Corpus, (usize, usize)>,
    /// Case texts whose setting was wrong, all of them counted. Unit: texts.
    set_wrong: usize,
}

/// The seat [`Run::set_in_lines`] fills: half an em a scalar, whatever it is.
/// Not a shaper; a width a reader can count, so that what the check holds is
/// where lines end, which is the paragraph's, and not a glyph's advance.
fn seat(run: &[char]) -> Result<Advance, NotAMetric> {
    Advance::in_design_units(500 * i32::try_from(run.len()).unwrap_or(i32::MAX))
}

/// A 1000-unit grid at a 32-pixel em: sixteen pixels a scalar.
fn seat_scale() -> Scale {
    Scale::new(1000, 32 * 64).unwrap_or_else(|why| panic!("the seat's scale: {why:?}"))
}

/// Parse `÷ 0020 × 0308 ÷ # ÷ [0.2] SPACE (Other) × [9.0] …` into scalars,
/// marks and labels.
fn parse(line: usize, raw: &str) -> Result<Case, String> {
    let (data, comment) = raw.split_once('#').unwrap_or((raw, ""));
    let mut text = Vec::new();
    let mut breaks = Vec::new();
    for (i, token) in data.split_whitespace().enumerate() {
        if i % 2 == 0 {
            breaks.push(match token {
                "÷" => true,
                "×" => false,
                other => return Err(format!("`{other}` where a mark belongs")),
            });
        } else {
            let c = u32::from_str_radix(token, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| format!("`{token}` is not a scalar"))?;
            text.push(c);
        }
    }
    if text.is_empty() || breaks.len() != text.len() + 1 {
        return Err(format!("{} scalar(s) and {} mark(s)", text.len(), breaks.len()));
    }
    // The comment repeats each mark with the rule's label after it.
    let mut labels = Vec::new();
    let mut words = comment.split_whitespace().peekable();
    while let Some(word) = words.next() {
        let mark = match word {
            "÷" => true,
            "×" => false,
            _ => continue,
        };
        let label = words
            .next_if(|w| w.starts_with('[') && w.ends_with(']'))
            .ok_or_else(|| format!("the comment's mark {} has no [label]", labels.len()))?;
        if breaks.get(labels.len()) != Some(&mark) {
            return Err(format!("the comment's mark {} is not the data's", labels.len()));
        }
        labels.push(label.trim_start_matches('[').trim_end_matches(']').to_string());
    }
    if labels.len() != breaks.len() {
        return Err(format!("{} mark(s) and {} label(s)", breaks.len(), labels.len()));
    }
    Ok(Case { line, text, breaks, labels })
}

/// Grapheme cluster boundaries, one per position: `f_text::grapheme`'s answer.
fn clusters(text: &[char]) -> Vec<bool> {
    let mut at = vec![false; text.len() + 1];
    for k in cluster_boundaries(text) {
        at[k] = true;
    }
    at
}

/// The rule `f_text::grapheme` applies at each position, by name; GB2 at the
/// end, which is the caller's in that module.
fn cluster_rules(text: &[char]) -> Vec<String> {
    let mut state = Clusters::new();
    let mut rules: Vec<String> = text.iter().map(|&c| format!("{:?}", state.push(c))).collect();
    rules.push("GB2".to_string());
    rules
}

/// The untailored algorithm's answer for one text, one entry per position.
/// Position 0 is LB2's and never a break.
struct Untailored {
    /// `÷` or `!`.
    breaks: Vec<bool>,
    /// `!`.
    hard: Vec<bool>,
    decided: Vec<Decision>,
}

fn untailored(text: &[char]) -> Untailored {
    let mut breaks = vec![false; text.len() + 1];
    let mut hard = vec![false; text.len() + 1];
    let decided: Vec<Decision> = decisions(text).collect();
    for d in &decided {
        breaks[d.at] = d.outcome != Outcome::Prohibited;
        hard[d.at] = d.outcome == Outcome::Mandatory;
    }
    Untailored { breaks, hard, decided }
}

/// The rule `f_text::line` applies at each position, by name; LB2 at the start.
fn line_rules(line: &Untailored) -> Vec<String> {
    let mut rules = vec!["LB2".to_string(); line.breaks.len()];
    for d in &line.decided {
        rules[d.at] = format!("{:?}", d.rule);
    }
    rules
}

/// Where LB3 to LB5 put a mandatory break, read from the `Line_Break`
/// property alone and not from `f_text::line`: at the end of the text (LB3),
/// and after every BK (LB4) and every CR, LF and NL (LB5) except a CR an LF
/// follows (LB5's `CR × LF`). Only LB2, at position 0, comes before them, and
/// LB9 absorbs nothing into those four classes, so this is the whole rule. The
/// file writes these breaks as `÷`, which is why they are derived rather than
/// read.
fn mandatory(text: &[char]) -> Vec<bool> {
    let mut at = vec![false; text.len() + 1];
    for k in 1..=text.len() {
        let before = line_break(text[k - 1]);
        let lf_follows = text.get(k).is_some_and(|&c| line_break(c) == LineBreak::LF);
        at[k] = k == text.len()
            || matches!(before, LineBreak::BK | LineBreak::LF | LineBreak::NL)
            || (before == LineBreak::CR && !lf_follows);
    }
    at
}

/// The rule [`mandatory`] reads at `k`, for the printed count.
fn mandatory_rule(text: &[char], k: usize) -> &'static str {
    if k == text.len() {
        "LB3"
    } else if line_break(text[k - 1]) == LineBreak::BK {
        "LB4"
    } else {
        "LB5"
    }
}

/// The positions in `breaks` that are not cluster boundaries.
fn inside_clusters(breaks: &[bool], boundaries: &[bool]) -> Vec<usize> {
    breaks
        .iter()
        .zip(boundaries)
        .enumerate()
        .filter(|(_, (brk, boundary))| **brk && !**boundary)
        .map(|(k, _)| k)
        .collect()
}

/// What the within-cluster assertion found wrong with one text path, by
/// position.
#[derive(Default, PartialEq, Eq)]
struct Verdict {
    inside: Vec<usize>,
    invented: Vec<usize>,
    lost: Vec<usize>,
    /// Called mandatory where LB3 to LB5 do not make it so, or not called so
    /// where they do, or a mandatory break missing.
    hard: Vec<usize>,
}

impl Verdict {
    fn is_empty(&self) -> bool {
        self.inside.is_empty()
            && self.invented.is_empty()
            && self.lost.is_empty()
            && self.hard.is_empty()
    }

    fn describe(&self) -> String {
        let mut wrong = Vec::new();
        if !self.inside.is_empty() {
            wrong.push(format!("the text path breaks inside a cluster at {:?}", self.inside));
        }
        if !self.invented.is_empty() {
            wrong.push(format!("it breaks where UAX #14 does not at {:?}", self.invented));
        }
        if !self.lost.is_empty() {
            wrong.push(format!(
                "it loses a UAX #14 opportunity at a cluster boundary at {:?}",
                self.lost
            ));
        }
        if !self.hard.is_empty() {
            wrong.push(format!("its mandatory breaks are not LB3-LB5's at {:?}", self.hard));
        }
        wrong.join("; ")
    }
}

/// The within-cluster assertion itself: what is wrong with `path`, and with
/// `hard` as the mandatory ones among it, as the text path's breaks — given
/// the untailored algorithm's breaks, the cluster boundaries and LB3 to LB5's
/// mandatory breaks, one per position.
fn within_clusters(
    path: &[bool],
    hard: &[bool],
    untailored: &[bool],
    boundaries: &[bool],
    must: &[bool],
) -> Verdict {
    let every = 0..path.len();
    Verdict {
        inside: inside_clusters(path, boundaries),
        invented: every.clone().filter(|&k| path[k] && !untailored[k]).collect(),
        lost: every.clone().filter(|&k| untailored[k] && boundaries[k] && !path[k]).collect(),
        hard: every.filter(|&k| (path[k] && hard[k] != must[k]) || (must[k] && !path[k])).collect(),
    }
}

fn spell(text: &[char], marks: &[bool]) -> String {
    let mut out = String::new();
    for (k, &m) in marks.iter().enumerate() {
        out.push_str(if m { "÷" } else { "×" });
        if let Some(c) = text.get(k) {
            out.push_str(&format!(" {:04X} ", u32::from(*c)));
        }
    }
    out
}

/// Every check on [`ARMS`] that does not need a case to run: an arm no case
/// reaches and no test argues, a cited test that does not exist, a label
/// listed twice.
fn arm_findings(arms: &[Arm], witnesses: &BTreeMap<(Corpus, String), usize>) -> Vec<String> {
    let mut findings = Vec::new();
    for (i, arm) in arms.iter().enumerate() {
        let seen = witnesses.get(&(arm.corpus, arm.label.to_string())).copied().unwrap_or(0);
        if seen == 0 && arm.argued.is_empty() {
            findings.push(format!(
                "{} [{}] {} `{}`: no case of the file reaches the arm and no unit test argues it, \
                 so nothing holds it",
                arm.corpus.file(),
                arm.label,
                arm.rule,
                arm.part
            ));
        }
        for name in arm.argued {
            let found = name.split_once("::").is_some_and(|(module, test)| {
                SOURCES.iter().any(|(m, source)| {
                    *m == module && source.contains(&format!("#[test]\n    fn {test}("))
                })
            });
            if !found {
                findings.push(format!(
                    "{} [{}] cites `{name}`, which is not a test in text/src",
                    arm.corpus.file(),
                    arm.label
                ));
            }
        }
        if arms[..i].iter().any(|a| a.corpus == arm.corpus && a.label == arm.label) {
            findings.push(format!("{} [{}] is listed twice", arm.corpus.file(), arm.label));
        }
    }
    findings
}

impl Run {
    fn finding(&mut self, text: String) {
        self.findings.push(Finding::Said(text));
    }

    /// The case's text set as a paragraph, in a measure nothing fits and in
    /// one everything fits; what the module header's last section holds.
    fn set_in_lines(&mut self, corpus: Corpus, case: &Case) {
        let n = case.text.len();
        if n == 0 {
            return;
        }
        let text: String = case.text.iter().collect();
        let offered: Vec<(usize, bool)> =
            opportunities(&case.text).map(|o| (o.at, o.mandatory)).collect();
        let every: Vec<usize> = offered.iter().map(|&(at, _)| at).collect();
        let hard: Vec<usize> = offered.iter().filter(|&&(_, m)| m).map(|&(at, _)| at).collect();
        let mut setting = Box::new(Setting::new());
        let mut wrong = None;
        let mut refused = false;
        for (px, nothing_fits) in [(1, true), (1 << 20, false)] {
            let Ok(measure) = Px::new(px) else {
                wrong = Some(format!("{px} px is not a measure"));
                break;
            };
            match setting.set(&text, BaseDirection::FirstStrong, seat_scale(), measure, seat) {
                Ok(lines) => {
                    let ends: Vec<usize> = lines.iter().map(|l| l.end).collect();
                    let starts: Vec<usize> = lines.iter().map(|l| l.start).collect();
                    let mut from = vec![0];
                    from.extend(ends.iter().copied().take(ends.len().saturating_sub(1)));
                    let tiled = starts == from && ends.last() == Some(&n);
                    let right = if nothing_fits {
                        ends == every
                    } else {
                        ends.iter().all(|e| every.contains(e))
                            && hard.iter().all(|h| ends.contains(h))
                    };
                    if !(tiled && right) {
                        wrong = Some(format!(
                            "{px} px: lines end at {ends:?} starting at {starts:?}; the whole text \
                             offers {every:?}, mandatory {hard:?}"
                        ));
                    }
                }
                Err(Unset::SeparatorMidLine { at })
                    if !every.contains(&at)
                        && matches!(
                            case.text.get(at.wrapping_sub(1)),
                            Some('\u{1C}'..='\u{1E}')
                        ) =>
                {
                    refused = true;
                }
                Err(other) => wrong = Some(format!("{px} px: refused, {other:?}")),
            }
        }
        match wrong {
            None => {
                let tally = self.set.entry(corpus).or_default();
                if refused {
                    tally.1 += 1;
                } else {
                    tally.0 += 1;
                }
            }
            Some(why) => {
                self.set_wrong += 1;
                if self.set_wrong <= SHOWN {
                    self.finding(format!(
                        "{}:{} set as a paragraph: {why}",
                        corpus.file(),
                        case.line
                    ));
                }
            }
        }
    }

    /// The within-cluster assertion over one text: `path` judged against the
    /// untailored breaks in `line`, the cluster boundaries and LB3 to LB5's
    /// mandatory breaks, and whatever is wrong reported through the one branch
    /// every caller reports through — the text path, the corpus-wide control
    /// and the made-up wrong answers alike. The controls drain what it reports
    /// and count it ([`Run::control`], [`Run::within_clusters_is_armed`]), so a
    /// report silenced or narrowed here is red there rather than a silence over
    /// the corpus.
    fn hold(&mut self, holder: Holder, name: &str, text: &[char], line: &Untailored, path: &Path) {
        let boundaries = clusters(text);
        let must = mandatory(text);
        let verdict = within_clusters(&path.breaks, &path.hard, &line.breaks, &boundaries, &must);
        if !verdict.is_empty() {
            let spelled = spell(text, &path.breaks);
            self.findings.push(Finding::Within { name: name.to_string(), verdict, spelled });
        }
        // Counted here, after the judgement, and only for a path consumed to
        // the end of the text: a run that never judged the text path, or
        // judged part of it, counts nothing.
        if path.ended {
            match holder {
                Holder::File(corpus) => self.tallies.entry(corpus).or_default().held += 1,
                Holder::Entry => self.entries_held += 1,
                Holder::Control => {}
            }
        }
    }

    /// The findings pushed since `before`, taken back out: what a control
    /// drew.
    fn drain(&mut self, before: usize) -> Vec<Finding> {
        self.findings.drain(before..).collect()
    }

    /// The corpus-wide control, over one case of `LineBreakTest.txt`: its
    /// untailored breaks sent through [`Run::hold`] in place of the text path.
    /// What `hold` reports is drained and counted. Breaks inside a cluster add
    /// to `inside_by_hold`, which must come to [`INSIDE_A_CLUSTER`]; anything
    /// else is a finding of its own, since the untailored breaks are UAX #14's
    /// and LB3 to LB5's by construction and can be wrong only by breaking
    /// inside a cluster.
    fn control(&mut self, name: &str, text: &[char], line: &Untailored) {
        let before = self.findings.len();
        self.hold(Holder::Control, name, text, line, &Path::untailored(line));
        for drawn in self.drain(before) {
            match drawn {
                Finding::Within { verdict, .. }
                    if verdict.invented.is_empty()
                        && verdict.lost.is_empty()
                        && verdict.hard.is_empty() =>
                {
                    self.inside_by_hold += verdict.inside.len();
                }
                other => self.finding(format!(
                    "the control, the untailored breaks judged as a text path, draws more than \
                     breaks inside a cluster: {other}"
                )),
            }
        }
    }

    /// The control for [`Run::hold`] on made-up answers, sent through it
    /// exactly as the text path is. The text is `a`, a space, U+0308, a space
    /// and `b`: UAX #14 breaks at 2 (LB18 after the first space, inside the
    /// cluster GB9 makes of that space and the mark), at 4 (LB18) and at 5
    /// (LB3, mandatory), and the cluster boundaries are 0, 1, 3, 4 and 5. So
    /// the right text path is 4 and 5, with 5 mandatory, and each of five
    /// wrong ones breaks exactly one of the four checks. Each must draw
    /// exactly one finding with exactly its fault, and the right answer none.
    /// Returns what the assertion got wrong, one line per answer.
    fn within_clusters_is_armed(&mut self) -> Vec<String> {
        let text = ['a', ' ', '\u{0308}', ' ', 'b'];
        let n = text.len();
        let line = untailored(&text);
        let inside = |k| Some(Verdict { inside: vec![k], ..Verdict::default() });
        let invented = |k| Some(Verdict { invented: vec![k], ..Verdict::default() });
        let lost = |k| Some(Verdict { lost: vec![k], ..Verdict::default() });
        let hard = |k| Some(Verdict { hard: vec![k], ..Verdict::default() });
        let answers = [
            ("the untailored breaks", &[2, 4, 5][..], &[5][..], inside(2)),
            ("a break UAX #14 does not make", &[3, 4, 5], &[5], invented(3)),
            ("an opportunity lost", &[5], &[5], lost(4)),
            ("the end not called mandatory", &[4, 5], &[], hard(5)),
            ("a break called mandatory that is not", &[4, 5], &[4, 5], hard(4)),
            ("the right answer", &[4, 5], &[5], None),
        ];
        let mut wrong = Vec::new();
        for (what, breaks, hard, want) in answers {
            let before = self.findings.len();
            let path = Path::made_up(n, breaks, hard);
            self.hold(Holder::Control, "the made-up text", &text, &line, &path);
            let drawn = self.drain(before);
            let right = match (&want, drawn.as_slice()) {
                (None, []) => true,
                (Some(want), [Finding::Within { verdict, .. }]) => verdict == want,
                _ => false,
            };
            if !right {
                let shown: Vec<String> = drawn.iter().map(|f| format!(": {f}")).collect();
                wrong.push(format!(
                    "fed {what}, drew {} finding(s) where it must draw {}{}",
                    drawn.len(),
                    usize::from(want.is_some()),
                    shown.concat()
                ));
            }
        }
        wrong
    }

    /// One case against the implementation, filed.
    fn case(&mut self, corpus: Corpus, case: &Case) {
        let n = case.text.len();
        let line = untailored(&case.text);
        let must = mandatory(&case.text);
        let (got, rules) = match corpus {
            Corpus::GraphemeBreakTest => (clusters(&case.text), cluster_rules(&case.text)),
            Corpus::LineBreakTest => (line.breaks.clone(), line_rules(&line)),
        };
        let mut wrong = Vec::new();
        if got != case.breaks {
            let differ: Vec<String> = if corpus == Corpus::LineBreakTest {
                line.decided
                    .iter()
                    .filter(|d| (d.outcome != Outcome::Prohibited) != case.breaks[d.at])
                    .map(|d| format!("{} by {:?}", d.at, d.rule))
                    .collect()
            } else {
                Vec::new()
            };
            wrong.push(format!(
                "the file says {}\n    this says     {}{}",
                spell(&case.text, &case.breaks),
                spell(&case.text, &got),
                if differ.is_empty() { String::new() } else { format!("\n    at {differ:?}") }
            ));
        }
        if corpus == Corpus::LineBreakTest {
            if line.hard != must {
                let at = |v: &[bool]| (0..=n).filter(|&k| v[k]).collect::<Vec<usize>>();
                wrong.push(format!(
                    "mandatory at {:?}, where LB3-LB5 read from the property put it at {:?}",
                    at(&line.hard),
                    at(&must)
                ));
            }
            for k in (1..=n).filter(|&k| must[k]) {
                *self.mandatory_by_rule.entry(mandatory_rule(&case.text, k)).or_default() += 1;
                if !case.breaks[k] {
                    self.finding(format!(
                        "{}:{}: LB3-LB5 read from the property put a mandatory break at {k}, \
                         where the file marks ×: the derivation is wrong, not the case",
                        corpus.file(),
                        case.line
                    ));
                }
            }
        }
        let outcome = if wrong.is_empty() { Ok(()) } else { Err(wrong.join("\n    ")) };
        let excluded = EXCLUDED.iter().position(|e| e.corpus == corpus && e.line == case.line);

        // Which rule decided each position, against the file's label for it.
        for (k, label) in case.labels.iter().enumerate() {
            if corpus == Corpus::LineBreakTest && k == 0 {
                // LB2's, which `decisions` does not yield and the file labels
                // with LB3's number.
                continue;
            }
            // A position witnesses its arm only once the rule the label names
            // is the rule applied: the coverage table counts what the
            // comparison held, not what the file labels.
            match attribute(ARMS, corpus, label, &rules[k]) {
                Attribution::Held => {
                    *self.witnesses.entry((corpus, label.clone())).or_default() += 1;
                }
                Attribution::Unknown => {
                    *self.unknown.entry((corpus, label.clone())).or_default() += 1;
                }
                Attribution::Misattributed(_) if excluded.is_some() => {}
                Attribution::Misattributed(arm) => {
                    self.misattributed += 1;
                    if self.misattributed <= SHOWN {
                        self.finding(format!(
                            "{}:{} position {k}: the file decides it by [{label}], which is {} \
                             `{}`, and this decides it by {}",
                            corpus.file(),
                            case.line,
                            arm.rule,
                            arm.part,
                            rules[k]
                        ));
                    }
                }
            }
        }

        let tally = self.tallies.entry(corpus).or_default();
        tally.ran += 1;
        match (outcome, excluded) {
            (Ok(()), None) => tally.passed += 1,
            (Err(_), Some(at)) => {
                tally.excluded += 1;
                self.matched[at] = true;
            }
            (Ok(()), Some(at)) => {
                tally.failed += 1;
                self.matched[at] = true;
                self.finding(format!(
                    "{}:{} passes and is in EXCLUDED, citing {}. An exclusion that is not needed \
                     widens the list without anything failing; remove it.",
                    corpus.file(),
                    case.line,
                    EXCLUDED[at].rule
                ));
            }
            (Err(why), None) => {
                tally.failed += 1;
                if self.failures_shown < SHOWN {
                    self.failures_shown += 1;
                    self.finding(format!("{}:{}\n    {why}", corpus.file(), case.line));
                }
            }
        }

        // The within-cluster assertion, over this case whichever file it is
        // from; and for the line file, its control and the count by rule.
        let name = format!("{}:{}", corpus.file(), case.line);
        self.hold(Holder::File(corpus), &name, &case.text, &line, &Path::offered(&case.text));
        if corpus == Corpus::LineBreakTest {
            self.control(&name, &case.text, &line);
            let boundaries = clusters(&case.text);
            for k in inside_clusters(&case.breaks, &boundaries) {
                self.inside_by_file.push((case.line, k));
            }
            for k in inside_clusters(&line.breaks, &boundaries) {
                self.inside_by_decisions += 1;
                let rule = line.decided.iter().find(|d| d.at == k).map(|d| format!("{:?}", d.rule));
                let (count, instances) =
                    self.inside_by_rule.entry(rule.unwrap_or_default()).or_default();
                *count += 1;
                if instances.len() < INSTANCES {
                    instances.push((case.line, k, case.text[k - 1], case.text[k]));
                }
            }
        }
    }
}

/// The file's text, or a failure naming it and the command that restores it.
fn read(corpus: Corpus) -> Result<String, String> {
    let path: PathBuf =
        [env!("CARGO_MANIFEST_DIR"), "..", CORPUS_DIR, corpus.file()].iter().collect();
    std::fs::read_to_string(&path).map_err(|e| {
        format!(
            "{}: {e}\n  The conformance corpus is committed rather than fetched, and a run \
             without it is a failure rather than a skip (RFC 0114). Restore it with\n    \
             git checkout -- {CORPUS_DIR}/{}",
            path.display(),
            corpus.file()
        )
    })
}

/// Every data line of one file, run; and the file's own `# Lines:` compared.
fn run_file(run: &mut Run, corpus: Corpus, text: &str) {
    let mut data = 0usize;
    let mut stated: Option<usize> = None;
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Some(count) = line.strip_prefix("# Lines:") {
            stated = count.trim().parse().ok();
            if stated.is_none() {
                run.finding(format!("{}:{} states an unreadable count", corpus.file(), index + 1));
            }
            continue;
        }
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        data += 1;
        match parse(index + 1, line) {
            Ok(case) => {
                run.case(corpus, &case);
                run.set_in_lines(corpus, &case);
            }
            Err(why) => {
                run.finding(format!("{}:{} does not parse: {why}", corpus.file(), index + 1));
            }
        }
    }
    run.tallies.entry(corpus).or_default().lines = data;
    if stated != Some(data) {
        run.finding(format!(
            "{} has {data} data lines and says `# Lines: {}`: the parser read a different file \
             from the one the file describes",
            corpus.file(),
            stated.map_or_else(|| "nothing".to_string(), |n| n.to_string())
        ));
    }
}

/// Every check on an exclusion list that does not need a case to run.
fn exclusion_findings(list: &[Exclusion]) -> Vec<String> {
    let mut findings = Vec::new();
    for (i, e) in list.iter().enumerate() {
        if !OUTSIDE_THE_LEVEL.iter().any(|(corpus, rule, _)| *corpus == e.corpus && *rule == e.rule)
        {
            findings.push(format!(
                "EXCLUDED[{i}] ({}:{}) cites `{}`, which the named level includes. Neither file \
                 puts a rule out of scope, so nothing may excuse a case; a failure is a defect \
                 or a renamed level",
                e.corpus.file(),
                e.line,
                e.rule
            ));
        }
        if !e.why.ends_with('.') || e.why.len() < 24 {
            findings.push(format!("EXCLUDED[{i}] does not say why in a sentence"));
        }
        if list[..i].iter().any(|d| d.corpus == e.corpus && d.line == e.line) {
            findings.push(format!("EXCLUDED[{i}] lists a case twice"));
        }
    }
    findings
}

/// Parse cargo's harness arguments: answer `--list` and a filter, accept
/// libtest's own flags, refuse anything else.
fn selected(args: &[String]) -> Result<Option<bool>, String> {
    let mut list = false;
    let mut filter: Option<&str> = None;
    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        match arg.as_str() {
            "--list" => list = true,
            "--nocapture" | "--quiet" | "-q" | "--exact" | "--show-output" | "--ignored"
            | "--include-ignored" | "--test" | "--bench" => {}
            "--test-threads" | "--color" | "--format" | "--logfile" | "--skip" | "-Z" => {
                walk.next().ok_or_else(|| format!("{arg} needs a value"))?;
            }
            other if other.starts_with('-') => return Err(format!("unknown argument: {other}")),
            other => filter = Some(other),
        }
    }
    if filter.is_some_and(|name| !TEST_NAME.contains(name)) {
        return Ok(None);
    }
    Ok(Some(list))
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match selected(&args) {
        Err(why) => {
            eprintln!("{TEST_NAME}: {why}");
            exit(2);
        }
        Ok(None) => {
            println!("{TEST_NAME}: 0 cases — the filter selects no test in this target");
            return;
        }
        Ok(Some(true)) => {
            println!("{TEST_NAME}: test\n\n1 test, 0 benchmarks");
            return;
        }
        Ok(Some(false)) => {}
    }
    exit(conformance());
}

/// The coverage table: per file and rule, each label with its positions, `*`
/// where a unit test argues the arm, and the tests cited.
fn print_arms(witnesses: &BTreeMap<(Corpus, String), usize>) {
    println!(
        "  arms, by each file's own [rule] labels: positions per label decided by the rule it \
         names; * a unit test argues the arm or alternatives inside it"
    );
    let mut at = 0;
    while at < ARMS.len() {
        let first = &ARMS[at];
        let group: Vec<&Arm> = ARMS[at..]
            .iter()
            .take_while(|a| a.corpus == first.corpus && a.rule == first.rule)
            .collect();
        at += group.len();
        let mut row = format!(
            "    {:<18} {:<6}",
            first.corpus.file().trim_start_matches("auxiliary/"),
            first.rule
        );
        let mut cited = BTreeSet::new();
        for arm in &group {
            let seen = witnesses.get(&(arm.corpus, arm.label.to_string())).copied().unwrap_or(0);
            let star = if arm.argued.is_empty() { "" } else { "*" };
            row.push_str(&format!(" [{}] {seen}{star}", arm.label));
            cited.extend(arm.argued.iter().copied());
        }
        println!("{row}");
        if !cited.is_empty() {
            let cited: Vec<&str> = cited.into_iter().collect();
            println!("      argued in {}", cited.join(", "));
        }
    }
}

#[allow(clippy::too_many_lines, reason = "the run's report, in the order it prints")]
fn conformance() -> i32 {
    println!("{TEST_NAME}: Unicode {UNICODE_VERSION}");
    for corpus in Corpus::ALL {
        println!("  {:<32} {}", corpus.file(), corpus.level());
    }
    let mut run = Run {
        tallies: BTreeMap::new(),
        findings: exclusion_findings(EXCLUDED).into_iter().map(Finding::Said).collect(),
        failures_shown: 0,
        matched: vec![false; EXCLUDED.len()],
        inside_by_file: Vec::new(),
        inside_by_decisions: 0,
        inside_by_hold: 0,
        inside_by_rule: BTreeMap::new(),
        entries_held: 0,
        witnesses: BTreeMap::new(),
        unknown: BTreeMap::new(),
        misattributed: 0,
        mandatory_by_rule: BTreeMap::new(),
        set: BTreeMap::new(),
        set_wrong: 0,
    };
    // The control. With `EXCLUDED` empty the checks above run over nothing and
    // could not fail, so they are fed entries each must refuse: a rule the
    // level includes, a reason that is a label, and one case twice.
    let decoy = Exclusion { corpus: Corpus::LineBreakTest, line: 1, rule: "LB25", why: "flaky" };
    let twice = [decoy, Exclusion { rule: "GB9c", ..decoy }];
    if exclusion_findings(&twice).len() != 5 {
        run.finding(
            "the exclusion checks accept an entry citing LB25 with no reason, listed twice, so \
             EXCLUDED is unchecked"
                .to_string(),
        );
    }
    // The second control: the within-cluster assertion, fed five wrong text
    // paths through the `hold` the text path goes through, must report each
    // as exactly its fault, and the right one not at all.
    for wrong in run.within_clusters_is_armed() {
        run.finding(format!(
            "the within-cluster check, {wrong}; so its silence over both files proves nothing"
        ));
    }
    // The third: the arm checks, fed an arm nothing reaches and nothing
    // argues, one citing a test that does not exist, a label with a rule it
    // does not name, and a label no arm has.
    let decoys = [
        lb("25.99", "LB25", "a decoy nothing reaches", &[]),
        lb("25.98", "LB25", "a decoy citing nothing", &["line::no_such_test"]),
    ];
    if arm_findings(&decoys, &BTreeMap::new()).len() != 2 || !attribution_is_armed() {
        run.finding(
            "the arm checks accept an arm no case reaches and no test argues, a citation of a \
             test that does not exist, a label decided by a rule it does not name or a label no \
             arm has, so the coverage table holds nothing"
                .to_string(),
        );
    }
    for corpus in Corpus::ALL {
        match read(corpus) {
            Ok(text) => run_file(&mut run, corpus, &text),
            Err(why) => run.finding(why),
        }
    }
    for (i, e) in EXCLUDED.iter().enumerate() {
        if !run.matched[i] {
            let finding = format!(
                "EXCLUDED[{i}] names {}:{}, which is not a case in the file",
                e.corpus.file(),
                e.line
            );
            run.finding(finding);
        }
    }
    // The argued eight: the within-cluster assertion over the corpus the rest
    // of the crate is argued against, whose expectations are in the modules.
    for script in Script::ALL {
        let text: Vec<char> = script.sample().chars().collect();
        let line = untailored(&text);
        let name = format!("corpus entry `{}`", script.name());
        run.hold(Holder::Entry, &name, &text, &line, &Path::offered(&text));
    }

    for corpus in Corpus::ALL {
        let tally = run.tallies.remove(&corpus).unwrap_or_default();
        println!(
            "  {:<32} {:>6} lines, {:>6} cases: {:>6} passed, {} excluded, {} failed; no break \
             inside a cluster held over {}",
            corpus.file(),
            tally.lines,
            tally.ran,
            tally.passed,
            tally.excluded,
            tally.failed,
            tally.held
        );
        if tally.ran == 0 {
            run.finding(format!(
                "{} ran no cases, and a run of nothing is not a pass (RFC 0115)",
                corpus.file()
            ));
        } else if tally.ran != corpus.cases() {
            run.finding(format!(
                "{} yielded {} cases where it holds {}: a case was lost or gained between the \
                 file and this run",
                corpus.file(),
                tally.ran,
                corpus.cases()
            ));
        }
        if tally.held != tally.ran {
            run.finding(format!(
                "{}: the within-cluster assertion consumed the text path to the end of {} of {} \
                 cases, so its silence covers only those",
                corpus.file(),
                tally.held,
                tally.ran
            ));
        }
    }
    let mut set_total = 0;
    for corpus in Corpus::ALL {
        let (held, refused) = run.set.get(&corpus).copied().unwrap_or_default();
        set_total += held + refused;
        println!(
            "  {:<32} set as a paragraph: {held} held (every line end offered over the whole \
             text; every offer taken in 1 px, every mandatory one in 1 048 576 px), {refused} \
             refused as U+001C-U+001E, which ends a paragraph where UAX #14 offers no break",
            corpus.file()
        );
        if held == 0 {
            run.finding(format!(
                "{}: no case text was set as a paragraph, and a run of nothing is not a pass",
                corpus.file()
            ));
        }
    }
    let cases: usize = Corpus::ALL.iter().map(|c| c.cases()).sum();
    if set_total + run.set_wrong != cases {
        run.finding(format!(
            "{set_total} case texts set as a paragraph and {} wrong, of {cases} cases: a case \
             went unset",
            run.set_wrong
        ));
    }
    if run.set_wrong > SHOWN {
        let finding =
            format!("{} case texts set wrongly as a paragraph, {SHOWN} shown", run.set_wrong);
        run.finding(finding);
    }
    println!(
        "  corpus entries                   {} texts: no break inside a cluster held over {}",
        Script::ALL.len(),
        run.entries_held
    );
    if run.entries_held != Script::ALL.len() {
        run.finding(format!(
            "the within-cluster assertion consumed the text path of {} of the {} corpus entries",
            run.entries_held,
            Script::ALL.len()
        ));
    }
    println!(
        "  excluded: {} case(s), listed in EXCLUDED in text/tests/break_conformance.rs; neither \
         file puts a rule out of scope, so an exclusion may cite nothing",
        EXCLUDED.len()
    );
    let hard: Vec<String> =
        run.mandatory_by_rule.iter().map(|(rule, count)| format!("{count} by {rule}")).collect();
    println!(
        "  mandatory breaks in LineBreakTest.txt, by LB3-LB5 from the property, each held to be \
         `!` and not `÷`: {}",
        hard.join(", ")
    );
    let found = run.inside_by_file.len();
    println!(
        "  UAX #14 opportunities inside an extended grapheme cluster in LineBreakTest.txt: {found} \
         by the file's marks, {} by f_text::line::decisions, {} reported by the within-cluster \
         assertion sent the untailored breaks; the text path offers none of them",
        run.inside_by_decisions, run.inside_by_hold
    );
    for (rule, (count, instances)) in &run.inside_by_rule {
        let shown: Vec<String> = instances
            .iter()
            .map(|&(line, at, before, after)| {
                format!(
                    "line {line} between U+{:04X} and U+{:04X} (scalar {at})",
                    u32::from(before),
                    u32::from(after)
                )
            })
            .collect();
        println!("    {count:>5} allowed by {rule:<5} e.g. {}", shown.join("; "));
    }
    if found != INSIDE_A_CLUSTER
        || run.inside_by_decisions != INSIDE_A_CLUSTER
        || run.inside_by_hold != INSIDE_A_CLUSTER
    {
        run.finding(format!(
            "INSIDE_A_CLUSTER says {INSIDE_A_CLUSTER}, the file's marks give {found}, the \
             algorithm's {} and the within-cluster assertion sent them {}: either the assertion \
             cannot see what it looks for, or the specification moved and the constant moves \
             with it",
            run.inside_by_decisions, run.inside_by_hold
        ));
    }

    print_arms(&run.witnesses);
    let unwitnessed: Vec<String> = ARMS
        .iter()
        .filter(|a| !run.witnesses.contains_key(&(a.corpus, a.label.to_string())))
        .map(|a| format!("[{}] {} `{}`", a.label, a.rule, a.part))
        .collect();
    println!(
        "  {} arm(s) no case reaches, each argued in a unit test: {}",
        unwitnessed.len(),
        unwitnessed.join("; ")
    );
    println!(
        "  every labelled position decided by the rule its label names: {} position(s) not",
        run.misattributed
    );
    let arms = arm_findings(ARMS, &run.witnesses);
    run.findings.extend(arms.into_iter().map(Finding::Said));
    let unknown: Vec<String> = run
        .unknown
        .iter()
        .map(|((corpus, label), count)| {
            format!(
                "{} labels {count} position(s) [{label}], which ARMS does not name: the file \
                 carries an arm this harness has not read",
                corpus.file()
            )
        })
        .collect();
    run.findings.extend(unknown.into_iter().map(Finding::Said));
    if run.misattributed > SHOWN {
        let finding = format!(
            "{} position(s) decided by a rule other than their label's, {SHOWN} shown",
            run.misattributed
        );
        run.finding(finding);
    }

    if run.findings.is_empty() {
        println!("{TEST_NAME}: ok");
        return 0;
    }
    println!();
    for finding in &run.findings {
        println!("finding  {finding}");
    }
    println!("\n{TEST_NAME}: FAILED — {} finding(s)", run.findings.len());
    1
}
