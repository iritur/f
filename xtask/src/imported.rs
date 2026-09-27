// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Everything in `xtask` that opens a file under `third_party/`: the check that
//! reads each import's `PROVENANCE.md` against the bytes it describes, and the
//! generator that re-spells Unicode's data files as Rust tables in `text/src/`.
//! RFC 0114, RFC 0138, RFC 0139.
//!
//! # Why one file
//!
//! RFC 0114's second check is that a path into the import is *named* rather than
//! merely unrefused, and `IMPORT_READERS` in `main.rs` is where it is named. A
//! row there is a file, so the fewer files in this crate that read imported
//! bytes the more that row says: this is the one, and `TOOLING`'s row names it.
//! What `lint-licensing` cannot see is a *second* file under `xtask/` that opens
//! the import, because this crate is exempt from the source checks for the
//! reason `TOOLING` gives — it contains the needles. That residue is stated
//! there and here rather than closed.
//!
//! # Why the generator is a command and not a build step
//!
//! `BUILD_SCRIPT_ALLOW` is empty and the emptiness is the check (RFC 0092). A
//! command somebody runs, with its output committed and reviewed, is the route
//! RFC 0114 left standing; and `lint-licensing` regenerates in memory and
//! compares, so a table nobody regenerated is a red lint rather than a table
//! that rots.
//!
//! # What the generator is allowed to decide
//!
//! Nothing, which is RFC 0114's first reversal condition: *the day the generator
//! makes decisions of its own — collapsing classes, choosing a default for an
//! unassigned scalar — what is committed is code derived from somebody else's
//! data*. So every value, every default and every name below comes from the
//! upstream file, and where the file could be read two ways the generator
//! refuses instead of picking one:
//!
//! - **Defaults** are the file's own `@missing` lines. The first must cover the
//!   whole code space; later ones override it for their ranges and must not
//!   overlap each other, so the order they are applied in decides nothing. An
//!   overlap is a refusal, because resolving it would be a precedence rule of
//!   this tree's. A file with no `@missing` line is read only if it lists every
//!   code point itself — `DerivedGeneralCategory.txt` does, `Cn` included — so
//!   no default is needed and none is chosen; one unlisted scalar is a refusal.
//!   A binary property's default is the file's own sentence, `All omitted code
//!   points have <Property>=No`, and a file that does not say it is refused
//!   (RFC 0139).
//! - **One property of several.** `DerivedCoreProperties.txt` and
//!   `emoji-data.txt` each hold many properties, and the generator reads the
//!   lines of the one a row names and no others: by its short name in the
//!   middle field for an enumerated property (`InCB`), by its name in the last
//!   for a binary one (`Extended_Pictographic`). Nothing is merged across
//!   properties.
//! - **Totals.** Where the file states `# Total code points: N` under a
//!   `# <Property>=<Value>` heading, the resolved table must agree, which is the
//!   upstream file checking this reading of its defaults — `L` is 1,095,407 code
//!   points only if every unassigned scalar landed where the file says. A total
//!   with no heading counts too when every line since the last total carried one
//!   value, which is how `GraphemeBreakProperty.txt` states its fourteen; a
//!   total after lines of two values is not attributed to either. A binary
//!   property's `# Total elements: N` is held the same way.
//! - **Names** are the file's short values, which are the names UAX #9's rules
//!   use. A long name appears only where the file spells one in a heading, and
//!   an underscore is dropped from a variant because Rust's camel case requires
//!   it, with the file's spelling kept in the variant's documentation.
//!
//! Merging adjacent code points of one value into one run is the only
//! transformation, and it loses nothing: the table and the file answer the same
//! question for every scalar.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// The one data import this module generates from. Spelled here once, because
/// this file is `IMPORT_READERS`' row for the generator and the literal is what
/// that row counts.
pub const IMPORT_DIR: &str = "third_party/unicode";

/// The command that regenerates every table `DERIVED_DATA` names, as each
/// generated header spells it.
pub const REGENERATE: &str = "cargo xtask unicode";

/// The only file names a data import may hold besides `.txt`. RFC 0114's third
/// category is *files no compiler in this workspace reads*, and an allow-list of
/// extensions holds that line where a deny-list of the languages somebody
/// thought of would not.
const DATA_NAMES: &[&str] = &["LICENSE", "PROVENANCE.md"];

/// The extensions a data import may hold.
const DATA_EXTENSIONS: &[&str] = &["txt"];

/// The one font file a data import may hold, by extension *and* by its first
/// four bytes, and the rule is that narrow on purpose. RFC 0141.
///
/// RFC 0114's category is *files no compiler in this workspace reads*, held by
/// an allow-list of extensions so that a language nobody thought of is refused
/// by default. A face is not text, so `txt` does not admit it; and a face is
/// not inert in general either — a TrueType file carries hinting programs, which
/// are instructions for an interpreter, and a CFF face carries charstrings,
/// which are too. So the rule admits exactly what this tree holds and argues
/// it: `ttf`, whose bytes begin with the TrueType `sfntVersion` `00 01 00 00`
/// (so a renamed CFF face, `OTTO`, a collection, `ttcf`, or a wrapper is still
/// refused), in a data import. What reads it here is the shaper behind the
/// licence boundary — `GSUB`, `GPOS`, `cmap`, `hmtx` — and `f_text::face`'s
/// admission, which reads four tables of integers. **Nothing in this tree runs
/// its hinting programs**, which is the sentence that makes it data here; the
/// day something does — Skrifa's hinter, `docs/TECHNICAL-DEBT.md`'s planned
/// import — that sentence is false and this rule is the one to revisit, with
/// the interpreter behind the boundary like the shaper.
///
/// Not `otf`, not `woff`, not `ttc`, and not a list: a second font format is a
/// second argument, made on the day there is a face in it. `the_font_rule_is_
/// one_extension_and_one_magic` holds all three refusals.
const DATA_FONT: (&str, [u8; 4]) = ("ttf", [0x00, 0x01, 0x00, 0x00]);

/// Is `file`, whose first bytes are `head`, the one font a data import admits?
fn is_admitted_font(file: &str, head: &[u8]) -> bool {
    let (extension, magic) = DATA_FONT;
    file.rsplit_once('.').is_some_and(|(_, ext)| ext == extension) && head.starts_with(&magic)
}

/// How one upstream file becomes one table.
///
/// A row of [`SHAPES`] and a row of `DERIVED_DATA` are the whole cost of a new
/// table. `E3-B03f`'s `Line_Break`, `Grapheme_Cluster_Break`, `East_Asian_Width`
/// and `General_Category` are [`Shape::Ranges`] rows like `Bidi_Class`;
/// `Indic_Conjunct_Break` and `Extended_Pictographic` needed the two shapes
/// after it, because their files hold other properties beside them (RFC 0139).
#[derive(Clone, Copy)]
pub enum Shape {
    /// An enumerated property as `range ; value` lines with `@missing` defaults.
    Ranges {
        /// The property's name as the file's `# <Property>=<Value>` headings
        /// spell it, which is where long names and totals are read from.
        property: &'static str,
        /// The Rust type the values become.
        type_name: &'static str,
        /// The name of the table of runs.
        table: &'static str,
    },
    /// An enumerated property as `range ; <short> ; value` lines in a file that
    /// holds other properties too: only lines whose middle field is `short` are
    /// read, `@missing` lines included.
    Field {
        /// The property's name as the file's headings spell it.
        property: &'static str,
        /// The short name the file's lines carry in their middle field.
        short: &'static str,
        /// The Rust type the values become.
        type_name: &'static str,
        /// The name of the table of runs.
        table: &'static str,
    },
    /// A binary property as `range ; <property>` lines in a file that holds
    /// other properties too, with the default the file states in words.
    Binary {
        /// The property's name as the file's lines spell it.
        property: &'static str,
        /// The name of the table of runs.
        table: &'static str,
    },
    /// `BidiBrackets.txt`: a code point, its paired bracket, and `o` or `c`.
    Brackets {
        /// The name of the table of pairs.
        table: &'static str,
    },
}

/// Each generated file and how it is made.
pub const SHAPES: &[(&str, Shape)] = &[
    (
        "text/src/bidi_class.rs",
        Shape::Ranges { property: "Bidi_Class", type_name: "BidiClass", table: "BIDI_CLASS" },
    ),
    ("text/src/bidi_brackets.rs", Shape::Brackets { table: "BIDI_BRACKETS" }),
    (
        "text/src/line_break.rs",
        Shape::Ranges { property: "Line_Break", type_name: "LineBreak", table: "LINE_BREAK" },
    ),
    (
        "text/src/grapheme_break.rs",
        Shape::Ranges {
            property: "Grapheme_Cluster_Break",
            type_name: "GraphemeClusterBreak",
            table: "GRAPHEME_CLUSTER_BREAK",
        },
    ),
    (
        "text/src/east_asian_width.rs",
        Shape::Ranges {
            property: "East_Asian_Width",
            type_name: "EastAsianWidth",
            table: "EAST_ASIAN_WIDTH",
        },
    ),
    (
        "text/src/general_category.rs",
        Shape::Ranges {
            property: "General_Category",
            type_name: "GeneralCategory",
            table: "GENERAL_CATEGORY",
        },
    ),
    (
        "text/src/indic_conjunct_break.rs",
        Shape::Field {
            property: "Indic_Conjunct_Break",
            short: "InCB",
            type_name: "IndicConjunctBreak",
            table: "INDIC_CONJUNCT_BREAK",
        },
    ),
    (
        "text/src/extended_pictographic.rs",
        Shape::Binary { property: "Extended_Pictographic", table: "EXTENDED_PICTOGRAPHIC" },
    ),
];

/// Every code point, as a `u32`. Unit: code points.
const CODE_SPACE: u32 = 0x11_0000;

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    crate::pack::hex(&f_hash::sha256(bytes))
}

/// The version a file states about itself, in full.
///
/// A UCD file says it in its first line, `# <Name>-<X.Y.Z>.txt`, and that is
/// the answer. `emoji-data.txt` says only `# Version: X.Y` in its header, which
/// is not enough to name the file's URL, so the third component comes from the
/// import's record — `PROVENANCE.md`'s `Version`, which `lint-licensing` reads
/// against every file that states one in full — and only if the record is a
/// release of what the file said. A file that states neither is refused.
fn stated_version(at: &Path, text: &str) -> Result<String, String> {
    if let Some(version) = self_stated_version(text) {
        return Ok(version.to_string());
    }
    let short = text
        .split('\n')
        .take_while(|line| line.starts_with('#') || line.trim().is_empty())
        .find_map(|line| line.strip_prefix("# Version: "))
        .map(str::trim)
        .ok_or(
            "states no version: neither `# <Name>-<X.Y.Z>.txt` in its first line nor a \
                `# Version:` line in its header",
        )?;
    let record = format!("{IMPORT_DIR}/PROVENANCE.md");
    let recorded = std::fs::read_to_string(at.join(&record))
        .ok()
        .and_then(|text| parse_provenance(&record, &text).0.fields.get("Version").cloned())
        .ok_or_else(|| {
            format!("states `{short}` and {record} records no Version to complete it")
        })?;
    let agrees = recorded
        .strip_prefix(short)
        .and_then(|rest| rest.strip_prefix('.'))
        .is_some_and(|z| !z.is_empty() && z.bytes().all(|b| b.is_ascii_digit()));
    if !agrees {
        return Err(format!(
            "states version `{short}` and {record} records `{recorded}`, which is not a release \
             of it"
        ));
    }
    Ok(recorded)
}

/// The version a UCD file states in its own first line, `# <Name>-<X.Y.Z>.txt`.
fn self_stated_version(text: &str) -> Option<&str> {
    let first = text.split('\n').next()?.strip_prefix("# ")?.strip_suffix(".txt")?;
    let (_, version) = first.rsplit_once('-')?;
    let parts: Vec<&str> = version.split('.').collect();
    (parts.len() == 3
        && parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())))
    .then_some(version)
}

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// The generated text for one `DERIVED_DATA` row, from the file on disk.
///
/// # Errors
///
/// No shape for `output`, an upstream path outside [`IMPORT_DIR`], an input
/// whose SHA-256 is not `sha256`, a file that states no version, or a file this
/// shape cannot read without deciding something — each named.
pub fn generate(at: &Path, output: &str, upstream: &str, sha256: &str) -> Result<String, String> {
    let shape = SHAPES
        .iter()
        .find(|(file, _)| *file == output)
        .map(|(_, shape)| *shape)
        .ok_or_else(|| format!("`{output}` has a DERIVED_DATA row and no shape in SHAPES"))?;
    let inside = upstream
        .strip_prefix(IMPORT_DIR)
        .and_then(|rest| rest.strip_prefix('/'))
        .ok_or_else(|| format!("`{upstream}` is not under {IMPORT_DIR}/"))?;
    let bytes = std::fs::read(at.join(upstream)).map_err(|e| {
        format!(
            "reading {upstream}: {e}\n  the table `{output}` is generated from it, and a \
             missing input is a failure rather than a skip (RFC 0114)"
        )
    })?;
    let actual = sha256_hex(&bytes);
    if actual != sha256 {
        return Err(format!(
            "{upstream} is not the file DERIVED_DATA names for `{output}`:\n  \
             DERIVED_DATA says {sha256}\n  the file hashes to {actual}\n  \
             An input that is not byte-identical to the one recorded is not the \
             specification's data; a version bump changes PROVENANCE.md, DERIVED_DATA \
             and the table together"
        ));
    }
    let text = String::from_utf8(bytes).map_err(|e| format!("{upstream} is not UTF-8: {e}"))?;
    let version = stated_version(at, &text).map_err(|e| format!("{upstream}: {e}"))?;
    let header = Header { upstream, inside, sha256, version: &version };
    match shape {
        Shape::Ranges { property, type_name, table } => {
            let resolved =
                resolve_ranges(&text, property).map_err(|e| format!("{upstream}: {e}"))?;
            render_ranges(&header, property, type_name, table, &resolved)
        }
        Shape::Field { property, short, type_name, table } => {
            let resolved = resolve_field(&text, property, Some(short))
                .map_err(|e| format!("{upstream}: {e}"))?;
            render_ranges(&header, property, type_name, table, &resolved)
        }
        Shape::Binary { property, table } => {
            let resolved =
                resolve_binary(&text, property).map_err(|e| format!("{upstream}: {e}"))?;
            Ok(render_binary(&header, property, table, &resolved))
        }
        Shape::Brackets { table } => {
            let pairs = parse_brackets(&text).map_err(|e| format!("{upstream}: {e}"))?;
            Ok(render_brackets(&header, table, &pairs))
        }
    }
}

/// Every committed table that is not what [`generate`] writes, and every shape
/// with no row. The number is how many rows were compared; a row that cannot
/// be regenerated at all is a finding rather than an error, so one bad row does
/// not hide the others.
pub fn table_findings(at: &Path, rows: &[(&str, &str, &str)]) -> (Vec<String>, usize) {
    let mut findings = Vec::new();
    let mut compared = 0usize;
    for (output, upstream, sha256) in rows {
        let want = match generate(at, output, upstream, sha256) {
            Ok(want) => want,
            Err(e) => {
                findings.push(format!("  {output}  cannot be regenerated: {e}"));
                continue;
            }
        };
        compared += 1;
        let have = std::fs::read(at.join(output)).unwrap_or_default();
        if have != want.as_bytes() {
            let line = first_difference(&String::from_utf8_lossy(&have), &want);
            findings.push(format!(
                "  {output}:{line}  is not what `{REGENERATE}` writes from {upstream}"
            ));
        }
    }
    for (output, _) in SHAPES {
        if !rows.iter().any(|(file, _, _)| file == output) {
            findings.push(format!(
                "  {output}  has a shape in SHAPES and no DERIVED_DATA row, so nothing checks \
                 its licence header or regenerates it"
            ));
        }
    }
    (findings, compared)
}

/// The first line, counted from one, on which two texts differ.
fn first_difference(have: &str, want: &str) -> usize {
    let mut a = have.split('\n');
    let mut b = want.split('\n');
    let mut n = 1;
    loop {
        match (a.next(), b.next()) {
            (None, None) => return n,
            (x, y) if x != y => return n,
            _ => n += 1,
        }
    }
}

/// `cargo xtask unicode`: write every table, or with `--check` report drift.
///
/// # Errors
///
/// A table that could not be generated, or under `--check` one that differs.
pub fn command(at: &Path, rows: &[(&str, &str, &str)], mode: Option<&str>) -> Result<(), String> {
    let check = match mode {
        None => false,
        Some("--check") => true,
        Some(other) => return Err(format!("unknown option for unicode: {other}")),
    };
    if check {
        let (findings, compared) = table_findings(at, rows);
        if findings.is_empty() {
            println!("unicode: ok  ({compared} table(s) are what the generator writes)");
            return Ok(());
        }
        return Err(format!("{} table(s) drifted:\n{}", findings.len(), findings.join("\n")));
    }
    for (output, upstream, sha256) in rows {
        let text = generate(at, output, upstream, sha256)?;
        let path = at.join(output);
        let before = std::fs::read(&path).unwrap_or_default();
        if before == text.as_bytes() {
            println!("unicode: {output} unchanged");
        } else {
            std::fs::write(&path, text.as_bytes()).map_err(|e| format!("writing {output}: {e}"))?;
            println!("unicode: {output} written from {upstream}");
        }
    }
    Ok(())
}

/// What every generated header names.
struct Header<'a> {
    upstream: &'a str,
    inside: &'a str,
    sha256: &'a str,
    version: &'a str,
}

impl Header<'_> {
    fn render(&self, what: &str) -> String {
        let mut out = String::new();
        let _ = writeln!(out, "// SPDX-License-Identifier: (Apache-2.0 OR MIT) AND Unicode-3.0");
        let _ = writeln!(out, "//");
        let _ = writeln!(
            out,
            "// Generated from Unicode's data by `{REGENERATE}`. Do not edit: a change"
        );
        let _ = writeln!(
            out,
            "// here is a change to `xtask/src/imported.rs`, regenerated and reviewed."
        );
        let _ = writeln!(out, "//");
        let _ = writeln!(out, "// Upstream-File: {}", self.upstream);
        let _ = writeln!(
            out,
            "// Upstream-URL: https://www.unicode.org/Public/{}/ucd/{}",
            self.version, self.inside
        );
        let _ = writeln!(out, "// Upstream-SHA-256: {}", self.sha256);
        let _ = writeln!(out, "// Unicode-Version: {}", self.version);
        let _ = writeln!(out, "// Regenerate: {REGENERATE}");
        let _ = writeln!(out, "//");
        let _ = writeln!(out, "// Two licences, because what follows is {what} re-spelled");
        let _ = writeln!(out, "// as Rust. `third_party/unicode/LICENSE` is Unicode's notice, and");
        let _ = writeln!(
            out,
            "// `LICENSING.md` says what a redistributor of a binary owes, since a binary"
        );
        let _ = writeln!(out, "// carries no header. RFC 0114.");
        let _ = writeln!(out);
        out
    }
}

/// One enumerated property, resolved over the whole code space.
struct Resolved {
    /// Each value's upstream spelling, in the order the file first names it.
    values: Vec<String>,
    /// Long names, where the file spells one in a heading.
    long: BTreeMap<String, String>,
    /// The first code point of each maximal run, and its value's index.
    runs: Vec<(u32, usize)>,
    /// How many totals the file stated and this reading reproduced.
    totals: usize,
}

fn parse_range(field: &str) -> Result<(u32, u32), String> {
    let hex = |s: &str| {
        u32::from_str_radix(s.trim(), 16).map_err(|_| format!("`{s}` is not a code point"))
    };
    let (start, end) = match field.split_once("..") {
        Some((a, b)) => (hex(a)?, hex(b)?),
        None => (hex(field)?, hex(field)?),
    };
    if start > end || end >= CODE_SPACE {
        return Err(format!("`{field}` is not a range inside the code space"));
    }
    Ok((start, end))
}

/// Read `range ; value` lines and the file's own defaults into one value per
/// code point. The module documentation says what may and may not be decided.
fn resolve_ranges(text: &str, property: &str) -> Result<Resolved, String> {
    resolve_field(text, property, None)
}

/// [`resolve_ranges`], or with `select` the `range ; <select> ; value` lines of
/// one property in a file that holds several, every other line skipped.
fn resolve_field(text: &str, property: &str, select: Option<&str>) -> Result<Resolved, String> {
    // The fields of a line that belong to this property, or `None` for a line
    // of another one. Without `select`, every line is this property's.
    fn pick<'a>(select: Option<&str>, fields: Vec<&'a str>) -> Option<Vec<&'a str>> {
        match (select, fields.as_slice()) {
            (None, _) => Some(fields),
            (Some(short), [range, name, value]) if *name == short => Some(vec![*range, *value]),
            (Some(_), _) => None,
        }
    }
    let ours = |fields| pick(select, fields);
    let heading = format!("# {property}=");
    let mut values: Vec<String> = Vec::new();
    let mut long: BTreeMap<String, String> = BTreeMap::new();
    let mut explicit: Vec<(u32, u32, String, usize)> = Vec::new();
    let mut missing: Vec<(u32, u32, String, usize)> = Vec::new();
    let mut stated: Vec<(String, u64, usize)> = Vec::new();
    let mut pending: Option<String> = None;
    let mut block: Option<String> = None;
    // The one value every line of this property carried since the last total,
    // or `Some(None)` once two differed. `GraphemeBreakProperty.txt` states a
    // total after each value's lines and heads none of them, and a total the
    // file states is a check on this reading whether or not a heading names it.
    let mut since: Option<Option<String>> = None;

    for (index, line) in text.split('\n').enumerate() {
        let n = index + 1;
        if let Some(rest) = line.strip_prefix("# @missing:") {
            let Some(fields) = ours(rest.split(';').map(str::trim).collect()) else { continue };
            let [range, value] = fields[..] else {
                return Err(format!("line {n}: an `@missing` line with other than two fields"));
            };
            let (a, b) = parse_range(range).map_err(|e| format!("line {n}: {e}"))?;
            missing.push((a, b, value.to_string(), n));
            continue;
        }
        if let Some(name) = line.strip_prefix(&heading) {
            pending = Some(name.trim().to_string());
            block = None;
            since = None;
            continue;
        }
        if let Some(count) = line.strip_prefix("# Total code points:") {
            let unheaded = since.take().flatten();
            if let Some(value) = block.take().or(unheaded) {
                let count: u64 =
                    count.trim().parse().map_err(|_| format!("line {n}: an unreadable total"))?;
                stated.push((value, count, n));
            }
            continue;
        }
        let data = line.split('#').next().unwrap_or("").trim();
        if data.is_empty() {
            continue;
        }
        let Some(fields) = ours(data.split(';').map(str::trim).collect()) else { continue };
        let [range, value] = fields[..] else {
            return Err(format!("line {n}: a data line with other than two fields"));
        };
        let (a, b) = parse_range(range).map_err(|e| format!("line {n}: {e}"))?;
        if !values.iter().any(|v| v == value) {
            values.push(value.to_string());
        }
        since = match since {
            None => Some(Some(value.to_string())),
            Some(Some(v)) if v == value => Some(Some(v)),
            Some(_) => Some(None),
        };
        if let Some(name) = pending.take() {
            if let Some(previous) = long.insert(value.to_string(), name.clone())
                && previous != name
            {
                return Err(format!(
                    "line {n}: `{value}` is headed both `{previous}` and `{name}`"
                ));
            }
            block = Some(value.to_string());
        }
        explicit.push((a, b, value.to_string(), n));
    }

    // A default's value is spelled long in some files and short in others; the
    // headings are the only alias table this reads, and a name found in neither
    // is a value of its own — `Grapheme_Cluster_Break=Other` has no data lines.
    let short_of = |name: &str, values: &mut Vec<String>| -> usize {
        if let Some(i) = values.iter().position(|v| v == name) {
            return i;
        }
        if let Some((short, _)) = long.iter().find(|(_, l)| l.as_str() == name)
            && let Some(i) = values.iter().position(|v| v == short)
        {
            return i;
        }
        values.push(name.to_string());
        values.len() - 1
    };

    // No `@missing` line is a file that must list every code point itself; that
    // is checked once the explicit lines are applied, below.
    if let Some(first) = missing.first()
        && (first.0, first.1) != (0, CODE_SPACE - 1)
    {
        return Err(format!(
            "line {}: the first `@missing` line does not cover the code space",
            first.3
        ));
    }
    for (i, a) in missing.iter().enumerate().skip(1) {
        for b in missing.iter().skip(i + 1) {
            if a.0 <= b.1 && b.0 <= a.1 {
                return Err(format!(
                    "lines {} and {}: two `@missing` ranges overlap, and applying them in \
                     either order would be a precedence rule of this tree's",
                    a.3, b.3
                ));
            }
        }
    }

    let unset = usize::MAX;
    let mut table = vec![unset; CODE_SPACE as usize];
    for (a, b, value, _) in &missing {
        let v = short_of(value, &mut values);
        for slot in &mut table[*a as usize..=*b as usize] {
            *slot = v;
        }
    }
    let mut listed = vec![false; CODE_SPACE as usize];
    for (a, b, value, n) in &explicit {
        let v = short_of(value, &mut values);
        for cp in *a..=*b {
            if listed[cp as usize] {
                return Err(format!("line {n}: U+{cp:04X} is listed twice"));
            }
            listed[cp as usize] = true;
            table[cp as usize] = v;
        }
    }

    if let Some(cp) = table.iter().position(|v| *v == unset) {
        return Err(format!(
            "no `@missing` line covers U+{cp:04X} and no line lists it, so it has no value the \
             file gives it and this would have to choose one"
        ));
    }
    let mut counts = vec![0u64; values.len()];
    for v in &table {
        counts[*v] += 1;
    }
    for (value, count, n) in &stated {
        let v = short_of(value, &mut values);
        let have = counts.get(v).copied().unwrap_or(0);
        if have != *count {
            return Err(format!(
                "line {n}: the file says `{value}` covers {count} code point(s) and this \
                 reading of it gives {have}"
            ));
        }
    }

    let mut runs = Vec::new();
    for (cp, v) in table.iter().enumerate() {
        if runs.last().is_none_or(|(_, last)| last != v) {
            runs.push((u32::try_from(cp).unwrap_or(u32::MAX), *v));
        }
    }
    Ok(Resolved { values, long, runs, totals: stated.len() })
}

/// A value's name as a Rust variant: the upstream spelling with any underscore
/// removed, which is the one change camel case requires.
fn variant(value: &str) -> Result<String, String> {
    let name: String = value.chars().filter(|c| *c != '_').collect();
    let mut chars = name.chars();
    let ok = chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric());
    if ok { Ok(name) } else { Err(format!("`{value}` is not a name this can spell as a variant")) }
}

fn render_ranges(
    header: &Header<'_>,
    property: &str,
    type_name: &str,
    table: &str,
    resolved: &Resolved,
) -> Result<String, String> {
    let names: Vec<String> =
        resolved.values.iter().map(|v| variant(v)).collect::<Result<_, _>>()?;
    let distinct: BTreeSet<&String> = names.iter().collect();
    if distinct.len() != names.len() {
        return Err("two values spell the same variant once underscores are removed".into());
    }

    let mut out = header.render(&format!("Unicode's `{property}`"));
    let _ = writeln!(
        out,
        "//! `{property}` for every code point, as runs, from Unicode {}.",
        header.version
    );
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! [`{table}`] holds the first code point of each maximal run of one value, in"
    );
    let _ = writeln!(
        out,
        "//! ascending order from zero, so the value of a code point is the value of the"
    );
    let _ = writeln!(
        out,
        "//! last entry whose start is at most it. A code point the upstream file does not"
    );
    let _ = writeln!(
        out,
        "//! list carries the value its `@missing` lines give it, and where the file"
    );
    let _ = writeln!(
        out,
        "//! states a total per value, this table reproduces it ({} of {} checked).",
        resolved.totals,
        resolved.values.len()
    );
    let _ = writeln!(
        out,
        "//! The lookup is not here: this file is data under two licences, and code in it"
    );
    let _ = writeln!(out, "//! would be too. [`crate::property`] reads it.");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// The Unicode version this table was generated from.");
    let _ = writeln!(out, "pub const UNICODE_VERSION: &str = \"{}\";", header.version);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// A value of `{property}`, by the name the upstream file gives it, in the"
    );
    let _ = writeln!(out, "/// order the file first names it.");
    let _ = writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]");
    let _ = writeln!(out, "#[allow(");
    let _ = writeln!(out, "    clippy::upper_case_acronyms,");
    let _ = writeln!(
        out,
        "    reason = \"the upstream file's own names, which the algorithm's rules use\""
    );
    let _ = writeln!(out, ")]");
    let _ = writeln!(out, "pub enum {type_name} {{");
    for (value, name) in resolved.values.iter().zip(&names) {
        match resolved.long.get(value) {
            Some(long) => {
                let _ = writeln!(out, "    /// `{long}`.");
            }
            None => {
                let _ = writeln!(out, "    /// `{value}`, as the upstream file spells it.");
            }
        }
        let _ = writeln!(out, "    {name},");
    }
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(out, "impl {type_name} {{");
    let _ = writeln!(out, "    /// Every value, in declaration order.");
    let _ = writeln!(out, "    #[rustfmt::skip]");
    let _ = writeln!(out, "    pub const ALL: &[{type_name}] = &[");
    for name in &names {
        let _ = writeln!(out, "        {type_name}::{name},");
    }
    let _ = writeln!(out, "    ];");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// The first code point of each run of one `{property}` value, ascending from zero."
    );
    let _ = writeln!(out, "#[rustfmt::skip]");
    let _ = writeln!(out, "pub const {table}: &[(u32, {type_name})] = &[");
    for (start, v) in &resolved.runs {
        let _ = writeln!(out, "    (0x{start:04X}, {type_name}::{}),", names[*v]);
    }
    let _ = writeln!(out, "];");
    Ok(out)
}

/// One binary property, resolved over the whole code space.
struct Binary {
    /// The first code point of each maximal run, and whether it has the
    /// property. The first run starts at zero and the runs alternate.
    runs: Vec<(u32, bool)>,
    /// Code points with the property. Unit: code points.
    count: u64,
}

/// Read the `range ; <property>` lines of one binary property out of a file
/// that holds several. The default is the file's own sentence for it, and the
/// file's `# Total elements:` after the property's lines must be the count.
fn resolve_binary(text: &str, property: &str) -> Result<Binary, String> {
    let default = format!("# All omitted code points have {property}=No");
    let stated = text.split('\n').filter(|line| line.trim_end() == default).count();
    if stated != 1 {
        return Err(format!(
            "`{default}` appears {stated} time(s), and it is the only default this reads for a \
             binary property: without it an unlisted code point has no value the file gives it"
        ));
    }
    let mut has = vec![false; CODE_SPACE as usize];
    let mut seen_ours = false;
    let mut total: Option<(u64, usize)> = None;
    for (index, line) in text.split('\n').enumerate() {
        let n = index + 1;
        if let Some(count) = line.strip_prefix("# Total elements:") {
            if seen_ours && total.is_none() {
                let count: u64 =
                    count.trim().parse().map_err(|_| format!("line {n}: an unreadable total"))?;
                total = Some((count, n));
            }
            continue;
        }
        let data = line.split('#').next().unwrap_or("").trim();
        if data.is_empty() {
            continue;
        }
        let fields: Vec<&str> = data.split(';').map(str::trim).collect();
        let [range, name] = fields[..] else {
            return Err(format!("line {n}: a data line with other than two fields"));
        };
        if name != property {
            continue;
        }
        if total.is_some() {
            return Err(format!("line {n}: `{property}` resumes after its stated total"));
        }
        seen_ours = true;
        let (a, b) = parse_range(range).map_err(|e| format!("line {n}: {e}"))?;
        for cp in a..=b {
            if has[cp as usize] {
                return Err(format!("line {n}: U+{cp:04X} is listed twice"));
            }
            has[cp as usize] = true;
        }
    }
    let count = has.iter().filter(|h| **h).count() as u64;
    let Some((stated, n)) = total else {
        return Err(format!("no `# Total elements:` follows `{property}`'s lines"));
    };
    if stated != count {
        return Err(format!(
            "line {n}: the file says `{property}` covers {stated} element(s) and this reading \
             of it gives {count}"
        ));
    }
    let mut runs: Vec<(u32, bool)> = Vec::new();
    for (cp, v) in has.iter().enumerate() {
        if runs.last().is_none_or(|(_, last)| last != v) {
            runs.push((u32::try_from(cp).unwrap_or(u32::MAX), *v));
        }
    }
    Ok(Binary { runs, count })
}

fn render_binary(header: &Header<'_>, property: &str, table: &str, resolved: &Binary) -> String {
    let mut out = header.render(&format!("Unicode's `{property}`"));
    let _ = writeln!(
        out,
        "//! `{property}` for every code point, as runs, from Unicode {}.",
        header.version
    );
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! [`{table}`] holds the first code point of each maximal run of one value, in"
    );
    let _ = writeln!(
        out,
        "//! ascending order from zero, so whether a code point has the property is the"
    );
    let _ = writeln!(
        out,
        "//! value of the last entry whose start is at most it, and the runs alternate. A"
    );
    let _ = writeln!(
        out,
        "//! code point the upstream file does not list does not have the property, which"
    );
    let _ = writeln!(
        out,
        "//! is the file's own sentence, and the {} that do reproduce its stated total.",
        resolved.count
    );
    let _ = writeln!(
        out,
        "//! The lookup is not here: this file is data under two licences, and code in it"
    );
    let _ = writeln!(out, "//! would be too. [`crate::property`] reads it.");
    let _ = writeln!(out);
    let _ = writeln!(out, "/// The Unicode version this table was generated from.");
    let _ = writeln!(out, "pub const UNICODE_VERSION: &str = \"{}\";", header.version);
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// The first code point of each run, and whether the run has `{property}`."
    );
    let _ = writeln!(out, "#[rustfmt::skip]");
    let _ = writeln!(out, "pub const {table}: &[(u32, bool)] = &[");
    for (start, v) in &resolved.runs {
        let _ = writeln!(out, "    (0x{start:04X}, {v}),");
    }
    let _ = writeln!(out, "];");
    out
}

/// One line of `BidiBrackets.txt`: the bracket, its pair, and whether it opens.
type Pair = (u32, u32, bool);

fn parse_brackets(text: &str) -> Result<Vec<Pair>, String> {
    let mut pairs: Vec<Pair> = Vec::new();
    for (index, line) in text.split('\n').enumerate() {
        let n = index + 1;
        let data = line.split('#').next().unwrap_or("").trim();
        if data.is_empty() {
            continue;
        }
        let fields: Vec<&str> = data.split(';').map(str::trim).collect();
        let [cp, pair, kind] = fields[..] else {
            return Err(format!("line {n}: a data line with other than three fields"));
        };
        let (cp, _) = parse_range(cp).map_err(|e| format!("line {n}: {e}"))?;
        let (pair, _) = parse_range(pair).map_err(|e| format!("line {n}: {e}"))?;
        let open = match kind {
            "o" => true,
            "c" => false,
            other => {
                return Err(format!(
                    "line {n}: a bracket type `{other}`, and the file lists only `o` and `c`"
                ));
            }
        };
        if char::from_u32(cp).is_none() || char::from_u32(pair).is_none() {
            return Err(format!("line {n}: a bracket that is not a scalar value"));
        }
        pairs.push((cp, pair, open));
    }
    pairs.sort_unstable();
    if pairs.windows(2).any(|w| w[0].0 == w[1].0) {
        return Err("a code point is listed twice".into());
    }
    if pairs.is_empty() {
        return Err("no brackets".into());
    }
    Ok(pairs)
}

fn render_brackets(header: &Header<'_>, table: &str, pairs: &[Pair]) -> String {
    let mut out = header.render("Unicode's bracket pairs");
    let _ = writeln!(
        out,
        "//! `Bidi_Paired_Bracket` and `Bidi_Paired_Bracket_Type`, from Unicode {}.",
        header.version
    );
    let _ = writeln!(out, "//!");
    let _ = writeln!(
        out,
        "//! Every code point whose type is `Open` or `Close`, ascending, with the bracket"
    );
    let _ = writeln!(
        out,
        "//! it pairs with. A code point not listed has type `None`, which is the upstream"
    );
    let _ = writeln!(
        out,
        "//! file's own statement: it lists only the other two. [`crate::property`] reads it."
    );
    let _ = writeln!(out);
    let _ = writeln!(out, "/// The Unicode version this table was generated from.");
    let _ = writeln!(out, "pub const UNICODE_VERSION: &str = \"{}\";", header.version);
    let _ = writeln!(out);
    let _ = writeln!(out, "/// Which side of a pair a bracket is on: `Bidi_Paired_Bracket_Type`.");
    let _ = writeln!(out, "#[derive(Clone, Copy, Debug, PartialEq, Eq)]");
    let _ = writeln!(out, "pub enum BracketType {{");
    let _ = writeln!(out, "    /// `o` in the upstream file.");
    let _ = writeln!(out, "    Open,");
    let _ = writeln!(out, "    /// `c` in the upstream file.");
    let _ = writeln!(out, "    Close,");
    let _ = writeln!(out, "}}");
    let _ = writeln!(out);
    let _ = writeln!(
        out,
        "/// Each bracket, the bracket it pairs with, and its type, ascending by the first."
    );
    let _ = writeln!(out, "#[rustfmt::skip]");
    let _ = writeln!(out, "pub const {table}: &[(char, char, BracketType)] = &[");
    for (cp, pair, open) in pairs {
        let kind = if *open { "Open" } else { "Close" };
        let _ = writeln!(out, "    ('\\u{{{cp:04X}}}', '\\u{{{pair:04X}}}', BracketType::{kind}),");
    }
    let _ = writeln!(out, "];");
    out
}

// ---------------------------------------------------------------------------
// Provenance
// ---------------------------------------------------------------------------

/// What `lint-licensing` found out about the imports it read, for its ok line.
#[derive(Default)]
pub struct Imports {
    /// Directories under `third_party/`. Unit: none — a count.
    pub trees: usize,
    /// Files whose byte count and SHA-256 were compared. Unit: none — a count.
    pub files: usize,
    /// Their total size. Unit: bytes.
    pub bytes: u64,
    /// Files whose self-stated version was compared. Unit: none — a count.
    pub versions: usize,
    /// Vendored crates held to their record, their lockfile and their own
    /// checksum file. Unit: none — a count.
    pub crates: usize,
}

/// One import's `PROVENANCE.md`, as its tables say it.
///
/// Four table shapes, told apart by their header row and not by position: the
/// `Field | Value` table every import has; a data import's `File | Bytes |
/// SHA-256 | From`; a data import's `Archive | Bytes | SHA-256`, naming what the
/// files were extracted from (`third_party/inter`'s shape, RFC 0141); and a
/// source import's `Crate | Version | Licence | checksum | Why`
/// (`third_party/harfrust`'s). A row under a header this reader does not know is
/// a finding rather than a skip, which is why the two newer imports turned
/// `lint-licensing` red on arrival and why this reader grew rather than a
/// record being bent to fit it.
struct Provenance {
    fields: BTreeMap<String, String>,
    files: BTreeMap<String, (u64, String)>,
    /// `Archive` rows: the name, its size and its SHA-256. The archive is not in
    /// the import — it is where the files came from — so it is checked for
    /// shape and never looked for on disk.
    archives: Vec<(String, u64, String)>,
    /// `Crate` rows, keyed `name-version` as `cargo vendor --versioned-dirs`
    /// names each directory: the crate, its version, its licence and
    /// crates.io's checksum.
    crates: BTreeMap<String, (String, String, String, String)>,
}

/// Is `value` a SHA-256 in lower-case hex, the one spelling this reader takes?
fn is_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// Does `value` carry a git commit — forty lower-case hex digits — as a word?
///
/// `E3-B03b0`'s exit asks the record for *a commit hash*, and a `Commit` field
/// that said `the one tagged 0.13.3` would be a field present and a hash absent.
fn names_a_commit(value: &str) -> bool {
    value.split(|c: char| !c.is_ascii_alphanumeric()).any(|word| {
        word.len() == 40 && word.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

fn cells(line: &str) -> Option<Vec<String>> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|c| c.trim().to_string()).collect())
}

/// Which of [`Provenance`]'s four tables a row is under.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Table {
    Fields,
    Files,
    Archives,
    Crates,
}

/// Read every table. A row this cannot read is a finding rather than a row it
/// skips, because a skipped row is a file with no record that looks recorded.
///
/// A row is read under the header above it, and the header is recognised by its
/// first cell and its width together — `Field` and two columns, `File` and four,
/// `Archive` and three, `Crate` and five — so a row of the wrong width under a
/// known header, and any row under an unknown one, is a finding.
fn parse_provenance(rel: &str, text: &str) -> (Provenance, Vec<String>) {
    let mut findings = Vec::new();
    let mut record = Provenance {
        fields: BTreeMap::new(),
        files: BTreeMap::new(),
        archives: Vec::new(),
        crates: BTreeMap::new(),
    };
    let mut under: Option<Table> = None;
    for (index, line) in text.split('\n').enumerate() {
        let n = index + 1;
        let Some(row) = cells(line) else {
            // A line that is not a table row ends the table: a header two
            // paragraphs up does not govern a row down here.
            if !line.trim().is_empty() {
                under = None;
            }
            continue;
        };
        if row.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')) {
            continue;
        }
        let header = match (row.first().map(String::as_str), row.len()) {
            (Some("Field"), 2) => Some(Table::Fields),
            (Some("File"), 4) => Some(Table::Files),
            (Some("Archive"), 3) => Some(Table::Archives),
            (Some("Crate"), 5) => Some(Table::Crates),
            _ => None,
        };
        if header.is_some() {
            under = header;
            continue;
        }
        let bytes_sha = |bytes: &str, sha: &str, findings: &mut Vec<String>| {
            let sha = sha.trim_matches('`').to_string();
            let Ok(size) = bytes.parse::<u64>() else {
                findings.push(format!("  {rel}:{n}  `{bytes}` is not a byte count"));
                return None;
            };
            if !is_sha256(&sha) {
                findings.push(format!("  {rel}:{n}  `{sha}` is not a SHA-256 in lowercase hex"));
                return None;
            }
            Some((size, sha))
        };
        match (under, row.as_slice()) {
            (Some(Table::Fields), [key, value]) => {
                if record.fields.insert(key.clone(), value.clone()).is_some() {
                    findings.push(format!("  {rel}:{n}  names `{key}` twice"));
                }
            }
            (Some(Table::Files), [file, bytes, sha, _from]) => {
                let path = file.trim_matches('`').to_string();
                let Some(entry) = bytes_sha(bytes, sha, &mut findings) else { continue };
                if record.files.insert(path.clone(), entry).is_some() {
                    findings.push(format!("  {rel}:{n}  records `{path}` twice"));
                }
            }
            (Some(Table::Archives), [name, bytes, sha]) => {
                let name = name.trim_matches('`').to_string();
                let Some((size, sha)) = bytes_sha(bytes, sha, &mut findings) else { continue };
                record.archives.push((name, size, sha));
            }
            (Some(Table::Crates), [name, version, licence, sha, _why]) => {
                let name = name.trim_matches('`').to_string();
                let sha = sha.trim_matches('`').to_string();
                if !is_sha256(&sha) {
                    findings
                        .push(format!("  {rel}:{n}  `{sha}` is not a SHA-256 in lowercase hex"));
                    continue;
                }
                let key = format!("{name}-{version}");
                let row = (name, version.clone(), licence.clone(), sha);
                if record.crates.insert(key.clone(), row).is_some() {
                    findings.push(format!("  {rel}:{n}  records crate `{key}` twice"));
                }
            }
            _ => findings.push(format!("  {rel}:{n}  a table row this reader cannot read")),
        }
    }
    (record, findings)
}

/// Every file under `dir`, relative to it, with `/` separators.
fn files_under(dir: &Path) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(d) = pending.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| format!("reading {}: {e}", d.display()))?;
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            } else if let Ok(rel) = path.strip_prefix(dir) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    Ok(out)
}

/// RFC 0114's third check: every imported tree carries `LICENSE` and a
/// `PROVENANCE.md`, and its record is read against its bytes — a data import's
/// file by file, and a source import's crate by crate and file by file.
///
/// # Why the hash comparison is part of this check and not a fifth
///
/// Because it is what *carries a `PROVENANCE.md`* means for data. RFC 0114's
/// item 3 names the SHA-256 of every file as a field the record needs, on the
/// ground that a data file not byte-identical to upstream is not the
/// specification's corpus any more; a record with the field and no comparison
/// is prose with a hash-shaped string in it. The fifth check this line adds is
/// a different question — whether a committed table is what the generator
/// writes — and it lives in [`table_findings`].
///
/// # A source import, read the same way (RFC 0141)
///
/// `third_party/harfrust` is the first source import, and its record says
/// *Changed: nothing is checkable rather than asserted*. Cargo checks it only
/// while it builds the crates from the vendored source `cargo xtask` names, and
/// only against each crate's own `.cargo-checksum.json`. So this check makes the
/// sentence true on every lint and without a build: [`source_findings`] holds
/// each `vendor/<crate>-<version>/` to a `Crate` row, each row's checksum to the
/// import's own `Cargo.lock` and to the crate's `.cargo-checksum.json`, every
/// file in the crate to the SHA-256 that file records for it — every file
/// listed, and nothing on disk that it does not list — and each crate's own
/// `license` to its row and to [`IMPORT_LICENCES`]. The checksum file itself is
/// anchored outside the import, by `IMPORT_CHECKSUMS` in `main.rs`, because a
/// file edited together with the list that vouches for it is otherwise green.
///
/// # What it does not check
///
/// A commit against its upstream, since nothing here can reach upstream: the
/// `Commit` field must *carry* a forty-digit hash, which is the shape `E3-B03b0`'s
/// exit asks for, and is not verified against anything. An archive a data
/// import's files were extracted from is checked for shape and not for bytes,
/// because it is not in the tree.
///
/// # Errors
///
/// Only a directory that cannot be listed; everything else is a finding.
pub fn import_findings(at: &Path) -> Result<(Vec<String>, Imports), String> {
    let mut findings = Vec::new();
    let mut seen = Imports::default();
    let base = at.join("third_party");
    let Ok(entries) = std::fs::read_dir(&base) else { return Ok((findings, seen)) };
    let mut dirs: Vec<_> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    dirs.retain(|p| p.is_dir());
    dirs.sort();
    for dir in dirs {
        seen.trees += 1;
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let rel = format!("third_party/{name}");
        let licence = std::fs::metadata(dir.join("LICENSE")).map(|m| m.len()).unwrap_or(0);
        if licence == 0 {
            findings.push(format!(
                "  {rel}/  carries no LICENSE — the terms this import arrived under"
            ));
        }
        let Ok(text) = std::fs::read_to_string(dir.join("PROVENANCE.md")) else {
            findings.push(format!(
                "  {rel}/  carries no PROVENANCE.md — upstream, version or commit, the date \
                 imported, and for data every file with its SHA-256"
            ));
            continue;
        };
        let record = format!("{rel}/PROVENANCE.md");
        let (provenance, unreadable) = parse_provenance(&record, &text);
        findings.extend(unreadable);
        let kind = provenance.fields.get("Kind").map(String::as_str);
        let mut required = vec!["Kind", "Upstream", "Imported", "Changed"];
        match kind {
            Some("data") => required.push("Version"),
            Some("source") => required.push("Commit"),
            _ => findings.push(format!("  {record}  `Kind` must be `data` or `source`")),
        }
        for key in required {
            if provenance.fields.get(key).is_none_or(String::is_empty) {
                findings.push(format!("  {record}  records no `{key}`"));
            }
        }
        if let Some(commit) = provenance.fields.get("Commit")
            && !names_a_commit(commit)
        {
            findings.push(format!(
                "  {record}  `Commit` carries no forty-digit commit hash: `{commit}`"
            ));
        }
        if let Some(date) = provenance.fields.get("Imported") {
            let shape = date.len() == 10
                && date
                    .bytes()
                    .enumerate()
                    .all(|(i, b)| if i == 4 || i == 7 { b == b'-' } else { b.is_ascii_digit() });
            if !shape {
                findings.push(format!("  {record}  `Imported` is `{date}`, not YYYY-MM-DD"));
            }
        }
        match kind {
            Some("data") => data_findings(&dir, &rel, &provenance, &mut findings, &mut seen)?,
            Some("source") => source_findings(&dir, &rel, &provenance, &mut findings, &mut seen)?,
            _ => {}
        }
    }
    Ok((findings, seen))
}

/// A data import's files against its record: every file a row, every row a
/// file, each the size and SHA-256 recorded, each a kind of file the category
/// admits, and each self-stated version the record's.
fn data_findings(
    dir: &Path,
    rel: &str,
    provenance: &Provenance,
    findings: &mut Vec<String>,
    seen: &mut Imports,
) -> Result<(), String> {
    let record = format!("{rel}/PROVENANCE.md");
    if !provenance.crates.is_empty() {
        findings.push(format!("  {record}  is a data import and records crates"));
    }
    let version = provenance.fields.get("Version").cloned().unwrap_or_default();
    let on_disk = files_under(dir)?;
    for file in &on_disk {
        if file == "PROVENANCE.md" {
            continue;
        }
        let bytes =
            std::fs::read(dir.join(file)).map_err(|e| format!("reading {rel}/{file}: {e}"))?;
        let admitted = DATA_NAMES.contains(&file.as_str())
            || file.rsplit_once('.').is_some_and(|(_, ext)| DATA_EXTENSIONS.contains(&ext))
            || is_admitted_font(file, &bytes);
        if !admitted {
            findings.push(format!(
                "  {rel}/{file}  is in a data import and is not a data file: the category \
                 holds files no compiler in this workspace reads (RFC 0114), held by \
                 extension — {} — rather than by a list of languages, and one font rule: \
                 `.{}` beginning with the TrueType version {:02x?} (RFC 0141)",
                DATA_EXTENSIONS.join(", "),
                DATA_FONT.0,
                DATA_FONT.1
            ));
        }
        let Some((size, sha)) = provenance.files.get(file) else {
            findings.push(format!("  {rel}/{file}  has no row in PROVENANCE.md"));
            continue;
        };
        seen.files += 1;
        seen.bytes += bytes.len() as u64;
        if bytes.len() as u64 != *size {
            findings.push(format!(
                "  {rel}/{file}  is {} byte(s) and PROVENANCE.md records {size}",
                bytes.len()
            ));
        }
        let actual = sha256_hex(&bytes);
        if actual != *sha {
            findings.push(format!(
                "  {rel}/{file}  hashes to {actual} and PROVENANCE.md records {sha}"
            ));
        }
        if let Some(stated) = std::str::from_utf8(&bytes).ok().and_then(self_stated_version) {
            seen.versions += 1;
            if stated != version {
                findings.push(format!(
                    "  {rel}/{file}  says it is version {stated} and PROVENANCE.md records \
                     {version}"
                ));
            }
        }
    }
    for file in provenance.files.keys() {
        if !on_disk.contains(file) {
            findings.push(format!("  {record}  records `{file}`, which is not in the import"));
        }
    }
    for (archive, _, _) in &provenance.archives {
        if on_disk.contains(archive) {
            findings.push(format!(
                "  {rel}/{archive}  is recorded as the archive the files came from, and an \
                 archive is not kept in the import"
            ));
        }
    }
    // The archive the record says the files came from has a row, and a row
    // names an archive the record says they came from. For a round the table
    // was optional: deleting it left the record as green as it was, so the one
    // measurement of what was fetched could go without a finding (RFC 0141).
    let retrieved = provenance.fields.get("Retrieved with").map_or("", String::as_str);
    let named = archives_named(retrieved);
    for archive in &named {
        if !provenance.archives.iter().any(|(row, _, _)| row == archive) {
            findings.push(format!(
                "  {record}  `Retrieved with` names the archive `{archive}` and no `Archive` row \
                 records its size and SHA-256"
            ));
        }
    }
    for (row, _, _) in &provenance.archives {
        if !named.contains(row) {
            findings.push(format!(
                "  {record}  records the archive `{row}`, which `Retrieved with` does not name"
            ));
        }
    }
    Ok(())
}

/// What a file name ends in when it is an archive a data import's files could
/// have been extracted from.
const ARCHIVE_EXTENSIONS: &[&str] =
    &[".zip", ".tar", ".tar.gz", ".tgz", ".tar.xz", ".txz", ".tar.bz2", ".tbz2", ".tar.zst", ".7z"];

/// Every archive a `Retrieved with` field names, by its file name: the last
/// path segment of any word ending in one of [`ARCHIVE_EXTENSIONS`].
fn archives_named(retrieved: &str) -> Vec<String> {
    let mut out: Vec<String> = retrieved
        .split(|c: char| c.is_whitespace() || matches!(c, '`' | ',' | '"' | '\'' | '(' | ')'))
        .filter_map(|word| word.rsplit('/').next())
        .map(|word| word.trim_end_matches(['.', ';', ':']))
        .filter(|word| {
            ARCHIVE_EXTENSIONS.iter().any(|ext| word.len() > ext.len() && word.ends_with(ext))
        })
        .map(str::to_string)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// The files a source import holds beside `vendor/`: its licence, its record,
/// and the lockfile that is its resolution.
const SOURCE_TOP: &[&str] = &["LICENSE", "PROVENANCE.md", "Cargo.lock"];

/// Where `cargo vendor --versioned-dirs` put the crates.
const VENDOR: &str = "vendor";

/// The file `cargo vendor` writes into every crate it vendors.
const CARGO_CHECKSUM: &str = ".cargo-checksum.json";

/// A source import's crates against its record, its lockfile and their own
/// checksum files. See [`import_findings`] for why.
///
/// The one shape of source import this tree has is a `cargo vendor` set, and
/// this reads that shape and refuses anything else under the import's top
/// level. *Reversal:* a source import that is not Rust — a C driver, RFC 0003's
/// first expectation — which needs a record of its own shape and a reader for
/// it, not this one loosened.
fn source_findings(
    dir: &Path,
    rel: &str,
    provenance: &Provenance,
    findings: &mut Vec<String>,
    seen: &mut Imports,
) -> Result<(), String> {
    let record = format!("{rel}/PROVENANCE.md");
    if !provenance.files.is_empty() || !provenance.archives.is_empty() {
        findings.push(format!("  {record}  is a source import and records data files"));
    }
    if provenance.crates.is_empty() {
        findings.push(format!(
            "  {record}  records no crates — a source import names every crate it vendors, \
             with its version and checksum"
        ));
    }
    let on_disk = files_under(dir)?;
    for file in &on_disk {
        if !SOURCE_TOP.contains(&file.as_str()) && !file.starts_with(&format!("{VENDOR}/")) {
            findings.push(format!(
                "  {rel}/{file}  is in a source import outside `{VENDOR}/` and is not one of {}",
                SOURCE_TOP.join(", ")
            ));
        }
    }
    let locked = std::fs::read_to_string(dir.join("Cargo.lock"))
        .map(|text| lock_checksums(&text))
        .unwrap_or_default();
    if locked.is_empty() {
        findings.push(format!("  {rel}/Cargo.lock  is missing or pins no checksum"));
    }

    let vendored: BTreeSet<String> = on_disk
        .iter()
        .filter_map(|file| file.strip_prefix(&format!("{VENDOR}/")))
        .filter_map(|file| file.split_once('/').map(|(krate, _)| krate.to_string()))
        .collect();
    for krate in &vendored {
        let Some((_, _, licence, sha)) = provenance.crates.get(krate) else {
            findings
                .push(format!("  {rel}/{VENDOR}/{krate}/  has no `Crate` row in PROVENANCE.md"));
            continue;
        };
        match locked.get(krate) {
            Some(lock) if lock == sha => {}
            Some(lock) => findings
                .push(format!("  {record}  records {krate} as {sha} and Cargo.lock pins {lock}")),
            None => findings.push(format!("  {rel}/Cargo.lock  pins no {krate}")),
        }
        let crate_dir = dir.join(VENDOR).join(krate);
        let Ok(json) = std::fs::read_to_string(crate_dir.join(CARGO_CHECKSUM)) else {
            findings.push(format!("  {rel}/{VENDOR}/{krate}/  carries no {CARGO_CHECKSUM}"));
            continue;
        };
        let Some((listed, package)) = cargo_checksum(&json) else {
            findings.push(format!(
                "  {rel}/{VENDOR}/{krate}/{CARGO_CHECKSUM}  is not the shape cargo writes"
            ));
            continue;
        };
        if package != *sha {
            findings.push(format!(
                "  {rel}/{VENDOR}/{krate}/{CARGO_CHECKSUM}  names package {package} and \
                 PROVENANCE.md records {sha}"
            ));
        }
        let manifest = std::fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap_or_default();
        findings.extend(licence_findings(&format!("{rel}/{VENDOR}/{krate}"), &manifest, licence));
        let files = files_under(&crate_dir)?;
        for file in &files {
            if file == CARGO_CHECKSUM {
                continue;
            }
            let Some(want) = listed.get(file) else {
                findings.push(format!(
                    "  {rel}/{VENDOR}/{krate}/{file}  is not listed in {CARGO_CHECKSUM}, so it is \
                     not what upstream published"
                ));
                continue;
            };
            let bytes = std::fs::read(crate_dir.join(file))
                .map_err(|e| format!("reading {rel}/{VENDOR}/{krate}/{file}: {e}"))?;
            seen.files += 1;
            seen.bytes += bytes.len() as u64;
            let actual = sha256_hex(&bytes);
            if actual != *want {
                findings.push(format!(
                    "  {rel}/{VENDOR}/{krate}/{file}  hashes to {actual} and {CARGO_CHECKSUM} \
                     records {want}"
                ));
            }
        }
        for file in listed.keys() {
            if !files.contains(file) {
                findings.push(format!(
                    "  {rel}/{VENDOR}/{krate}/{CARGO_CHECKSUM}  lists `{file}`, which is not \
                     in the crate"
                ));
            }
        }
    }
    for krate in provenance.crates.keys() {
        if !vendored.contains(krate) {
            findings
                .push(format!("  {record}  records crate `{krate}`, which is not in `{VENDOR}/`"));
        }
    }
    seen.crates += vendored.len();
    Ok(())
}

/// Every `[[package]]` in a lockfile that carries a checksum, keyed
/// `name-version` as `cargo vendor --versioned-dirs` names its directories.
pub(crate) fn lock_checksums(text: &str) -> BTreeMap<String, String> {
    lock_packages(text)
        .into_iter()
        .filter_map(|p| p.checksum.map(|sum| (format!("{}-{}", p.name, p.version), sum)))
        .collect()
}

/// One `[[package]]` of a lockfile, as cargo wrote it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct LockPackage {
    /// Its name.
    pub name: String,
    /// Its version.
    pub version: String,
    /// Where it resolved from — `registry+…`, `git+…` — and none for a path.
    pub source: Option<String>,
    /// The registry's SHA-256 of the package, for a registry source.
    pub checksum: Option<String>,
    /// The package a `[replace]` table put in its place, if one did.
    pub replace: Option<String>,
}

/// Every `[[package]]` in a lockfile.
///
/// Line by line, because a lockfile is cargo's own output in one shape: a
/// `[[package]]` header, then `name`, `version`, `source`, `checksum` and
/// `replace` rows, then a `dependencies` array this does not need. A row this
/// does not know is skipped: the callers judge what is here, and a row cargo
/// adds later is not one they depend on.
pub(crate) fn lock_packages(text: &str) -> Vec<LockPackage> {
    let mut out: Vec<LockPackage> = Vec::new();
    let value = |line: &str, key: &str| {
        line.strip_prefix(key)
            .and_then(|rest| rest.trim_start().strip_prefix('='))
            .map(|rest| rest.trim().trim_matches('"').to_string())
    };
    for line in text.lines() {
        if line.trim() == "[[package]]" {
            out.push(LockPackage::default());
            continue;
        }
        let Some(package) = out.last_mut() else { continue };
        if let Some(v) = value(line, "name") {
            package.name = v;
        } else if let Some(v) = value(line, "version") {
            package.version = v;
        } else if let Some(v) = value(line, "source") {
            package.source = Some(v);
        } else if let Some(v) = value(line, "checksum") {
            package.checksum = Some(v);
        } else if let Some(v) = value(line, "replace") {
            package.replace = Some(v);
        }
    }
    out
}

/// The licences a crate of a source import may be under: `deny.toml`'s
/// `[licenses] allow`, which is what the permissive tree's own dependencies are
/// held to, and `Zlib`.
///
/// Stated here because `cargo deny` runs over the workspace and the shim is
/// outside it (RFC 0141), so until this list existed nothing read the licence
/// of any crate linked into the shaper's image — PROVENANCE.md's column was
/// parsed and dropped, and a crate re-licensed GPL-3.0 in both its manifest and
/// its checksum file left every lint green. `Zlib` is the one addition, and it
/// is `bytemuck`'s: permissive, no copyleft, no patent clause to weigh, and in
/// `LICENSING.md`'s sentence of what the shaper's image carries. It is not in
/// `deny.toml` because nothing in the workspace needs it, and adding it there
/// would widen the workspace for an import's sake. The test
/// `the_import_licences_are_deny_toml_s_and_zlib` holds the two lists together,
/// so a licence admitted to one and not the other is red.
///
/// *What would reverse this:* an import whose crate needs a licence outside the
/// list — which is a licence argued in an RFC before it is a row here.
pub(crate) const IMPORT_LICENCES: &[&str] =
    &["Apache-2.0", "MIT", "BSD-2-Clause", "BSD-3-Clause", "ISC", "Unicode-3.0", "Zlib"];

/// The `license` a crate's own `Cargo.toml` states in `[package]`.
///
/// `cargo vendor` writes the normalised manifest crates.io serves, so the row
/// is `license = "…"` on one line; `license-file` is not a licence this reads,
/// and a crate that has only that is a finding.
fn crate_licence(manifest: &str) -> Option<String> {
    let mut section = String::new();
    for line in manifest.lines() {
        let code = line.trim();
        if let Some(head) = code.strip_prefix('[') {
            section = head.trim_end_matches(']').trim().to_string();
            continue;
        }
        if section != "package" {
            continue;
        }
        let Some(rest) = code.strip_prefix("license") else { continue };
        let Some(value) = rest.trim_start().strip_prefix('=') else { continue };
        return Some(value.trim().trim_matches('"').to_string());
    }
    None
}

/// Every licence an SPDX expression names — the operators, the parentheses and
/// the old `/` spelling of `OR` taken out.
///
/// **Every one, and not a choice among them.** `cargo deny` accepts an
/// expression it can satisfy; this refuses any expression that *names* a
/// licence outside [`IMPORT_LICENCES`], because the record carries one column
/// per crate and a reader of it should not have to know which branch somebody
/// chose. Stricter, and the direction that costs a row rather than a surprise.
fn licence_ids(expression: &str) -> Vec<String> {
    expression
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '/'))
        .filter(|word| !word.is_empty() && !matches!(*word, "AND" | "OR" | "WITH"))
        .map(str::to_string)
        .collect()
}

/// A vendored crate's own `license` against its record's `Licence` column and
/// against [`IMPORT_LICENCES`]. `at` names the crate in the finding.
fn licence_findings(at: &str, manifest: &str, recorded: &str) -> Vec<String> {
    let Some(own) = crate_licence(manifest) else {
        return vec![format!(
            "  {at}/Cargo.toml  states no `license`, so nothing says what the shaper's image \
             carries from it"
        )];
    };
    let mut findings = Vec::new();
    if own != recorded {
        findings.push(format!(
            "  {at}/Cargo.toml  is `{own}` and its `Crate` row in PROVENANCE.md records \
             `{recorded}`"
        ));
    }
    let mut named = licence_ids(&own);
    named.extend(licence_ids(recorded));
    named.sort();
    named.dedup();
    for id in named {
        if !IMPORT_LICENCES.contains(&id.as_str()) {
            findings.push(format!(
                "  {at}  names `{id}`, which is not a licence an import's crate may carry: {} \
                 (deny.toml's list and Zlib)",
                IMPORT_LICENCES.join(", ")
            ));
        }
    }
    findings
}

/// Every vendored crate's directory, relative to `at`, for every source import:
/// `third_party/<import>/vendor/<crate>-<version>`.
pub fn vendored_crate_dirs(at: &Path) -> Vec<String> {
    let mut out = Vec::new();
    for import in source_crates(at).keys() {
        let Ok(entries) = std::fs::read_dir(at.join(import).join(VENDOR)) else { continue };
        let mut dirs: Vec<String> = entries
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .collect();
        dirs.sort();
        out.extend(dirs.into_iter().map(|name| format!("{import}/{VENDOR}/{name}")));
    }
    out
}

/// A `.cargo-checksum.json`: every file and its SHA-256, and the package's.
///
/// Not a JSON parser: the file is cargo's output in one shape — string keys and
/// string values, an object `files` of them and a string `package` — so this
/// reads the strings in order, decoding the escapes JSON allows in them, and
/// takes the pairs inside `files` and the value after `package`. A shape it does
/// not recognise is `None`, which the caller reports, rather than a partial
/// reading it would then believe.
fn cargo_checksum(json: &str) -> Option<(BTreeMap<String, String>, String)> {
    // Tokens: a string, or one of the structural characters.
    let mut tokens: Vec<Result<String, char>> = Vec::new();
    let mut chars = json.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                loop {
                    match chars.next()? {
                        '"' => break,
                        '\\' => match chars.next()? {
                            'u' => {
                                let hex: String = (0..4).filter_map(|_| chars.next()).collect();
                                s.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
                            }
                            'n' => s.push('\n'),
                            't' => s.push('\t'),
                            other => s.push(other),
                        },
                        other => s.push(other),
                    }
                }
                tokens.push(Ok(s));
            }
            '{' | '}' | ':' | ',' => tokens.push(Err(c)),
            c if c.is_whitespace() => {}
            _ => return None,
        }
    }
    let mut files = BTreeMap::new();
    let mut package = None;
    let mut depth = 0usize;
    let mut in_files = false;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            Err('{') => depth += 1,
            Err('}') => {
                depth = depth.checked_sub(1)?;
                in_files = false;
            }
            Ok(key) if tokens.get(i + 1) == Some(&Err(':')) => {
                match tokens.get(i + 2) {
                    Some(Err('{')) if depth == 1 && key == "files" => in_files = true,
                    Some(Ok(value)) if in_files && depth == 2 => {
                        files.insert(key.clone(), value.clone());
                        i += 3;
                        continue;
                    }
                    Some(Ok(value)) if depth == 1 && key == "package" => {
                        package = Some(value.clone());
                        i += 3;
                        continue;
                    }
                    Some(Ok(_)) if depth == 1 => {
                        i += 3;
                        continue;
                    }
                    _ => {}
                }
                i += 2;
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    (!files.is_empty()).then_some(())?;
    Some((files, package?))
}

/// Every vendored crate of every source import that executes on the build
/// machine when the import is built — a procedural macro, or a crate with a
/// build script — as `(third_party/<import>/vendor/<crate>, what it is)`.
///
/// Read from each crate's own `Cargo.toml`, which `cargo vendor` normalises: a
/// build script is a `build = "…"` row, or a `build.rs` with no `build = false`
/// to say it is not one; a procedural macro is `proc-macro = true`. This is
/// what `IMPORT_HOST_CODE` in `main.rs` is compared against in both directions,
/// so a re-import that brings a new one is red until somebody has read it.
/// RFC 0141.
pub fn host_code(at: &Path) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for rel in vendored_crate_dirs(at) {
        let dir = at.join(&rel);
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml")).unwrap_or_default();
        let rows: Vec<String> = manifest
            .lines()
            .map(|line| line.split('#').next().unwrap_or("").replace(' ', ""))
            .collect();
        if rows.iter().any(|row| row == "proc-macro=true" || row == "proc_macro=true") {
            out.push((rel.clone(), "a procedural macro"));
        }
        let declared = rows.iter().find_map(|row| row.strip_prefix("build="));
        let builds = match declared {
            Some("false") => false,
            Some(_) => true,
            None => dir.join("build.rs").is_file(),
        };
        if builds {
            out.push((rel, "a build script"));
        }
    }
    out
}

/// Every source import and the crates its record lists, as
/// `(third_party/<name>, [(crate, version)])`.
///
/// What `IMPORT_LINKERS` is checked against: the one manifest allowed to take a
/// crate of an import by name, and every other manifest that may take none.
pub fn source_crates(at: &Path) -> BTreeMap<String, Vec<(String, String)>> {
    let mut out = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(at.join("third_party")) else { return out };
    for dir in entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()) {
        let Ok(text) = std::fs::read_to_string(dir.join("PROVENANCE.md")) else { continue };
        let (record, _) = parse_provenance("", &text);
        if record.fields.get("Kind").map(String::as_str) != Some("source") {
            continue;
        }
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let crates = record
            .crates
            .values()
            .map(|(name, version, ..)| (name.clone(), version.clone()))
            .collect();
        out.insert(format!("third_party/{name}"), crates);
    }
    out
}

/// The names of every file a **data** import holds, other than its licence and
/// its record: the second needle `IMPORT_READERS` reads for, because a path can
/// be assembled without spelling the import's directory and still spell its
/// file.
///
/// Data imports only, and the restriction is RFC 0141's. A source import's file
/// names are `lib.rs`, `mod.rs`, `Cargo.toml` and `README.md` — words every
/// crate in the permissive tree says — so taken as needles they found three
/// hundred readers on the day HarfRust arrived, every one of them false. What
/// guards a source import is the other needle, the import's directory, and the
/// fact that nothing opens source at run time: source is linked (by the one
/// `IMPORT_LINKERS` row) or it is not reached at all.
pub fn data_file_names(at: &Path) -> Vec<String> {
    let mut names = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(at.join("third_party")) else { return Vec::new() };
    for dir in entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()) {
        let kind = std::fs::read_to_string(dir.join("PROVENANCE.md"))
            .ok()
            .and_then(|text| parse_provenance("", &text).0.fields.get("Kind").cloned());
        // A tree whose record does not say `source` is read as data: an import
        // with no record is refused by the check above, and until it is fixed
        // its file names are needles rather than a silence.
        if kind.as_deref() == Some("source") {
            continue;
        }
        for file in files_under(&dir).unwrap_or_default() {
            let base = file.rsplit('/').next().unwrap_or(&file).to_string();
            if !DATA_NAMES.contains(&base.as_str()) {
                names.insert(base);
            }
        }
    }
    names.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use super::{
        IMPORT_DIR, generate, import_findings, parse_brackets, resolve_binary, resolve_field,
        resolve_ranges, self_stated_version, sha256_hex, stated_version, table_findings,
    };
    use std::path::{Path, PathBuf};

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("xtask has a parent").to_path_buf()
    }

    fn fixture(name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let at = crate::target_dir().join(crate::FIXTURE_DIR).join(name);
        let _ = std::fs::remove_dir_all(&at);
        for (rel, bytes) in files {
            let path = at.join(rel);
            std::fs::create_dir_all(path.parent().expect("a parent")).expect("fixture dir");
            std::fs::write(&path, bytes).expect("fixture file");
        }
        at
    }

    /// A three-value property with a heading, a total and two defaults, small
    /// enough to reason about by hand.
    const SMALL: &str = "# Small-1.2.3.txt\n\
        # @missing: 0000..10FFFF; Left\n\
        # @missing: 0100..01FF; Right\n\
        \n\
        # P=Left\n\
        0041..005A ; L # letters\n\
        # Total code points: 1113855\n\
        \n\
        # P=Right\n\
        0030 ; R\n\
        # Total code points: 257\n";

    #[test]
    fn defaults_come_from_the_file_and_its_totals_hold_them() {
        let r = resolve_ranges(SMALL, "P").expect("a readable file");
        assert_eq!(r.values, vec!["L".to_string(), "R".to_string()]);
        assert_eq!(r.totals, 2);
        assert_eq!(r.runs, vec![(0, 0), (0x30, 1), (0x31, 0), (0x100, 1), (0x200, 0)]);
    }

    /// The totals are what catch a wrong reading of the defaults. Swapping the
    /// order the two `@missing` lines are applied in would give `Right` 0 extra
    /// code points instead of 256, and this is that, from the file's side.
    #[test]
    fn a_total_the_reading_does_not_reproduce_is_refused() {
        let wrong = SMALL.replace("Total code points: 257", "Total code points: 1");
        let e = resolve_ranges(&wrong, "P").err().expect("refused");
        assert!(e.contains("covers 1 code point(s) and this reading of it gives 257"), "{e}");
    }

    #[test]
    fn overlapping_defaults_are_refused_rather_than_ordered() {
        let overlap = SMALL.replace(
            "# @missing: 0100..01FF; Right\n",
            "# @missing: 0100..01FF; Right\n# @missing: 01F0..02FF; Left\n",
        );
        let e = resolve_ranges(&overlap, "P").err().expect("refused");
        assert!(e.contains("overlap"), "{e}");
    }

    #[test]
    fn a_file_with_no_universal_default_is_refused() {
        let none = SMALL.replace("# @missing: 0000..10FFFF; Left\n", "");
        let e = resolve_ranges(&none, "P").err().expect("refused");
        assert!(e.contains("does not cover the code space"), "{e}");
    }

    #[test]
    fn a_code_point_listed_twice_is_refused() {
        let twice = SMALL.replace("0030 ; R\n", "0030 ; R\n0030 ; R\n");
        assert!(resolve_ranges(&twice, "P").err().expect("refused").contains("listed twice"));
    }

    /// RFC 0139's first reading: a file with no `@missing` line is read only
    /// because it lists every code point itself, and one it leaves out is a
    /// refusal rather than a default this tree would choose.
    #[test]
    fn a_file_with_no_default_must_list_every_code_point() {
        let every = "# All-1.0.0.txt\n# P=Few\n0000..0040 ; F\n# Total code points: 65\n\
                     # P=Many\n0041..10FFFF ; M\n# Total code points: 1114047\n";
        let r = resolve_ranges(every, "P").expect("every code point listed");
        assert_eq!(r.runs, vec![(0, 0), (0x41, 1)]);
        assert_eq!(r.totals, 2);
        let gap = every.replace("0041..10FFFF ; M\n", "0042..10FFFF ; M\n");
        let e = resolve_ranges(&gap, "P").err().expect("refused");
        assert!(e.contains("no `@missing` line covers U+0041"), "{e}");
    }

    /// A total no heading names still counts when every line since the last
    /// total carried one value, which is how `GraphemeBreakProperty.txt`
    /// states its totals; after lines of two values it is attributed to
    /// neither.
    #[test]
    fn an_unheaded_total_is_held_to_its_one_value() {
        let unheaded = "# U-1.0.0.txt\n# @missing: 0000..10FFFF; Other\n\
                        0041..0042 ; A\n# Total code points: 2\n0030 ; B\n0031 ; C\n\
                        # Total code points: 99\n";
        let r = resolve_ranges(unheaded, "P").expect("the mixed total is not attributed");
        assert_eq!(r.totals, 1);
        let wrong = unheaded.replace("# Total code points: 2\n", "# Total code points: 3\n");
        let e = resolve_ranges(&wrong, "P").err().expect("refused");
        assert!(e.contains("covers 3 code point(s) and this reading of it gives 2"), "{e}");
    }

    /// One property of several: only the lines whose middle field names it,
    /// its own `@missing` line included; a binary property's two-field lines
    /// and another enumerated property's lines are skipped.
    #[test]
    fn one_property_of_several_reads_only_its_own_lines() {
        let mixed = "# D-1.0.0.txt\n# @missing: 0000..10FFFF; X; Nope\n\
                     # @missing: 0000..10FFFF; InCB; None\n0041 ; Alpha\n\
                     # Indic_Conjunct_Break=Linker\n0042 ; InCB; Linker\n\
                     # Total code points: 1\n0043 ; X; Yes\n";
        let r = resolve_field(mixed, "Indic_Conjunct_Break", Some("InCB")).expect("read");
        assert_eq!(r.values, vec!["Linker".to_string(), "None".to_string()]);
        assert_eq!(r.runs, vec![(0, 1), (0x42, 0), (0x43, 1)]);
        assert_eq!(r.totals, 1);
    }

    /// RFC 0139's second reading: a binary property's default is the file's
    /// sentence for it, and its `# Total elements:` must be reproduced.
    #[test]
    fn a_binary_property_needs_its_sentence_and_its_total() {
        let good = "# e.txt\n# All omitted code points have Other=No\n0030 ; Other\n\
                    # Total elements: 1\n# All omitted code points have Pict=No\n\
                    00A9 ; Pict\n2000..2001 ; Pict\n# Total elements: 3\n";
        let r = resolve_binary(good, "Pict").expect("read");
        assert_eq!(
            r.runs,
            vec![(0, false), (0xA9, true), (0xAA, false), (0x2000, true), (0x2002, false)]
        );
        assert_eq!(r.count, 3);
        let silent = good.replace("# All omitted code points have Pict=No\n", "");
        let e = resolve_binary(&silent, "Pict").err().expect("refused");
        assert!(e.contains("appears 0 time(s)"), "{e}");
        let short = good.replace("# Total elements: 3", "# Total elements: 2");
        let e = resolve_binary(&short, "Pict").err().expect("refused");
        assert!(e.contains("covers 2 element(s) and this reading of it gives 3"), "{e}");
        let resumed = format!("{good}0300 ; Pict\n");
        let e = resolve_binary(&resumed, "Pict").err().expect("refused");
        assert!(e.contains("resumes after its stated total"), "{e}");
    }

    /// RFC 0139's third reading: `emoji-data.txt` states `Version: 17.0`, and
    /// the third component is the record's only if the record is a release of
    /// what the file said.
    #[test]
    fn a_short_version_is_completed_only_by_a_record_that_agrees() {
        let record = |version: &str| {
            format!(
                "| Field | Value |\n|---|---|\n| Kind | data |\n| Version | {version} |\n\n\
                 | File | Bytes | SHA-256 | From |\n|---|---|---|---|\n"
            )
        };
        let emoji = "# emoji-data.txt\n# Version: 17.0\n#\n0023 ; Emoji\n";
        let agrees = record("17.0.0");
        let at =
            fixture("import-version", &[("third_party/unicode/PROVENANCE.md", agrees.as_bytes())]);
        assert_eq!(stated_version(&at, emoji), Ok("17.0.0".to_string()));
        assert_eq!(stated_version(&at, "# X-16.0.0.txt\n"), Ok("16.0.0".to_string()));
        let differs = record("17.01.0");
        let at =
            fixture("import-version", &[("third_party/unicode/PROVENANCE.md", differs.as_bytes())]);
        let e = stated_version(&at, emoji).expect_err("refused");
        assert!(e.contains("which is not a release of it"), "{e}");
        let e = stated_version(&at, "# nothing.txt\n0023 ; Emoji\n").expect_err("refused");
        assert!(e.contains("states no version"), "{e}");
    }

    #[test]
    fn a_bracket_type_the_file_does_not_define_is_refused() {
        assert!(parse_brackets("0028; 0029; o\n0029; 0028; c\n").is_ok());
        let e = parse_brackets("0028; 0029; n\n").expect_err("refused");
        assert!(e.contains("lists only `o` and `c`"), "{e}");
    }

    #[test]
    fn the_version_is_read_from_the_first_line_and_nowhere_else() {
        assert_eq!(self_stated_version("# BidiTest-17.0.0.txt\n"), Some("17.0.0"));
        assert_eq!(self_stated_version("# emoji-data.txt\n# Version: 17.0\n"), None);
        assert_eq!(self_stated_version("\n# BidiTest-17.0.0.txt\n"), None);
    }

    /// The input is the one DERIVED_DATA names, or nothing is generated.
    #[test]
    fn an_input_that_is_not_the_recorded_one_generates_nothing() {
        let at = fixture(
            "import-input",
            &[("third_party/unicode/BidiBrackets.txt", b"# B-1.0.0.txt\n0028; 0029; o\n")],
        );
        let upstream = format!("{IMPORT_DIR}/BidiBrackets.txt");
        let wrong = "0".repeat(64);
        let e = generate(&at, "text/src/bidi_brackets.rs", &upstream, &wrong).expect_err("refused");
        assert!(e.contains("is not the file DERIVED_DATA names"), "{e}");
        let right = sha256_hex(b"# B-1.0.0.txt\n0028; 0029; o\n");
        assert!(generate(&at, "text/src/bidi_brackets.rs", &upstream, &right).is_ok());
        let e = generate(&at, "text/src/nothing.rs", &upstream, &right).expect_err("refused");
        assert!(e.contains("no shape in SHAPES"), "{e}");
    }

    /// The fifth check, red: a committed table one byte away from the
    /// generator's output, and a shape nobody gave a row.
    #[test]
    fn a_table_that_is_not_what_the_generator_writes_is_found() {
        let input: &[u8] = b"# B-1.0.0.txt\n0028; 0029; o\n0029; 0028; c\n";
        let at = fixture("import-drift", &[("third_party/unicode/BidiBrackets.txt", input)]);
        let upstream = format!("{IMPORT_DIR}/BidiBrackets.txt");
        let sha = sha256_hex(input);
        let good = generate(&at, "text/src/bidi_brackets.rs", &upstream, &sha).expect("generated");
        let out = at.join("text/src/bidi_brackets.rs");
        std::fs::create_dir_all(out.parent().expect("parent")).expect("dir");
        std::fs::write(&out, &good).expect("write");
        let rows = [("text/src/bidi_brackets.rs", upstream.as_str(), sha.as_str())];
        let (findings, compared) = table_findings(&at, &rows);
        assert_eq!(compared, 1);
        assert_eq!(
            findings.len(),
            super::SHAPES.len() - 1,
            "only the other shapes' missing rows: {findings:?}"
        );
        assert!(findings.iter().all(|f| f.contains("has a shape")), "{findings:?}");
        assert!(findings[0].contains("text/src/bidi_class.rs  has a shape"), "{findings:?}");
        std::fs::write(&out, good.replace("Close", "Open")).expect("write");
        let (findings, _) = table_findings(&at, &rows);
        assert!(
            findings.iter().any(|f| f.contains("bidi_brackets.rs:") && f.contains("is not what")),
            "{findings:?}"
        );
    }

    /// Byte-identical on regeneration, over the real import: two runs, one text.
    #[test]
    fn the_generator_writes_the_same_bytes_twice() {
        for (output, upstream, sha) in crate::DERIVED_DATA {
            let a = generate(&root(), output, upstream, sha).expect("generated");
            let b = generate(&root(), output, upstream, sha).expect("generated");
            assert_eq!(a, b, "{output}");
            assert!(!a.contains('\r'), "{output} would carry a carriage return");
        }
    }

    /// `E3-B03f`'s tables are a row and not a rewrite: the three of its
    /// properties whose files hold nothing else read through the parser the
    /// bidi class does, cover the code space, and reproduce every total their
    /// files state. The other three are the shapes RFC 0139 added, held above.
    #[test]
    fn every_break_property_this_import_holds_is_a_row() {
        for (file, property) in [
            ("LineBreak.txt", "Line_Break"),
            ("auxiliary/GraphemeBreakProperty.txt", "Grapheme_Cluster_Break"),
            ("EastAsianWidth.txt", "East_Asian_Width"),
        ] {
            let text = std::fs::read_to_string(root().join(IMPORT_DIR).join(file)).expect(file);
            let r = resolve_ranges(&text, property).unwrap_or_else(|e| panic!("{file}: {e}"));
            assert_eq!(r.runs.first().map(|(s, _)| *s), Some(0), "{file}");
            assert!(r.values.len() > 1, "{file}");
            for value in &r.values {
                super::variant(value).unwrap_or_else(|e| panic!("{file}: {e}"));
            }
        }
    }

    /// Check 3, red: a directory under `third_party/` with neither file.
    #[test]
    fn an_import_with_neither_file_is_refused() {
        let at = fixture("import-neither", &[("third_party/bare/data.txt", b"x\n")]);
        let (findings, seen) = import_findings(&at).expect("listed");
        assert_eq!(seen.trees, 1);
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(findings[0].contains("carries no LICENSE"), "{findings:?}");
        assert!(findings[1].contains("carries no PROVENANCE.md"), "{findings:?}");
    }

    fn recorded(files: &[(&str, &[u8])], extra_row: &str) -> String {
        let mut rows = String::new();
        for (name, bytes) in files {
            rows.push_str(&format!(
                "| `{name}` | {} | `{}` | https://example.invalid/{name} |\n",
                bytes.len(),
                sha256_hex(bytes)
            ));
        }
        format!(
            "| Field | Value |\n|---|---|\n| Kind | data |\n| Upstream | https://example.invalid/ |\n\
             | Version | 1.0.0 |\n| Imported | 2026-09-25 |\n| Changed | nothing |\n\n\
             | File | Bytes | SHA-256 | From |\n|---|---|---|---|\n{rows}{extra_row}"
        )
    }

    #[test]
    fn a_data_import_is_read_against_its_bytes() {
        let data: &[u8] = b"# D-1.0.0.txt\n0041;L\n";
        let files: [(&str, &[u8]); 2] = [("LICENSE", b"terms\n"), ("sub/D.txt", data)];
        let record = recorded(&files, "");
        let at = fixture(
            "import-data",
            &[
                ("third_party/d/LICENSE", b"terms\n"),
                ("third_party/d/sub/D.txt", data),
                ("third_party/d/PROVENANCE.md", record.as_bytes()),
            ],
        );
        let (findings, seen) = import_findings(&at).expect("listed");
        assert_eq!(findings, Vec::<String>::new());
        assert_eq!((seen.files, seen.versions), (2, 1));

        // One byte changed, one file unrecorded, one row with no file, one
        // version that disagrees, and one file a compiler could read.
        std::fs::write(at.join("third_party/d/sub/D.txt"), b"# D-1.0.1.txt\n0041;L\n").expect("w");
        std::fs::write(at.join("third_party/d/shim.rs"), b"fn main() {}\n").expect("w");
        let ghost = "| `gone.txt` | 1 | `".to_string() + &"a".repeat(64) + "` | x |\n";
        std::fs::write(at.join("third_party/d/PROVENANCE.md"), recorded(&files, &ghost))
            .expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        let all = findings.join("\n");
        for needle in [
            "sub/D.txt  hashes to",
            "sub/D.txt  says it is version 1.0.1",
            "shim.rs  is in a data import and is not a data file",
            "shim.rs  has no row",
            "records `gone.txt`, which is not in the import",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }
    }

    /// The real import, read against its record. Red until `PROVENANCE.md`
    /// exists beside the files, which is what it is for.
    #[test]
    fn the_import_this_tree_ships_with_is_recorded() {
        let (findings, seen) = import_findings(&root()).expect("listed");
        assert_eq!(findings, Vec::<String>::new());
        assert!(
            seen.trees >= 1 && seen.files >= 12,
            "{} tree(s), {} file(s)",
            seen.trees,
            seen.files
        );
    }

    /// RFC 0141's font rule: one extension and one magic, and nothing either
    /// side of it.
    #[test]
    fn the_font_rule_is_one_extension_and_one_magic() {
        use super::is_admitted_font;
        let truetype: &[u8] = &[0x00, 0x01, 0x00, 0x00, 0x00, 0x11];
        assert!(is_admitted_font("Inter-Regular.ttf", truetype));
        assert!(is_admitted_font("sub/Face.ttf", truetype));
        assert!(!is_admitted_font("Inter-Regular.otf", truetype), "a second extension");
        assert!(!is_admitted_font("Inter-Regular.woff2", truetype), "a wrapper");
        assert!(!is_admitted_font("Inter-Regular.ttc", truetype), "a collection");
        assert!(!is_admitted_font("Inter-Regular.TTF", truetype), "a spelling");
        assert!(!is_admitted_font("Inter-Regular.ttf", b"OTTO\0\x11"), "CFF outlines renamed");
        assert!(!is_admitted_font("Inter-Regular.ttf", b"ttcf\0\x02"), "a collection renamed");
        assert!(!is_admitted_font("Inter-Regular.ttf", b"wOF2"), "a wrapper renamed");
        assert!(!is_admitted_font("Inter-Regular.ttf", &truetype[..3]), "too short to say");
    }

    fn record_with(kind: &str, extra_fields: &str, tables: &str) -> String {
        format!(
            "| Field | Value |\n|---|---|\n| Kind | {kind} |\n| Upstream | https://example.invalid/ |\n\
             | Version | 1.0 |\n| Imported | 2026-09-26 |\n| Changed | nothing |\n{extra_fields}\n\
             {tables}"
        )
    }

    #[test]
    fn a_data_import_holds_a_face_and_names_its_archive() {
        let face: &[u8] = &[0x00, 0x01, 0x00, 0x00, 1, 2, 3, 4];
        let files = format!(
            "| Archive | Bytes | SHA-256 |\n|---|---|---|\n| `face.zip` | 12 | `{}` |\n\n\
             | File | Bytes | SHA-256 | From |\n|---|---|---|---|\n\
             | `LICENSE` | 6 | `{}` | x |\n| `Face.ttf` | {} | `{}` | x |\n",
            "b".repeat(64),
            sha256_hex(b"terms\n"),
            face.len(),
            sha256_hex(face)
        );
        let retrieved = "| Retrieved with | `curl` of `https://example.invalid/v1/face.zip`, then \
                         two files extracted |";
        let record = record_with("data", retrieved, &files);
        let at = fixture(
            "import-face",
            &[
                ("third_party/f/LICENSE", b"terms\n"),
                ("third_party/f/Face.ttf", face),
                ("third_party/f/PROVENANCE.md", record.as_bytes()),
            ],
        );
        let (findings, seen) = import_findings(&at).expect("listed");
        assert_eq!(findings, Vec::<String>::new());
        assert_eq!(seen.files, 2);

        // The same bytes under a second extension are not admitted, and an
        // archive kept in the import is not an archive.
        std::fs::write(at.join("third_party/f/Face.otf"), face).expect("w");
        std::fs::write(at.join("third_party/f/face.zip"), b"PK\x03\x04").expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        let all = findings.join("\n");
        for needle in [
            "Face.otf  is in a data import and is not a data file",
            "Face.otf  has no row",
            "face.zip  is recorded as the archive",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }

        // An archive row that is not an archive's.
        let bad = record_with("data", retrieved, &files.replace(&"b".repeat(64), "not-a-hash"));
        std::fs::write(at.join("third_party/f/PROVENANCE.md"), bad).expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        assert!(
            findings.iter().any(|f| f.contains("`not-a-hash` is not a SHA-256")),
            "{findings:#?}"
        );
    }

    /// A one-crate source import, as `cargo vendor --versioned-dirs` lays one
    /// out, with its record, its lockfile and its crate's checksum file.
    fn source_fixture(name: &str) -> (PathBuf, String) {
        let lib: &[u8] = b"pub fn f() {}\n";
        let manifest: &[u8] =
            b"[package]\nname = \"foo\"\nversion = \"1.0.0\"\nlicense = \"MIT OR Apache-2.0\"\n";
        let package = "c".repeat(64);
        let json = format!(
            "{{\"$comment\":\"cargo's own\",\"files\":{{\"Cargo.toml\":\"{}\",\"src/lib.rs\":\"{}\"}},\
             \"package\":\"{package}\"}}",
            sha256_hex(manifest),
            sha256_hex(lib)
        );
        let lock = format!(
            "version = 4\n\n[[package]]\nname = \"foo\"\nversion = \"1.0.0\"\n\
             source = \"registry+https://github.com/rust-lang/crates.io-index\"\n\
             checksum = \"{package}\"\n"
        );
        let crates = format!(
            "| Crate | Version | Licence | checksum | Why |\n|---|---|---|---|---|\n\
             | `foo` | 1.0.0 | MIT OR Apache-2.0 | `{package}` | the one |\n"
        );
        let record = record_with("source", &format!("| Commit | `{}` |", "d".repeat(40)), &crates);
        let at = fixture(
            name,
            &[
                ("third_party/s/LICENSE", b"terms\n"),
                ("third_party/s/PROVENANCE.md", record.as_bytes()),
                ("third_party/s/Cargo.lock", lock.as_bytes()),
                ("third_party/s/vendor/foo-1.0.0/Cargo.toml", manifest),
                ("third_party/s/vendor/foo-1.0.0/src/lib.rs", lib),
                ("third_party/s/vendor/foo-1.0.0/.cargo-checksum.json", json.as_bytes()),
            ],
        );
        (at, record)
    }

    #[test]
    fn a_source_import_is_read_crate_by_crate_and_file_by_file() {
        let (at, _) = source_fixture("import-source");
        let (findings, seen) = import_findings(&at).expect("listed");
        assert_eq!(findings, Vec::<String>::new());
        assert_eq!((seen.crates, seen.files), (1, 2));

        // One byte changed, one file nobody listed, one stray file at the top.
        std::fs::write(at.join("third_party/s/vendor/foo-1.0.0/src/lib.rs"), b"pub fn g() {}\n")
            .expect("w");
        std::fs::write(at.join("third_party/s/vendor/foo-1.0.0/src/extra.rs"), b"\n").expect("w");
        std::fs::write(at.join("third_party/s/shim.rs"), b"\n").expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        let all = findings.join("\n");
        for needle in [
            "foo-1.0.0/src/lib.rs  hashes to",
            "foo-1.0.0/src/extra.rs  is not listed in .cargo-checksum.json",
            "third_party/s/shim.rs  is in a source import outside `vendor/`",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }
    }

    #[test]
    fn a_source_record_is_held_to_its_lockfile_and_its_commit() {
        let (at, record) = source_fixture("import-source-record");
        let wrong = record.replace(&"c".repeat(64), &"e".repeat(64));
        std::fs::write(at.join("third_party/s/PROVENANCE.md"), &wrong).expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        let all = findings.join("\n");
        for needle in ["Cargo.lock pins", "names package"] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }

        let tagless = record.replace(&"d".repeat(40), "the one tagged 1.0.0");
        std::fs::write(at.join("third_party/s/PROVENANCE.md"), &tagless).expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        assert!(findings.iter().any(|f| f.contains("carries no forty-digit")), "{findings:#?}");

        let crateless = record.replace("| `foo` | 1.0.0 |", "| `bar` | 1.0.0 |");
        std::fs::write(at.join("third_party/s/PROVENANCE.md"), &crateless).expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        let all = findings.join("\n");
        for needle in ["foo-1.0.0/  has no `Crate` row", "records crate `bar-1.0.0`"] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }
    }

    #[test]
    fn a_source_imports_file_names_are_not_needles() {
        // The day HarfRust arrived its `lib.rs`, `mod.rs` and `README.md` were
        // needles, and three hundred sources were readers of the import.
        let (at, _) = source_fixture("import-source-needles");
        let face: &[u8] = &[0x00, 0x01, 0x00, 0x00];
        std::fs::create_dir_all(at.join("third_party/d")).expect("d");
        std::fs::write(at.join("third_party/d/Face.ttf"), face).expect("w");
        std::fs::write(at.join("third_party/d/PROVENANCE.md"), record_with("data", "", ""))
            .expect("w");
        let names = super::data_file_names(&at);
        assert!(names.contains(&"Face.ttf".to_string()), "{names:?}");
        assert!(!names.iter().any(|n| n == "lib.rs" || n == "Cargo.toml"), "{names:?}");
    }

    /// Audit 1's third finding: the `Licence` column was parsed and dropped, and
    /// a crate re-licensed in its own manifest (and its checksum file) was green.
    #[test]
    fn a_vendored_crates_licence_is_its_rows_and_one_this_tree_admits() {
        let (at, record) = source_fixture("import-source-licence");
        let crate_dir = at.join("third_party/s/vendor/foo-1.0.0");
        let manifest =
            b"[package]\nname = \"foo\"\nversion = \"1.0.0\"\nlicense = \"GPL-3.0-only\"\n";
        std::fs::write(crate_dir.join("Cargo.toml"), manifest).expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        let all = findings.join("\n");
        for needle in [
            "foo-1.0.0/Cargo.toml  is `GPL-3.0-only` and its `Crate` row in PROVENANCE.md records \
             `MIT OR Apache-2.0`",
            "foo-1.0.0  names `GPL-3.0-only`, which is not a licence an import's crate may carry",
        ] {
            assert!(all.contains(needle), "missing `{needle}` in:\n{all}");
        }

        // The row changed to match, so only the allow-list stands between.
        let agreed = record.replace("| MIT OR Apache-2.0 |", "| GPL-3.0-only |");
        std::fs::write(at.join("third_party/s/PROVENANCE.md"), agreed).expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        assert!(findings.iter().any(|f| f.contains("names `GPL-3.0-only`")), "{findings:#?}");
        assert!(!findings.iter().any(|f| f.contains("its `Crate` row")), "{findings:#?}");

        // No licence at all is not a licence.
        std::fs::write(crate_dir.join("Cargo.toml"), b"[package]\nname = \"foo\"\n").expect("w");
        let (findings, _) = import_findings(&at).expect("listed");
        assert!(findings.iter().any(|f| f.contains("states no `license`")), "{findings:#?}");
    }

    #[test]
    fn an_spdx_expression_names_every_licence_it_offers() {
        assert_eq!(
            super::licence_ids("(MIT OR Apache-2.0) AND Unicode-3.0"),
            ["MIT", "Apache-2.0", "Unicode-3.0"]
        );
        assert_eq!(super::licence_ids("MIT/Apache-2.0"), ["MIT", "Apache-2.0"]);
        assert_eq!(
            super::licence_ids("GPL-2.0-only WITH Classpath-exception-2.0"),
            ["GPL-2.0-only", "Classpath-exception-2.0"]
        );
    }

    /// The list is `deny.toml`'s and one more, and says so; this is what keeps
    /// that sentence true when either file changes.
    #[test]
    fn the_import_licences_are_deny_toml_s_and_zlib() {
        let deny = std::fs::read_to_string(root().join("deny.toml")).expect("deny.toml");
        let line = deny
            .lines()
            .find(|line| line.trim_start().starts_with("allow = ["))
            .expect("deny.toml's licence allow-list");
        let mut expected: Vec<&str> = line.split('"').skip(1).step_by(2).collect();
        expected.push("Zlib");
        expected.sort_unstable();
        let mut listed = super::IMPORT_LICENCES.to_vec();
        listed.sort_unstable();
        assert_eq!(listed, expected);
    }

    /// Audit 1's eighth finding: the `Archive` row was optional.
    #[test]
    fn an_archive_the_record_names_has_a_row() {
        let face: &[u8] = &[0x00, 0x01, 0x00, 0x00, 1, 2, 3, 4];
        let archive = format!(
            "| Archive | Bytes | SHA-256 |\n|---|---|---|\n| `face.zip` | 12 | `{}` |\n\n",
            "b".repeat(64)
        );
        let files = format!(
            "| File | Bytes | SHA-256 | From |\n|---|---|---|---|\n\
             | `LICENSE` | 6 | `{}` | x |\n| `Face.ttf` | {} | `{}` | x |\n",
            sha256_hex(b"terms\n"),
            face.len(),
            sha256_hex(face)
        );
        let retrieved = "| Retrieved with | `curl -fsSL https://example.invalid/face.zip` |";
        let write = |record: String| {
            fixture(
                "import-archive",
                &[
                    ("third_party/f/LICENSE", b"terms\n"),
                    ("third_party/f/Face.ttf", face),
                    ("third_party/f/PROVENANCE.md", record.as_bytes()),
                ],
            )
        };
        let at = write(record_with("data", retrieved, &format!("{archive}{files}")));
        assert_eq!(import_findings(&at).expect("listed").0, Vec::<String>::new());

        let at = write(record_with("data", retrieved, &files));
        let (findings, _) = import_findings(&at).expect("listed");
        assert!(
            findings
                .iter()
                .any(|f| f.contains("names the archive `face.zip` and no `Archive` row")),
            "{findings:#?}"
        );

        let at = write(record_with(
            "data",
            "| Retrieved with | by hand |",
            &format!("{archive}{files}"),
        ));
        let (findings, _) = import_findings(&at).expect("listed");
        assert!(
            findings.iter().any(|f| f.contains("records the archive `face.zip`, which")),
            "{findings:#?}"
        );
        assert_eq!(
            super::archives_named("`curl.exe -fsSL` of `https://h/v4.1/Inter-4.1.zip`, then two"),
            ["Inter-4.1.zip"]
        );
    }

    #[test]
    fn a_cargo_checksum_file_is_read_as_cargo_writes_it() {
        let json = "{\"$comment\":\"a \\\"quoted\\\" note\",\"files\":{\"a\\u002frs\":\"1\",\
                    \"b\":\"2\"},\"package\":\"3\"}";
        let (files, package) = super::cargo_checksum(json).expect("cargo's shape");
        assert_eq!(package, "3");
        assert_eq!(files.get("a/rs").map(String::as_str), Some("1"));
        assert_eq!(files.len(), 2);
        assert!(super::cargo_checksum("{\"files\":{}}").is_none(), "no package, no files");
        assert!(super::cargo_checksum("[1, 2]").is_none(), "not an object");
    }
}
