// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E3-B03e`'s exit: UAX #9's own conformance files, run over `f_text::bidi`.
//!
//! # The level, named here rather than implied by the code
//!
//! **Full bidirectionality, through rule L2 inclusively, over every case in
//! both files, with nothing excluded.** *Full bidirectionality* is UAX #9
//! section 4.2's name for an implementation that interprets every explicit
//! formatting character; *through rule L2 inclusively* is the phrase
//! `BidiCharacterTest.txt`'s header uses for what the two files can verify.
//! The rules in scope are therefore P2–P3, X1–X10, W1–W7, N0–N2, I1–I2, L1 and
//! L2, and [`OUTSIDE_THE_LEVEL`] lists the three that are not — P1, L3 and L4
//! — because both headers say no case exercises them. An exclusion may cite
//! one of those three and nothing else, so a case that fails for a rule inside
//! the level cannot be listed away without the level itself being renamed here,
//! in `TODO.md`, and in `text/src/bidi.rs`.
//!
//! # What is excluded, and where a reader finds it
//!
//! [`EXCLUDED`], below, and it is empty. It is a checked artefact rather than a
//! comment, which is RFC 0115's third obligation: a case that fails and is not
//! listed is red, a case that is listed and passes is red, a listed case the
//! file does not contain is red, and a listed case citing a rule the level
//! includes is red. So the list cannot drift in either direction without
//! somebody editing it, and every run prints how many cases each file ran,
//! passed and excluded, and by which rule.
//!
//! # The other three obligations RFC 0115 put on this file
//!
//! 1. **A run of zero is red, and so is a file that is not there.** Each file
//!    must yield cases, and the number [`Corpus::cases`] states for it;
//!    `BidiTest.txt`'s data lines must add up to the sum of its own `#Count:`
//!    lines; and a line that does not parse is a failure rather than a skip. A missing file fails by name with the command that
//!    restores it (RFC 0114); it does not skip.
//! 2. **The scalar that stands for a class comes from the generated table.**
//!    `BidiTest.txt` is written in classes, so each is run as the first scalar
//!    `text/src/bidi_class.rs` gives that class — read out of `BIDI_CLASS`, not
//!    typed here, because a typed list would make this a test of the list
//!    against the table it was copied from. The file assumes no bracket pairs,
//!    so a representative that is a bracket is refused rather than run.
//! 3. **It runs on both architectures and compares integers.** `f-text` is in
//!    `cargo xtask test-host` on the arm runner as well as here, and a level is
//!    a small integer and an order a permutation, so a divergence between the
//!    two would be a defect rather than a rounding.
//!
//! # Why a target of its own
//!
//! `harness = false`, for the reason `blob/tests/million.rs` gives and one of
//! its own: libtest captures what a passing test prints, and this run's counts
//! are the evidence the exit asks for, so they are printed on every run rather
//! than only on a failure.
//!
//! ```text
//! cargo test -p f-text --test bidi_conformance
//! ```

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::exit;

use f_text::bidi::{BaseDirection, Slot, line_levels, resolve, visual_order};
use f_text::bidi_class::{BIDI_CLASS, BidiClass, UNICODE_VERSION};
use f_text::property::{bidi_class, paired_bracket};

/// The name this target answers to when cargo's harness protocol asks.
const TEST_NAME: &str = "bidi_conformance";

/// The import both files are in, relative to the workspace root.
const CORPUS_DIR: &str = "third_party/unicode";

/// Failures printed in full before the rest are only counted. Unit: cases.
const SHOWN: usize = 20;

/// The level this run holds the implementation to, in words, as printed.
const LEVEL: &str = "full bidirectionality (UAX #9 section 4.2), through rule L2 inclusively: \
                     P2-P3, X1-X10, W1-W7, N0-N2, I1-I2, L1-L2";

/// The rules the named level does not include, each with why no case reaches
/// it. An exclusion may cite these and nothing else.
const OUTSIDE_THE_LEVEL: &[(&str, &str)] = &[
    ("P1", "paragraph splitting: every case in both files is one paragraph"),
    ("L3", "marks after a right-to-left base: platform-specific, and out of both files' scope"),
    ("L4", "mirrored glyphs: out of both files' scope, and needs Bidi_Mirrored"),
];

/// The two files.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Corpus {
    /// Sequences of classes, each under up to three paragraph directions.
    BidiTest,
    /// Sequences of scalars, brackets included, each under one direction.
    BidiCharacterTest,
}

impl Corpus {
    const ALL: [Self; 2] = [Self::BidiTest, Self::BidiCharacterTest];

    const fn file(self) -> &'static str {
        match self {
            Self::BidiTest => "BidiTest.txt",
            Self::BidiCharacterTest => "BidiCharacterTest.txt",
        }
    }

    /// The cases the file holds, as this run counts them. Unit: cases.
    ///
    /// A number about the corpus rather than about the implementation, written
    /// down so that a case deleted from either file is red here and not only
    /// in `cargo xtask lint`, whose hash check is the stronger guard but is not
    /// this run: `BidiTest.txt` also states its own counts and is held to them,
    /// and `BidiCharacterTest.txt` states none, so without this a deleted line
    /// there would be one fewer case passing. A Unicode version moves both,
    /// and moving them is a line of the diff that moves the version.
    const fn cases(self) -> usize {
        match self {
            Self::BidiTest => 770_241,
            Self::BidiCharacterTest => 91_707,
        }
    }
}

/// One case this run does not hold to the named level.
#[derive(Clone, Copy)]
struct Exclusion {
    corpus: Corpus,
    /// The case's line in the file. Unit: lines, counted from one.
    line: usize,
    /// The paragraph direction, since one line of `BidiTest.txt` is up to
    /// three cases.
    direction: BaseDirection,
    /// A rule from [`OUTSIDE_THE_LEVEL`].
    rule: &'static str,
    /// Why this case needs that rule, in a sentence.
    why: &'static str,
}

/// Every case excluded from the named level. Empty: every case in both files
/// passes, and one that stops passing is red rather than listed.
///
/// *What would reverse the emptiness:* a Unicode version whose files carry a
/// case that needs P1, L3 or L4 — the only rules an entry here may cite. A case
/// that fails for any other rule is a defect in `text/src/bidi.rs`, and adding
/// it here would be renaming the level, which is a diff to this file's first
/// section, to `TODO.md` and to that module's, and not to this list alone.
const EXCLUDED: &[Exclusion] = &[];

/// One case's inputs and the specification's answer.
struct Case<'a> {
    corpus: Corpus,
    line: usize,
    text: &'a [char],
    direction: BaseDirection,
    /// `BidiCharacterTest.txt` states it; `BidiTest.txt` does not.
    paragraph: Option<u8>,
    /// `None` where the file says `x`: removed by X9.
    levels: &'a [Option<u8>],
    order: &'a [usize],
}

/// What a file came to.
#[derive(Default)]
struct Tally {
    lines: usize,
    ran: usize,
    passed: usize,
    excluded: usize,
    failed: usize,
    /// Excluded cases, by the rule each cites.
    by_rule: BTreeMap<&'static str, usize>,
}

/// Everything the run found, in the order it found it.
struct Run {
    tallies: BTreeMap<Corpus, Tally>,
    findings: Vec<String>,
    failures_shown: usize,
    /// Which entries of [`EXCLUDED`] matched a case.
    matched: Vec<bool>,
    /// Buffers reused across the cases, because there are eight hundred
    /// thousand of them and none needs its own.
    work: Vec<Slot>,
    levels: Vec<u8>,
    order: Vec<u32>,
}

impl Run {
    fn finding(&mut self, text: String) {
        self.findings.push(text);
    }

    /// Run one case and file its outcome.
    fn case(&mut self, case: &Case<'_>) {
        let outcome = self.check(case);
        let excluded = EXCLUDED.iter().position(|e| {
            e.corpus == case.corpus && e.line == case.line && e.direction == case.direction
        });
        let tally = self.tallies.entry(case.corpus).or_default();
        tally.ran += 1;
        match (outcome, excluded) {
            (Ok(()), None) => tally.passed += 1,
            (Err(_), Some(at)) => {
                tally.excluded += 1;
                *tally.by_rule.entry(EXCLUDED[at].rule).or_default() += 1;
                self.matched[at] = true;
            }
            (Ok(()), Some(at)) => {
                tally.failed += 1;
                self.matched[at] = true;
                self.finding(format!(
                    "{}:{} {:?} passes and is in EXCLUDED, citing {}. An exclusion that is not \
                     needed widens the list without anything failing; remove it.",
                    case.corpus.file(),
                    case.line,
                    case.direction,
                    EXCLUDED[at].rule
                ));
            }
            (Err(why), None) => {
                tally.failed += 1;
                if self.failures_shown < SHOWN {
                    self.failures_shown += 1;
                    let scalars: Vec<String> =
                        case.text.iter().map(|&c| format!("{:04X}", u32::from(c))).collect();
                    self.finding(format!(
                        "{}:{} {:?} [{}]\n    {why}",
                        case.corpus.file(),
                        case.line,
                        case.direction,
                        scalars.join(" ")
                    ));
                }
            }
        }
    }

    /// The case against the implementation, or what differed.
    fn check(&mut self, case: &Case<'_>) -> Result<(), String> {
        let n = case.text.len();
        if case.levels.len() != n {
            return Err(format!("the file gives {} levels for {n} scalars", case.levels.len()));
        }
        self.work.clear();
        self.work.resize(n, Slot::EMPTY);
        let paragraph = resolve(case.text, case.direction, &mut self.work)
            .map_err(|refusal| format!("resolve refused: {refusal:?}"))?;
        if let Some(want) = case.paragraph
            && paragraph.level != want
        {
            return Err(format!("paragraph level {} where the file says {want}", paragraph.level));
        }
        self.levels.clear();
        self.levels.resize(n, 0);
        line_levels(&self.work, paragraph, &mut self.levels)
            .map_err(|refusal| format!("line_levels refused: {refusal:?}"))?;
        let got: Vec<Option<u8>> = self
            .work
            .iter()
            .zip(&self.levels)
            .map(|(slot, &level)| (!slot.removed()).then_some(level))
            .collect();
        if got != case.levels {
            return Err(format!(
                "levels {} where the file says {}",
                spell_levels(&got),
                spell_levels(case.levels)
            ));
        }
        self.order.clear();
        self.order.resize(n, 0);
        visual_order(&self.levels, &mut self.order)
            .map_err(|refusal| format!("visual_order refused: {refusal:?}"))?;
        let seen: Vec<usize> =
            self.order.iter().map(|&k| k as usize).filter(|&k| !self.work[k].removed()).collect();
        if seen != case.order {
            return Err(format!("order {seen:?} where the file says {:?}", case.order));
        }
        Ok(())
    }
}

fn spell_levels(levels: &[Option<u8>]) -> String {
    let spelled: Vec<String> =
        levels.iter().map(|l| l.map_or_else(|| "x".to_string(), |l| l.to_string())).collect();
    spelled.join(" ")
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

/// The scalar each class is run as: the first `text/src/bidi_class.rs` gives
/// it. RFC 0115's second obligation.
fn representatives() -> Result<Vec<(BidiClass, char)>, String> {
    let mut out = Vec::new();
    for &class in BidiClass::ALL {
        let &(start, _) = BIDI_CLASS
            .iter()
            .find(|(_, value)| *value == class)
            .ok_or_else(|| format!("{class:?} has no run in BIDI_CLASS"))?;
        let c = char::from_u32(start)
            .ok_or_else(|| format!("{class:?}'s first scalar U+{start:04X} is not a char"))?;
        if bidi_class(c) != class {
            return Err(format!("U+{start:04X} does not look up as {class:?}"));
        }
        if paired_bracket(c).is_some() {
            return Err(format!(
                "{class:?}'s first scalar U+{start:04X} is a bracket, and BidiTest.txt assumes \
                 none: running it would test N0 on cases written without it"
            ));
        }
        out.push((class, c));
    }
    Ok(out)
}

/// A line's `@Levels:` or `BidiCharacterTest.txt` field 3.
fn parse_levels(field: &str) -> Result<Vec<Option<u8>>, String> {
    field
        .split_whitespace()
        .map(|t| match t {
            "x" => Ok(None),
            n => n.parse::<u8>().map(Some).map_err(|_| format!("`{n}` is not a level")),
        })
        .collect()
}

fn parse_order(field: &str) -> Result<Vec<usize>, String> {
    field
        .split_whitespace()
        .map(|t| t.parse::<usize>().map_err(|_| format!("`{t}` is not an index")))
        .collect()
}

/// `BidiTest.txt`'s running state: the `@Levels` and `@Reorder` the next data
/// line is held to, and the two counts compared at the end.
struct ClassFile {
    names: BTreeMap<String, char>,
    levels: Option<Vec<Option<u8>>>,
    order: Option<Vec<usize>>,
    /// The sum of the file's own `#Count:` lines. Unit: data lines.
    counted: usize,
    /// Data lines read. Unit: data lines.
    data: usize,
    scalars: Vec<char>,
}

impl ClassFile {
    /// The paragraph directions a bitset names, in the file's bit order.
    const DIRECTIONS: [(u32, BaseDirection); 3] = [
        (1, BaseDirection::FirstStrong),
        (2, BaseDirection::LeftToRight),
        (4, BaseDirection::RightToLeft),
    ];

    fn line(&mut self, run: &mut Run, number: usize, line: &str) -> Result<(), String> {
        if let Some(rest) = line.strip_prefix("@Levels:") {
            self.levels = Some(parse_levels(rest)?);
        } else if let Some(rest) = line.strip_prefix("@Reorder:") {
            self.order = Some(parse_order(rest)?);
        } else if let Some(rest) = line.strip_prefix("#Count:") {
            self.counted += rest.trim().parse::<usize>().map_err(|_| "a bad #Count".to_string())?;
        } else if line.starts_with('@') || line.starts_with('#') || line.trim().is_empty() {
            // A comment, or an `@` line the header says is to be ignored.
        } else {
            self.data += 1;
            let (classes, bits) = line.split_once(';').ok_or("no `;`")?;
            let bits = u32::from_str_radix(bits.trim(), 16).map_err(|_| "a bad bitset")?;
            if bits == 0 || bits & !7 != 0 {
                return Err(format!("bitset {bits:#x} names no direction or an unknown one"));
            }
            self.scalars.clear();
            for name in classes.split_whitespace() {
                let c = self.names.get(name).ok_or_else(|| format!("no class `{name}`"))?;
                self.scalars.push(*c);
            }
            let levels = self.levels.as_deref().ok_or("data before any @Levels")?;
            let order = self.order.as_deref().ok_or("data before any @Reorder")?;
            for (bit, direction) in Self::DIRECTIONS {
                if bits & bit != 0 {
                    run.case(&Case {
                        corpus: Corpus::BidiTest,
                        line: number,
                        text: &self.scalars,
                        direction,
                        paragraph: None,
                        levels,
                        order,
                    });
                }
            }
        }
        Ok(())
    }
}

/// `BidiTest.txt`: classes, and up to three directions per line.
fn bidi_test(run: &mut Run, text: &str, stand_in: &[(BidiClass, char)]) {
    let corpus = Corpus::BidiTest;
    let mut file = ClassFile {
        names: stand_in.iter().map(|&(class, c)| (format!("{class:?}"), c)).collect(),
        levels: None,
        order: None,
        counted: 0,
        data: 0,
        scalars: Vec::new(),
    };
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if let Err(why) = file.line(run, index + 1, line) {
            run.finding(format!("{}:{} does not parse: {why}", corpus.file(), index + 1));
        }
    }
    run.tallies.entry(corpus).or_default().lines = file.data;
    if file.data != file.counted {
        run.finding(format!(
            "{} has {} data lines and its #Count lines add up to {}: the parser read a \
             different file from the one the file describes",
            corpus.file(),
            file.data,
            file.counted
        ));
    }
}

/// One line of `BidiCharacterTest.txt`: scalars, a direction, and every answer.
fn character_line(run: &mut Run, number: usize, line: &str) -> Result<(), String> {
    let fields: Vec<&str> = line.split(';').collect();
    let [points, direction, paragraph, levels, order] = fields[..] else {
        return Err(format!("{} fields where there are five", fields.len()));
    };
    let scalars = points
        .split_whitespace()
        .map(|h| {
            u32::from_str_radix(h, 16)
                .ok()
                .and_then(char::from_u32)
                .ok_or_else(|| format!("`{h}` is not a scalar"))
        })
        .collect::<Result<Vec<char>, String>>()?;
    let direction = match direction.trim() {
        "0" => BaseDirection::LeftToRight,
        "1" => BaseDirection::RightToLeft,
        "2" => BaseDirection::FirstStrong,
        other => return Err(format!("direction `{other}`")),
    };
    let paragraph =
        paragraph.trim().parse::<u8>().map_err(|_| "a bad paragraph level".to_string())?;
    let levels = parse_levels(levels)?;
    let order = parse_order(order)?;
    run.case(&Case {
        corpus: Corpus::BidiCharacterTest,
        line: number,
        text: &scalars,
        direction,
        paragraph: Some(paragraph),
        levels: &levels,
        order: &order,
    });
    Ok(())
}

/// `BidiCharacterTest.txt`, every line that is not a comment.
fn bidi_character_test(run: &mut Run, text: &str) {
    let corpus = Corpus::BidiCharacterTest;
    let mut data = 0usize;
    for (index, raw) in text.split('\n').enumerate() {
        let line = raw.strip_suffix('\r').unwrap_or(raw);
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        data += 1;
        if let Err(why) = character_line(run, index + 1, line) {
            run.finding(format!("{}:{} does not parse: {why}", corpus.file(), index + 1));
        }
    }
    run.tallies.entry(corpus).or_default().lines = data;
}

/// Every check on an exclusion list that does not need a case to run.
fn exclusion_findings(list: &[Exclusion]) -> Vec<String> {
    let mut findings = Vec::new();
    for (i, e) in list.iter().enumerate() {
        if !OUTSIDE_THE_LEVEL.iter().any(|(rule, _)| *rule == e.rule) {
            findings.push(format!(
                "EXCLUDED[{i}] ({}:{}) cites `{}`, which the named level includes. Only P1, \
                 L3 and L4 may excuse a case; anything else is a defect or a renamed level",
                e.corpus.file(),
                e.line,
                e.rule
            ));
        }
        if !e.why.ends_with('.') || e.why.len() < 24 {
            findings.push(format!("EXCLUDED[{i}] does not say why in a sentence"));
        }
        if list[..i]
            .iter()
            .any(|d| d.corpus == e.corpus && d.line == e.line && d.direction == e.direction)
        {
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

fn conformance() -> i32 {
    println!("{TEST_NAME}: Unicode {UNICODE_VERSION}, {LEVEL}");
    let stand_in = match representatives() {
        Ok(stand_in) => stand_in,
        Err(why) => {
            println!("\nfinding  {why}");
            return 1;
        }
    };
    let spelled: Vec<String> =
        stand_in.iter().map(|(class, c)| format!("{class:?}=U+{:04X}", u32::from(*c))).collect();
    println!("  BidiTest.txt's classes run as: {}", spelled.join(" "));

    let mut run = Run {
        tallies: BTreeMap::new(),
        findings: exclusion_findings(EXCLUDED),
        failures_shown: 0,
        matched: vec![false; EXCLUDED.len()],
        work: Vec::new(),
        levels: Vec::new(),
        order: Vec::new(),
    };
    // The control. With `EXCLUDED` empty, the checks above run over nothing
    // and could not fail, so they are fed an entry each must refuse: a rule the
    // level includes, a reason that is a label, and one case twice. A checker
    // that accepted it would make every later entry unchecked.
    let decoy = Exclusion {
        corpus: Corpus::BidiTest,
        line: 1,
        direction: BaseDirection::LeftToRight,
        rule: "W1",
        why: "flaky",
    };
    let twice = [decoy, Exclusion { rule: "L4", ..decoy }];
    if exclusion_findings(&twice).len() != 4 {
        run.finding(
            "the exclusion checks accept an entry citing W1 with no reason, listed twice, so \
             EXCLUDED is unchecked"
                .to_string(),
        );
    }
    for corpus in Corpus::ALL {
        match read(corpus) {
            Ok(text) => match corpus {
                Corpus::BidiTest => bidi_test(&mut run, &text, &stand_in),
                Corpus::BidiCharacterTest => bidi_character_test(&mut run, &text),
            },
            Err(why) => run.finding(why),
        }
    }
    for (i, e) in EXCLUDED.iter().enumerate() {
        if !run.matched[i] {
            let finding = format!(
                "EXCLUDED[{i}] names {}:{} {:?}, which is not a case in the file",
                e.corpus.file(),
                e.line,
                e.direction
            );
            run.finding(finding);
        }
    }

    for corpus in Corpus::ALL {
        let tally = run.tallies.remove(&corpus).unwrap_or_default();
        println!(
            "  {:<22} {:>6} lines, {:>6} cases: {:>6} passed, {} excluded, {} failed",
            corpus.file(),
            tally.lines,
            tally.ran,
            tally.passed,
            tally.excluded,
            tally.failed
        );
        for (rule, count) in &tally.by_rule {
            println!("  {:<22} {count} excluded under {rule}", "");
        }
        if tally.ran == 0 {
            run.findings.push(format!(
                "{} ran no cases, and a run of nothing is not a pass (RFC 0115)",
                corpus.file()
            ));
        } else if tally.ran != corpus.cases() {
            run.findings.push(format!(
                "{} yielded {} cases where it holds {}: a case was lost or gained between \
                 the file and this run",
                corpus.file(),
                tally.ran,
                corpus.cases()
            ));
        }
    }
    let rules: Vec<&str> = OUTSIDE_THE_LEVEL.iter().map(|(rule, _)| *rule).collect();
    println!(
        "  excluded: {} case(s), listed in EXCLUDED in text/tests/bidi_conformance.rs; an \
         exclusion may cite only {}",
        EXCLUDED.len(),
        rules.join(", ")
    );

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
