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

fn sha256_hex(bytes: &[u8]) -> String {
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
}

/// One import's `PROVENANCE.md`, as its two tables say it.
struct Provenance {
    fields: BTreeMap<String, String>,
    files: BTreeMap<String, (u64, String)>,
}

fn cells(line: &str) -> Option<Vec<String>> {
    let inner = line.trim().strip_prefix('|')?.strip_suffix('|')?;
    Some(inner.split('|').map(|c| c.trim().to_string()).collect())
}

/// Read both tables. A row this cannot read is a finding rather than a row it
/// skips, because a skipped row is a file with no record that looks recorded.
fn parse_provenance(rel: &str, text: &str) -> (Provenance, Vec<String>) {
    let mut findings = Vec::new();
    let mut fields = BTreeMap::new();
    let mut files = BTreeMap::new();
    for (index, line) in text.split('\n').enumerate() {
        let n = index + 1;
        let Some(row) = cells(line) else { continue };
        if row.iter().all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':')) {
            continue;
        }
        match row.as_slice() {
            [key, _] if key == "Field" => {}
            [key, value] => {
                if fields.insert(key.clone(), value.clone()).is_some() {
                    findings.push(format!("  {rel}:{n}  names `{key}` twice"));
                }
            }
            [file, _, _, _] if file == "File" => {}
            [file, bytes, sha, _from] => {
                let path = file.trim_matches('`').to_string();
                let sha = sha.trim_matches('`').to_string();
                let Ok(size) = bytes.parse::<u64>() else {
                    findings.push(format!("  {rel}:{n}  `{bytes}` is not a byte count"));
                    continue;
                };
                if sha.len() != 64 || !sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
                    findings
                        .push(format!("  {rel}:{n}  `{sha}` is not a SHA-256 in lowercase hex"));
                    continue;
                }
                if files.insert(path.clone(), (size, sha)).is_some() {
                    findings.push(format!("  {rel}:{n}  records `{path}` twice"));
                }
            }
            _ => findings.push(format!("  {rel}:{n}  a table row this reader cannot read")),
        }
    }
    (Provenance { fields, files }, findings)
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
/// `PROVENANCE.md`, and a data import's record is read against its bytes.
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
/// # What it does not check
///
/// A source import's commit against its upstream, since nothing here can reach
/// upstream: for `Kind: source` the fields are required and not verified. There
/// is no such import today, so the gap is stated before it has an occupant.
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
        if kind != Some("data") {
            continue;
        }
        let version = provenance.fields.get("Version").cloned().unwrap_or_default();
        let on_disk = files_under(&dir)?;
        for file in &on_disk {
            if file == "PROVENANCE.md" {
                continue;
            }
            let admitted = DATA_NAMES.contains(&file.as_str())
                || file.rsplit_once('.').is_some_and(|(_, ext)| DATA_EXTENSIONS.contains(&ext));
            if !admitted {
                findings.push(format!(
                    "  {rel}/{file}  is in a data import and is not a data file: the category \
                     holds files no compiler in this workspace reads (RFC 0114), held by \
                     extension — {} — rather than by a list of languages",
                    DATA_EXTENSIONS.join(", ")
                ));
            }
            let Some((size, sha)) = provenance.files.get(file) else {
                findings.push(format!("  {rel}/{file}  has no row in PROVENANCE.md"));
                continue;
            };
            let bytes =
                std::fs::read(dir.join(file)).map_err(|e| format!("reading {rel}/{file}: {e}"))?;
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
    }
    Ok((findings, seen))
}

/// The names of every file a data import holds, other than its licence and its
/// record: the second needle `IMPORT_READERS` reads for, because a path can be
/// assembled without spelling the import's directory and still spell its file.
pub fn data_file_names(at: &Path) -> Vec<String> {
    let mut names = BTreeSet::new();
    let Ok(entries) = std::fs::read_dir(at.join("third_party")) else { return Vec::new() };
    for dir in entries.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()) {
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
}
