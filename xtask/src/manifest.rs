// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The component manifest, as a check that can fail a build.
//!
//! `docs/manifest.md` is the schema a person reads; this is the same schema as
//! code, and the two are kept together by `cargo xtask lint-manifests`, which
//! runs this over every `manifest.toml` in the permissive tree on every lint.
//! A manifest that this refuses is a component the supervisor of E1-B05 will
//! refuse to spawn, found at lint time rather than at boot — that is the
//! *topology check* RFC 0005 promised in the R02 row of `CONTRIBUTING.md`.
//!
//! # Why this reads TOML itself
//!
//! `main.rs` reads `claims/*.toml` with three functions that know a scalar, a
//! key and a table, and it says of them that a claim needing more needs a
//! parser rather than a longer version of those. A manifest needs more — a
//! list of capabilities and a list of rings are arrays of tables — and the
//! tree's answer to "needs a parser" is not a dependency: `xtask` has none
//! outside the workspace and `.claude/skills/licence-boundary` says why one
//! bought for a format is a review finding. So this module is that parser, for
//! a **subset** of TOML chosen to be the smallest thing the schema needs:
//! comments, `[table]` and `[[array]]` headers, and `key = value` where the
//! value is a string with no escapes, an unsigned integer, a boolean, or a
//! one-line list of strings. Every valid manifest is valid TOML, so any TOML
//! reader accepts it; this reader accepts nothing else. Multi-line strings,
//! inline tables, dotted keys and escapes are refused with a line number, not
//! parsed approximately. That is R04 applied to the syntax before it is applied
//! to the fields.
//!
//! The subset is also a statement about the *wire* form. The supervisor does
//! not read TOML: the spawn entry of RFC 0008 names a manifest by content hash,
//! and what it hashes is a fixed-layout record E1-B05 defines in `abi/`. Every
//! bound in this file — [`NAME_MAX`], [`CAPABILITIES_MAX`], [`RINGS_MAX`] —
//! exists so that such a record can exist. A field the subset cannot express is
//! a field the record would not have room for either.
//!
//! # What this cannot see
//!
//! Stated so it is not believed to be closed. The manifest declares needs and
//! names siblings; the *topology* — which supervisor routes which sibling to
//! whom — is not in the file, so a `sibling:` reference is checked for shape and
//! not for existence. Whether a native component *holds a secret* and therefore
//! belongs in `private` is the judgement RFC 0005 leaves to review, and this
//! checks only that the field is present and closed. And a manifest whose image
//! is not yet built is reported, not refused, because writing the manifest
//! before the driver is the order `TODO.md` asks for — E1-D04 precedes E1-B02
//! the way a claim precedes its number.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// The schema version this checker knows, and the only value `schema` may
/// carry. A manifest written to a later schema is refused rather than read
/// approximately; a reader that guesses at fields it was not written for is
/// two readers with different beliefs about one file.
pub const SCHEMA: u64 = 1;

/// The longest component, capability or ring name, in bytes. Names are
/// `[a-z0-9-]`, so bytes are characters. Thirty-two is what a fixed-layout
/// record can afford per slot without the record being mostly names.
pub const NAME_MAX: usize = 32;

/// The most capabilities a manifest may declare, needs and asks together.
///
/// Half of `kernel::cap::TABLE_SLOTS`. The other half is what the component
/// mints and is granted at run time — channels from clients connecting, buffer
/// sets granted for a transfer — and a manifest that filled the table at spawn
/// would be a component that cannot accept a client. E1-B13 makes the table an
/// object paid for from `Untyped`, after which this bound is the wire record's
/// and not the table's; it does not go away, it stops being about memory.
pub const CAPABILITIES_MAX: usize = 16;

/// The most data rings a manifest may declare. The control ring is not one of
/// them: it is implicit, exactly one, and refused if declared (RFC 0008).
pub const RINGS_MAX: usize = 8;

/// Bounds on a data ring's `entries`. A power of two, as `ChannelHeader::
/// ring_size` requires; at least two because a ring with one slot cannot have a
/// producer and a consumer on different entries; at most this because a ring is
/// mapped memory the component's account pays for and sixty-four thousand
/// cache-line entries is four mebibytes of submission queue per client.
pub const RING_ENTRIES_MIN: u64 = 2;
/// See [`RING_ENTRIES_MIN`].
pub const RING_ENTRIES_MAX: u64 = 65_536;

/// The most simultaneous clients a server ring may declare. One SPSC ring per
/// client, always, with the count bounded at creation — `ring-scene-boot`
/// section 06 — and this is the bound on the bound.
pub const CLIENTS_MAX: u64 = 64;

/// One page, which is what a `frame` is and the grain `bytes` is stated in.
pub const FRAME_BYTES: u64 = 4096;

/// The hard class holds pre-faulted huge pages (RFC 0007), so a hard-class
/// `memory_bytes` is a multiple of this.
pub const HUGE_BYTES: u64 = 2 * 1024 * 1024;

/// The three speculation-domain kinds of RFC 0005, in the RFC's spelling. The
/// only values `domain` may carry; there is no default.
pub const DOMAINS: &[&str] = &["shared", "private", "hostile"];

/// `abi::cap::CapType`, one snake_case word per variant, in wire order. The
/// test `the_type_table_matches_the_abi` reads the enum out of `abi/src/cap.rs`
/// and fails when this drifts from it, because `xtask` cannot depend on the
/// crate without changing `Cargo.lock` and a table nobody checks is a table
/// that is wrong within a milestone.
pub const CAP_TYPES: &[&str] =
    &["untyped", "frame", "address_space", "channel", "endpoint", "irq", "buffer_set"];

/// `abi::cap::rights`, one word per bit. Checked against the source the same
/// way as [`CAP_TYPES`].
pub const RIGHTS: &[&str] = &["read", "write", "execute", "derive", "revoke", "grant"];

/// `abi::feature`, one word per bit. Checked against the source the same way.
pub const FEATURES: &[&str] =
    &["shared_virtual_memory", "user_interrupt_doorbell", "admission_control", "control_events"];

/// What the supervisor does when a component ends. `docs/manifest.md` says what
/// each means; RFC 0008 says a restart is a new spawn and not a resurrection.
pub const RESTART_POLICIES: &[&str] = &["never", "on_fault", "always"];

/// RFC 0007's two classes.
pub const CLASSES: &[&str] = &["soft", "hard"];

/// How a data ring's payload reaches the peer. `inline` is in the entry;
/// `registered` is a registered buffer set (`ring-scene-boot` section 04, the
/// zero-copy path E1-B02's exit counts); `shared_virtual` is the device walking
/// the submitter's own page tables, behind its feature bit.
pub const PAYLOADS: &[&str] = &["inline", "registered", "shared_virtual"];

/// Which end of a data ring the component occupies.
pub const ROLES: &[&str] = &["server", "client"];

/// The header every manifest starts with. A manifest is authored source that
/// names an image and is hashed into a component's identity, so its licence is
/// part of what is being hashed.
pub const SPDX: &str = "# SPDX-License-Identifier: Apache-2.0 OR MIT";

/// The file name that makes a file a manifest. Discovery is by name and not by
/// directory, so where a component lives is not load-bearing for the lint.
pub const FILE_NAME: &str = "manifest.toml";

/// What a checked manifest declares, reduced to what the lint and the assembler
/// need after the check. E1-B05's wire record is the full form; this is not it,
/// and is not `#[repr(C)]` on purpose.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    /// `name`.
    pub name: String,
    /// `image`, verbatim: a tree-relative path or `sha256:<hex>`.
    pub image: String,
    /// `domain`, one of [`DOMAINS`].
    pub domain: String,
    /// How many `[[capability]]` entries, needs and asks together.
    pub capabilities: usize,
    /// How many `[[ring]]` entries, the control ring excluded.
    pub rings: usize,
    /// `[restart] policy`.
    pub restart: String,
}

impl Manifest {
    /// Is the image named by content rather than by a path into the tree?
    #[must_use]
    pub fn image_is_hash(&self) -> bool {
        self.image.starts_with("sha256:")
    }
}

/// Whether the image a manifest names is in the tree yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Image {
    /// A crate directory exists at the path.
    Present,
    /// No crate at the path yet — nothing there, or a directory holding the
    /// manifest and no `Cargo.toml`. Not a finding — see the module
    /// documentation — but reported, so a manifest for a component nobody built
    /// is visible in every lint run rather than in none.
    NotYet,
    /// Named by hash; there is nothing in the tree to look at.
    ByHash,
    /// A file is at the path. An image is built from a crate, so this is a
    /// path that was typed wrong rather than a crate that is not written yet.
    Wrong(String),
}

/// Look for the image a checked manifest names, under `root`.
#[must_use]
pub fn image_state(root: &Path, manifest: &Manifest) -> Image {
    if manifest.image_is_hash() {
        return Image::ByHash;
    }
    let path = root.join(&manifest.image);
    if !path.exists() {
        return Image::NotYet;
    }
    if !path.is_dir() {
        return Image::Wrong(format!(
            "{} is a file, and an image is built from a crate",
            manifest.image
        ));
    }
    // A directory with no crate in it is the declared-before-built state, not
    // a mistake: the manifest lives in the directory the image will be built
    // from, so the directory exists the moment the manifest does.
    if !path.join("Cargo.toml").exists() {
        return Image::NotYet;
    }
    Image::Present
}

/// Every component manifest in the permissive tree, sorted.
///
/// `third_party/` is excluded on purpose and not for the reason the other walks
/// exclude it. An imported tree is verbatim and may contain a file called
/// anything; and a manifest is *policy* — which domain, which capabilities,
/// which restart — authored and reviewed here, which a re-import must not be
/// able to change. So an imported driver's manifest lives in the permissive tree
/// and its `image` points into `third_party/`, which is exactly the shape RFC
/// 0005 rule 4 checks.
pub fn files(root: &Path, build: &Path) -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, build: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !matches!(name, "target" | ".git" | "third_party" | "docs") && path != build {
                    walk(&path, build, out)?;
                }
            } else if name == FILE_NAME {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(root, build, &mut out).map_err(|e| format!("walking the tree for manifests: {e}"))?;
    out.sort();
    Ok(out)
}

/// Check one manifest. `rel` is the path findings are reported against.
///
/// # Errors
///
/// Every finding, each on its own line in the shape the other lints use, so
/// that a manifest with three things wrong reports three things. A syntax
/// error stops the check at the syntax: fields are not judged in a file whose
/// shape is not known.
pub fn check(rel: &str, text: &str) -> Result<Manifest, Vec<String>> {
    let mut findings = Vec::new();
    if text.lines().next().map(|l| l.trim_end()) != Some(SPDX) {
        findings.push(format!("  {rel}:1  first line is not `{SPDX}`"));
    }
    let doc = match parse(rel, text) {
        Ok(doc) => doc,
        Err(mut syntax) => {
            findings.append(&mut syntax);
            return Err(findings);
        }
    };
    let manifest = Checker { rel, findings: &mut findings }.run(doc);
    match manifest {
        Some(manifest) if findings.is_empty() => Ok(manifest),
        _ => Err(findings),
    }
}

// ---------------------------------------------------------------------------
// The reader.
// ---------------------------------------------------------------------------

/// A value the subset admits.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Value {
    Str(String),
    Int(u64),
    Bool(bool),
    List(Vec<String>),
}

impl Value {
    fn kind(&self) -> &'static str {
        match self {
            Self::Str(_) => "a string",
            Self::Int(_) => "an integer",
            Self::Bool(_) => "a boolean",
            Self::List(_) => "a list",
        }
    }
}

/// One `key = value`, with the line it came from so a finding can point at it.
#[derive(Debug, Clone)]
struct Entry {
    line: usize,
    value: Value,
}

/// The keys of one table. A `BTreeMap` and not a `Vec`, because a duplicate key
/// is refused at insertion and because the iteration order of leftover keys in
/// a finding is then the same on every run — RFC 0004 applies to the checker
/// too, and `xtask` is linted by the rule it implements.
type Table = BTreeMap<String, Entry>;

/// A whole file, in the three shapes the subset knows.
#[derive(Debug, Default)]
struct Doc {
    top: Table,
    /// `[name]`, with the header's line.
    tables: BTreeMap<String, (usize, Table)>,
    /// `[[name]]`, in file order, each with its header's line.
    arrays: BTreeMap<String, Vec<(usize, Table)>>,
}

/// Which table the next `key = value` lands in.
enum Cursor {
    Top,
    Table(String),
    Array(String, usize),
}

/// The text before a `#` that is not inside a string.
fn strip_comment(line: &str) -> &str {
    let mut quoted = false;
    for (at, c) in line.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return &line[..at],
            _ => {}
        }
    }
    line
}

/// A bare key or header name: `[a-z_]+`. Nothing in the schema needs more, and
/// a quoted or dotted key is a TOML feature this reader would have to guess at.
fn is_bare(key: &str) -> bool {
    !key.is_empty() && key.bytes().all(|b| b.is_ascii_lowercase() || b == b'_')
}

/// A name a manifest gives something: `[a-z0-9-]`, no edge hyphen, bounded.
fn is_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= NAME_MAX
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !name.starts_with('-')
        && !name.ends_with('-')
}

/// One string literal: opening and closing quotes, no escapes, no inner quote.
fn parse_string(raw: &str) -> Result<String, &'static str> {
    let inner = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or("a string starts and ends with a double quote")?;
    if inner.contains('"') {
        return Err("a string contains no quote; there are no escapes in this subset");
    }
    if inner.contains('\\') {
        return Err("a string contains no backslash; a name that needs an escape is not a name");
    }
    Ok(inner.to_string())
}

/// One value of the subset, or the sentence saying why it is not one.
fn parse_value(raw: &str) -> Result<Value, String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Err("no value after `=`".into());
    }
    if raw.starts_with('"') {
        return parse_string(raw).map(Value::Str).map_err(Into::into);
    }
    if let Some(inner) = raw.strip_prefix('[') {
        let inner = inner
            .strip_suffix(']')
            .ok_or_else(|| "a list closes on the same line it opens".to_string())?;
        let inner = inner.trim();
        if inner.is_empty() {
            return Ok(Value::List(Vec::new()));
        }
        let mut items = Vec::new();
        for item in inner.split(',') {
            let item = item.trim();
            if item.is_empty() {
                return Err("a list has no empty item and no trailing comma".into());
            }
            items.push(parse_string(item).map_err(|why| format!("in a list, {why}"))?);
        }
        return Ok(Value::List(items));
    }
    match raw {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        _ => {}
    }
    if raw.bytes().all(|b| b.is_ascii_digit() || b == b'_') {
        if raw.starts_with('_') || raw.ends_with('_') || raw.contains("__") {
            return Err("an integer's underscores sit between digits".into());
        }
        let digits: String = raw.chars().filter(|c| *c != '_').collect();
        return digits
            .parse::<u64>()
            .map(Value::Int)
            .map_err(|_| "an integer fits in sixty-four unsigned bits".to_string());
    }
    if raw.starts_with('-') || raw.starts_with('+') {
        return Err("no quantity in a manifest is signed; a negative one is refused".into());
    }
    if raw.starts_with('{') {
        return Err("no inline tables; write a `[table]` header".into());
    }
    Err(format!("`{raw}` is not a string, an unsigned integer, a boolean or a list of strings"))
}

/// Read the subset. Every syntax error is collected; none is repaired.
fn parse(rel: &str, text: &str) -> Result<Doc, Vec<String>> {
    let mut doc = Doc::default();
    let mut errors = Vec::new();
    let mut cursor = Cursor::Top;

    for (index, raw) in text.lines().enumerate() {
        let line = index + 1;
        let code = strip_comment(raw).trim();
        if code.is_empty() {
            continue;
        }
        let refuse = |errors: &mut Vec<String>, why: &str| {
            errors.push(format!("  {rel}:{line}  {why}"));
        };

        if let Some(inner) = code.strip_prefix("[[") {
            let Some(name) = inner.strip_suffix("]]").map(str::trim) else {
                refuse(&mut errors, "an array header is `[[name]]` on one line");
                continue;
            };
            if !is_bare(name) {
                refuse(&mut errors, &format!("`[[{name}]]` is not a bare lower-case name"));
                continue;
            }
            let items = doc.arrays.entry(name.to_string()).or_default();
            items.push((line, Table::new()));
            cursor = Cursor::Array(name.to_string(), items.len() - 1);
            continue;
        }
        if let Some(inner) = code.strip_prefix('[') {
            let Some(name) = inner.strip_suffix(']').map(str::trim) else {
                refuse(&mut errors, "a table header is `[name]` on one line");
                continue;
            };
            if !is_bare(name) {
                refuse(&mut errors, &format!("`[{name}]` is not a bare lower-case name"));
                continue;
            }
            if doc.tables.contains_key(name) {
                refuse(&mut errors, &format!("`[{name}]` appears twice; a table is declared once"));
                continue;
            }
            doc.tables.insert(name.to_string(), (line, Table::new()));
            cursor = Cursor::Table(name.to_string());
            continue;
        }

        let Some((key, value)) = code.split_once('=') else {
            refuse(&mut errors, "a line is a header, a comment, or `key = value`");
            continue;
        };
        let key = key.trim();
        if !is_bare(key) {
            refuse(
                &mut errors,
                &format!("`{key}` is not a bare lower-case key; no quoted or dotted keys"),
            );
            continue;
        }
        let value = match parse_value(value) {
            Ok(value) => value,
            Err(why) => {
                refuse(&mut errors, &format!("`{key}`: {why}"));
                continue;
            }
        };
        let table = match &cursor {
            Cursor::Top => &mut doc.top,
            Cursor::Table(name) => &mut doc.tables.get_mut(name).expect("cursor names a table").1,
            Cursor::Array(name, at) => {
                &mut doc.arrays.get_mut(name).expect("cursor names an array")[*at].1
            }
        };
        if table.contains_key(key) {
            refuse(&mut errors, &format!("`{key}` appears twice in one table"));
            continue;
        }
        table.insert(key.to_string(), Entry { line, value });
    }

    if errors.is_empty() { Ok(doc) } else { Err(errors) }
}

// ---------------------------------------------------------------------------
// The schema.
// ---------------------------------------------------------------------------

/// One table's fields, consumed key by key so that whatever is left at the end
/// is unknown and refused. The consuming is the point: a checker that looked
/// keys up would pass a file with a misspelt optional field, which is the
/// silent acceptance R04 forbids.
struct Fields<'a> {
    rel: &'a str,
    /// Where these fields are, for the reader: `[restart]`, `[[ring]] #2`.
    place: String,
    /// The header's line, for a finding about a field that is missing.
    line: usize,
    table: Table,
    findings: &'a mut Vec<String>,
}

impl Fields<'_> {
    fn refuse(&mut self, line: usize, why: &str) {
        self.findings.push(format!("  {}:{line}  {}: {why}", self.rel, self.place));
    }

    fn take(&mut self, key: &str, required: bool) -> Option<Entry> {
        let entry = self.table.remove(key);
        if entry.is_none() && required {
            self.refuse(self.line, &format!("`{key}` is required and missing"));
        }
        entry
    }

    fn string(&mut self, key: &str, required: bool) -> Option<(usize, String)> {
        let entry = self.take(key, required)?;
        match entry.value {
            Value::Str(s) => Some((entry.line, s)),
            other => {
                self.refuse(entry.line, &format!("`{key}` is a string, not {}", other.kind()));
                None
            }
        }
    }

    fn int(&mut self, key: &str, required: bool) -> Option<(usize, u64)> {
        let entry = self.take(key, required)?;
        match entry.value {
            Value::Int(n) => Some((entry.line, n)),
            other => {
                self.refuse(entry.line, &format!("`{key}` is an integer, not {}", other.kind()));
                None
            }
        }
    }

    fn boolean(&mut self, key: &str) -> Option<(usize, bool)> {
        let entry = self.take(key, false)?;
        match entry.value {
            Value::Bool(b) => Some((entry.line, b)),
            other => {
                self.refuse(entry.line, &format!("`{key}` is a boolean, not {}", other.kind()));
                None
            }
        }
    }

    fn list(&mut self, key: &str, required: bool) -> Option<(usize, Vec<String>)> {
        let entry = self.take(key, required)?;
        match entry.value {
            Value::List(items) => Some((entry.line, items)),
            other => {
                self.refuse(
                    entry.line,
                    &format!("`{key}` is a list of strings, not {}", other.kind()),
                );
                None
            }
        }
    }

    /// A string that must be one of a closed set. The set is printed in the
    /// refusal so the reader is told the vocabulary rather than sent to look.
    fn one_of(&mut self, key: &str, allowed: &[&str]) -> Option<(usize, String)> {
        let (line, value) = self.string(key, true)?;
        if allowed.contains(&value.as_str()) {
            return Some((line, value));
        }
        self.refuse(
            line,
            &format!(
                "`{key} = \"{value}\"` is not one of {}; unknown values are refused",
                quoted(allowed)
            ),
        );
        None
    }

    /// A list whose items must each be one of a closed set, once.
    fn subset_of(
        &mut self,
        key: &str,
        allowed: &[&str],
        required: bool,
    ) -> Option<(usize, Vec<String>)> {
        let (line, items) = self.list(key, required)?;
        let mut seen = BTreeSet::new();
        let mut sound = true;
        for item in &items {
            if !allowed.contains(&item.as_str()) {
                self.refuse(
                    line,
                    &format!("`{key}` names \"{item}\", which is not one of {}", quoted(allowed)),
                );
                sound = false;
            } else if !seen.insert(item.as_str()) {
                self.refuse(line, &format!("`{key}` names \"{item}\" twice"));
                sound = false;
            }
        }
        sound.then_some((line, items))
    }

    /// A field that means nothing here and is therefore refused rather than
    /// ignored: a reader who sees it will believe it.
    fn forbid(&mut self, key: &str, why: &str) {
        if let Some(entry) = self.table.remove(key) {
            self.refuse(entry.line, &format!("`{key}` is refused here: {why}"));
        }
    }

    /// Whatever was not consumed is unknown.
    fn finish(mut self) {
        let leftover: Vec<(String, usize)> =
            self.table.iter().map(|(k, e)| (k.clone(), e.line)).collect();
        for (key, line) in leftover {
            self.refuse(
                line,
                &format!("`{key}` is not a field of this table; unknown fields are refused"),
            );
        }
    }
}

fn quoted(words: &[&str]) -> String {
    let list: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
    list.join(", ")
}

/// A capability as the ring check needs to see it.
struct Cap {
    name: String,
    kind: String,
    rights: Vec<String>,
}

struct Checker<'a> {
    rel: &'a str,
    findings: &'a mut Vec<String>,
}

impl Checker<'_> {
    fn fields(&mut self, place: String, line: usize, table: Table) -> Fields<'_> {
        Fields { rel: self.rel, place, line, table, findings: &mut *self.findings }
    }

    fn note(&mut self, line: usize, why: &str) {
        self.findings.push(format!("  {}:{line}  {why}", self.rel));
    }

    fn run(mut self, doc: Doc) -> Option<Manifest> {
        let Doc { top, mut tables, mut arrays } = doc;

        // Top level.
        let mut f = self.fields("top level".into(), 1, top);
        if let Some((line, schema)) = f.int("schema", true)
            && schema != SCHEMA
        {
            f.refuse(
                line,
                &format!(
                    "`schema = {schema}`; this checker knows schema {SCHEMA} and refuses others"
                ),
            );
        }
        let name = f.string("name", true).and_then(|(line, name)| {
            if is_name(&name) {
                Some(name)
            } else {
                f.refuse(line, &format!("`name = \"{name}\"` is not `[a-z0-9-]`, at most {NAME_MAX} bytes, with no edge hyphen"));
                None
            }
        });
        let image = f.string("image", true).and_then(|(line, image)| {
            if let Some(why) = image_problem(&image) {
                f.refuse(line, &format!("`image = \"{image}\"`: {why}"));
                None
            } else {
                Some(image)
            }
        });
        let domain = f.one_of("domain", DOMAINS).map(|(_, d)| d);
        f.finish();

        // The two rules of RFC 0005 that a lint can apply to `image`.
        if let (Some(image), Some(domain)) = (&image, &domain)
            && image.starts_with("third_party/")
            && domain == "shared"
        {
            self.note(1, "an image built from third_party/ may not declare `domain = \"shared\"` — RFC 0005 rule 4: the licence boundary is the speculation boundary");
        }
        if let (Some(image), Some(domain)) = (&image, &domain)
            && image.starts_with("sha256:")
            && domain != "hostile"
        {
            self.note(1, "an image named by hash has no source in this tree, so nobody here vouches for it; RFC 0005's table puts that in `domain = \"hostile\"`");
        }

        // Capabilities.
        let mut caps: Vec<Cap> = Vec::new();
        let cap_items = arrays.remove("capability").unwrap_or_default();
        if cap_items.len() > CAPABILITIES_MAX {
            self.note(cap_items[CAPABILITIES_MAX].0, &format!("more than {CAPABILITIES_MAX} capabilities; the bound is half the table, and the rest is for what arrives at run time"));
        }
        let mut cap_names = BTreeSet::new();
        for (index, (line, table)) in cap_items.into_iter().enumerate() {
            let place = format!("[[capability]] #{}", index + 1);
            let own = name.clone();
            let mut f = self.fields(place, line, table);
            let cap_name = f.string("name", true).and_then(|(line, n)| {
                if !is_name(&n) {
                    f.refuse(line, &format!("`name = \"{n}\"` is not `[a-z0-9-]`, at most {NAME_MAX} bytes, with no edge hyphen"));
                    return None;
                }
                if !cap_names.insert(n.clone()) {
                    f.refuse(line, &format!("a second capability named \"{n}\"; names are what a ring's `to` refers to"));
                    return None;
                }
                Some(n)
            });
            let kind = f.one_of("type", CAP_TYPES).map(|(_, k)| k);
            let rights = f.subset_of("rights", RIGHTS, true);
            if let (Some(kind), Some((line, rights))) = (&kind, &rights)
                && kind == "endpoint"
                && rights.iter().any(|r| r == "execute")
            {
                f.refuse(*line, "`execute` on an endpoint is undefined and a derivation asking for it is refused — RFC 0008");
            }
            if let Some((line, from)) = f.string("from", true) {
                match from.as_str() {
                    "supervisor" | "powerbox" => {}
                    other => match other.strip_prefix("sibling:") {
                        Some(sibling) if is_name(sibling) => {
                            if own.as_deref() == Some(sibling) {
                                f.refuse(line, "`from = \"sibling:…\"` names this component; a component is not its own sibling");
                            }
                        }
                        _ => f.refuse(line, &format!("`from = \"{from}\"` is not \"supervisor\", \"powerbox\" or \"sibling:<name>\"")),
                    },
                }
            }
            f.boolean("optional");
            match kind.as_deref() {
                Some("frame") => {
                    if let Some((line, frames)) = f.int("frames", true)
                        && frames == 0
                    {
                        f.refuse(line, "`frames = 0` is a frame set with nothing in it");
                    }
                    f.forbid("bytes", "a frame set is counted in `frames`, each one page");
                }
                Some("untyped") => {
                    if let Some((line, bytes)) = f.int("bytes", true)
                        && (bytes == 0 || bytes % FRAME_BYTES != 0)
                    {
                        f.refuse(line, &format!("`bytes = {bytes}` is not a positive multiple of {FRAME_BYTES}; untyped memory is retyped a page at a time"));
                    }
                    f.forbid("frames", "untyped memory is stated in `bytes`");
                }
                _ => {
                    f.forbid("frames", "only a `frame` capability has a count");
                    f.forbid("bytes", "only an `untyped` capability has a size");
                }
            }
            f.finish();
            if let (Some(n), Some(k), Some((_, r))) = (cap_name, kind, rights) {
                caps.push(Cap { name: n, kind: k, rights: r });
            }
        }

        // Rings.
        let ring_items = arrays.remove("ring").unwrap_or_default();
        if ring_items.len() > RINGS_MAX {
            self.note(ring_items[RINGS_MAX].0, &format!("more than {RINGS_MAX} data rings"));
        }
        let ring_count = ring_items.len();
        let mut ring_names = BTreeSet::new();
        for (index, (line, table)) in ring_items.into_iter().enumerate() {
            let place = format!("[[ring]] #{}", index + 1);
            let mut f = self.fields(place, line, table);
            if let Some((line, n)) = f.string("name", true) {
                if n == "control" {
                    f.refuse(line, "the control ring is implicit and exactly one — RFC 0008; a manifest declares data rings only");
                } else if !is_name(&n) {
                    f.refuse(line, &format!("`name = \"{n}\"` is not `[a-z0-9-]`, at most {NAME_MAX} bytes, with no edge hyphen"));
                } else if !ring_names.insert(n.clone()) {
                    f.refuse(line, &format!("a second ring named \"{n}\""));
                }
            }
            let role = f.one_of("role", ROLES).map(|(_, r)| r);
            if let Some((line, protocol)) = f.string("protocol", true) {
                let sound = !protocol.is_empty()
                    && protocol.len() <= NAME_MAX
                    && protocol.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.'
                    });
                if !sound {
                    f.refuse(line, &format!("`protocol = \"{protocol}\"` is not `[a-z0-9.-]` of at most {NAME_MAX} bytes"));
                }
            }
            let min = f.int("version_min", true);
            let max = f.int("version", true);
            if let Some((line, v)) = min
                && v == 0
            {
                f.refuse(line, "`version_min = 0`; zero is not a version — RFC 0011");
            }
            if let Some((line, v)) = max
                && v == 0
            {
                f.refuse(line, "`version = 0`; zero is not a version — RFC 0011");
            }
            if let (Some((_, lo)), Some((line, hi))) = (min, max)
                && lo > hi
            {
                f.refuse(line, &format!("`version_min = {lo}` is above `version = {hi}`; the floor is never above the ceiling"));
            }
            if let Some((line, entries)) = f.int("entries", true)
                && (!entries.is_power_of_two()
                    || !(RING_ENTRIES_MIN..=RING_ENTRIES_MAX).contains(&entries))
            {
                f.refuse(line, &format!("`entries = {entries}` is not a power of two between {RING_ENTRIES_MIN} and {RING_ENTRIES_MAX}"));
            }
            let payload = f.one_of("payload", PAYLOADS).map(|(_, p)| p);
            let features = f.subset_of("features", FEATURES, false);
            let offered: Vec<String> =
                features.as_ref().map(|(_, v)| v.clone()).unwrap_or_default();
            if let Some((line, _)) = &features
                && offered.iter().any(|x| x == "control_events")
            {
                f.refuse(*line, "`control_events` belongs to the control ring alone — RFC 0008; a data ring that offers it is two control rings");
            }
            if payload.as_deref() == Some("shared_virtual")
                && !offered.iter().any(|x| x == "shared_virtual_memory")
            {
                f.refuse(line, "`payload = \"shared_virtual\"` without `shared_virtual_memory` in `features`; the payload path is the feature bit — R01, name the mechanism");
            }
            if let Some((line, required)) = f.subset_of("features_required", FEATURES, false) {
                for item in &required {
                    if !offered.contains(item) {
                        f.refuse(line, &format!("`features_required` names \"{item}\", which `features` does not offer; the required set is a subset of the offered one — RFC 0011"));
                    }
                }
            }
            match role.as_deref() {
                Some("server") => {
                    if let Some((line, clients)) = f.int("clients", true)
                        && (clients == 0 || clients > CLIENTS_MAX)
                    {
                        f.refuse(line, &format!("`clients = {clients}` is not between 1 and {CLIENTS_MAX}; one SPSC ring per client, bounded at creation"));
                    }
                    f.forbid("to", "a server ring is connected to, through the endpoint the supervisor holds; it names nobody");
                }
                Some("client") => {
                    if let Some((line, to)) = f.string("to", true) {
                        match caps.iter().find(|c| c.name == to) {
                            None => f.refuse(line, &format!("`to = \"{to}\"` names no capability in this manifest")),
                            Some(cap) if cap.kind != "endpoint" => f.refuse(line, &format!("`to = \"{to}\"` names a {} capability; a client connects through an endpoint", cap.kind)),
                            Some(cap) if !cap.rights.iter().any(|r| r == "write") => f.refuse(line, &format!("`to = \"{to}\"` names an endpoint without `write`, and `write` on an endpoint is the right to connect — RFC 0008")),
                            Some(_) => {}
                        }
                    }
                    f.forbid("clients", "a client ring has one peer; `clients` bounds a server");
                }
                _ => {}
            }
            f.finish();
        }

        // Restart.
        let restart = match tables.remove("restart") {
            None => {
                self.note(
                    1,
                    "no `[restart]` table; a restart policy is declared, never assumed — RFC 0008",
                );
                None
            }
            Some((line, table)) => {
                let mut f = self.fields("[restart]".into(), line, table);
                let policy = f.one_of("policy", RESTART_POLICIES).map(|(_, p)| p);
                match policy.as_deref() {
                    Some("never") => {
                        let why = "`policy = \"never\"` restarts nothing, so a backoff or a count would be read and never applied";
                        f.forbid("backoff_first_ms", why);
                        f.forbid("backoff_max_ms", why);
                        f.forbid("max_restarts", why);
                    }
                    Some(_) => {
                        let first = f.int("backoff_first_ms", true);
                        let max = f.int("backoff_max_ms", true);
                        if let Some((line, v)) = first
                            && v == 0
                        {
                            f.refuse(
                                line,
                                "`backoff_first_ms = 0` is a restart loop with no pause in it",
                            );
                        }
                        if let (Some((_, lo)), Some((line, hi))) = (first, max)
                            && hi < lo
                        {
                            f.refuse(
                                line,
                                &format!(
                                    "`backoff_max_ms = {hi}` is below `backoff_first_ms = {lo}`"
                                ),
                            );
                        }
                        if let Some((line, n)) = f.int("max_restarts", true)
                            && n == 0
                        {
                            f.refuse(line, "`max_restarts = 0` is `policy = \"never\"` under another name; say that instead");
                        }
                    }
                    None => {}
                }
                f.finish();
                policy
            }
        };

        // Reservation.
        match tables.remove("reservation") {
            None => self.note(1, "no `[reservation]` table; admission control refuses what was not declared — RFC 0007, E1-B07"),
            Some((line, table)) => {
                let mut f = self.fields("[reservation]".into(), line, table);
                let class = f.one_of("class", CLASSES).map(|(_, c)| c);
                let memory = f.int("memory_bytes", true);
                if let Some((line, bytes)) = memory {
                    let grain = if class.as_deref() == Some("hard") { HUGE_BYTES } else { FRAME_BYTES };
                    if bytes == 0 || bytes % grain != 0 {
                        f.refuse(line, &format!("`memory_bytes = {bytes}` is not a positive multiple of {grain}; the {} class is granted in that grain", class.as_deref().unwrap_or("declared")));
                    }
                }
                match class.as_deref() {
                    Some("hard") => {
                        if let Some((line, cores)) = f.int("cores", true)
                            && cores == 0
                        {
                                f.refuse(line, "`cores = 0` reserves no core, and the hard class holds whole physical cores — RFC 0007");
                        }
                        let period = f.int("cpu_period_ns", true);
                        let budget = f.int("cpu_budget_ns", true);
                        if let Some((line, p)) = period
                            && p == 0
                        {
                                f.refuse(line, "`cpu_period_ns = 0`; a period is what admission tests against");
                        }
                        if let (Some((_, p)), Some((line, b))) = (period, budget)
                            && (b == 0 || b > p)
                        {
                                f.refuse(line, &format!("`cpu_budget_ns = {b}` is not between 1 and the period {p}"));
                        }
                    }
                    Some(_) => {
                        let why = "the soft class has no core, period or budget; it is scheduled around the hard class and refused nothing at admission but memory — RFC 0007";
                        f.forbid("cores", why);
                        f.forbid("cpu_period_ns", why);
                        f.forbid("cpu_budget_ns", why);
                    }
                    None => {}
                }
                f.finish();
            }
        }

        // Anything else at the top of the file is unknown.
        for (table, (line, _)) in tables {
            self.note(
                line,
                &format!("`[{table}]` is not a table of this schema; unknown tables are refused"),
            );
        }
        for (array, items) in arrays {
            if let Some((line, _)) = items.first() {
                self.note(
                    *line,
                    &format!(
                        "`[[{array}]]` is not an array of this schema; unknown arrays are refused"
                    ),
                );
            }
        }

        Some(Manifest {
            name: name?,
            image: image?,
            domain: domain?,
            capabilities: caps.len(),
            rings: ring_count,
            restart: restart?,
        })
    }
}

/// Why an `image` value is not one, or `None` if it is.
fn image_problem(image: &str) -> Option<&'static str> {
    if let Some(hex) = image.strip_prefix("sha256:") {
        let sound = hex.len() == 64
            && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        return (!sound).then_some(
            "a content address is `sha256:` and exactly sixty-four lower-case hex digits",
        );
    }
    if image.is_empty() {
        return Some("empty");
    }
    if image.starts_with('/') || image.contains(':') || image.contains('\\') {
        return Some(
            "a path is relative to the tree root, with forward slashes, or a `sha256:` content address",
        );
    }
    if image.split('/').any(|seg| seg.is_empty() || seg == "." || seg == "..") {
        return Some("a path has no empty, `.` or `..` segment; it names one place in this tree");
    }
    if image.starts_with("target/") {
        return Some("build output is not a source of images; name the crate that builds it");
    }
    None
}

// ---------------------------------------------------------------------------
// Fixtures. A lint that has never failed is indistinguishable from one that
// cannot, so each rule above is broken here on purpose. Strings rather than
// files, for the reason `mechanised_rules` in main.rs gives: a broken file on
// disk would be checked by every other lint too.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// A manifest with nothing wrong, small enough to read in one look. The
    /// worked example is `user/virtio-blk/manifest.toml`; this one exists so a
    /// test can break a single line of it.
    const SOUND: &str = "\
# SPDX-License-Identifier: Apache-2.0 OR MIT
schema = 1
name   = \"example\"
image  = \"user/example\"
domain = \"shared\"

[[capability]]
name   = \"pages\"
type   = \"frame\"
rights = [\"read\", \"write\"]
from   = \"supervisor\"
frames = 2

[[capability]]
name   = \"store\"
type   = \"endpoint\"
rights = [\"write\"]
from   = \"sibling:store\"

[[ring]]
name        = \"store\"
role        = \"client\"
protocol    = \"store\"
version_min = 1
version     = 1
entries     = 16
payload     = \"inline\"
to          = \"store\"

[restart]
policy = \"never\"

[reservation]
class        = \"soft\"
memory_bytes = 65536
";

    fn findings(text: &str) -> Vec<String> {
        match check("user/example/manifest.toml", text) {
            Ok(m) => panic!("expected a refusal, got {m:?}"),
            Err(f) => f,
        }
    }

    /// [`SOUND`] with its first occurrence of `from` replaced by `to`.
    fn edit(from: &str, to: &str) -> String {
        assert!(SOUND.contains(from), "fixture does not contain {from:?}");
        SOUND.replacen(from, to, 1)
    }

    fn refused_for(text: &str, needle: &str) {
        let f = findings(text);
        assert!(
            f.iter().any(|line| line.contains(needle)),
            "expected a finding containing {needle:?}, got:\n{}",
            f.join("\n")
        );
    }

    #[test]
    fn the_sound_fixture_passes() {
        let m = check("user/example/manifest.toml", SOUND)
            .unwrap_or_else(|f| panic!("{}", f.join("\n")));
        assert_eq!(m.name, "example");
        assert_eq!(m.domain, "shared");
        assert_eq!(m.capabilities, 2);
        assert_eq!(m.rings, 1);
        assert_eq!(m.restart, "never");
        assert!(!m.image_is_hash());
    }

    #[test]
    fn a_missing_domain_is_refused() {
        // RFC 0005 rule 1: required, no default.
        refused_for(&edit("domain = \"shared\"\n", ""), "`domain` is required");
    }

    #[test]
    fn an_unknown_domain_is_refused() {
        // The brief's working names were `trusted` and `confined`; the RFC
        // landed with `shared` and `private`, and a value outside the table is
        // refused rather than mapped.
        refused_for(
            &edit("domain = \"shared\"", "domain = \"confined\""),
            "not one of \"shared\", \"private\", \"hostile\"",
        );
    }

    #[test]
    fn an_unknown_field_is_refused() {
        refused_for(
            &edit("domain = \"shared\"\n", "domain = \"shared\"\nauthor = \"x\"\n"),
            "`author` is not a field",
        );
    }

    #[test]
    fn an_unknown_table_is_refused() {
        refused_for(&format!("{SOUND}\n[telemetry]\nrate = 1\n"), "`[telemetry]` is not a table");
    }

    #[test]
    fn a_later_schema_is_refused() {
        refused_for(&edit("schema = 1", "schema = 2"), "knows schema 1");
    }

    #[test]
    fn a_third_party_image_may_not_be_shared() {
        // RFC 0005 rule 4, the one line of this RFC a grep can enforce.
        refused_for(
            &edit("image  = \"user/example\"", "image  = \"third_party/gpu/shim\""),
            "rule 4",
        );
        // And the same image in `private` is fine: the rule is about `shared`.
        let text = edit("image  = \"user/example\"", "image  = \"third_party/gpu/shim\"").replacen(
            "domain = \"shared\"",
            "domain = \"private\"",
            1,
        );
        check("user/gpu/manifest.toml", &text).unwrap_or_else(|f| panic!("{}", f.join("\n")));
    }

    #[test]
    fn an_image_by_hash_is_hostile_or_refused() {
        let hash = format!("sha256:{}", "ab".repeat(32));
        refused_for(&edit("image  = \"user/example\"", &format!("image  = \"{hash}\"")), "hostile");
        let text = edit("image  = \"user/example\"", &format!("image  = \"{hash}\"")).replacen(
            "domain = \"shared\"",
            "domain = \"hostile\"",
            1,
        );
        let m =
            check("user/guest/manifest.toml", &text).unwrap_or_else(|f| panic!("{}", f.join("\n")));
        assert!(m.image_is_hash());
        // A hash of the wrong shape is refused before the domain rule is reached.
        refused_for(&edit("image  = \"user/example\"", "image  = \"sha256:abc\""), "sixty-four");
    }

    #[test]
    fn an_image_path_stays_inside_the_tree() {
        refused_for(&edit("image  = \"user/example\"", "image  = \"../elsewhere\""), "`..`");
        refused_for(
            &edit("image  = \"user/example\"", "image  = \"/abs/path\""),
            "relative to the tree root",
        );
        refused_for(
            &edit("image  = \"user/example\"", "image  = \"target/x86_64/init\""),
            "build output",
        );
    }

    #[test]
    fn an_unknown_capability_type_is_refused() {
        // `space` and `bufset` are the short labels `CapType::label` prints;
        // the manifest spells the variant, and a label is refused.
        refused_for(
            &edit("type   = \"frame\"", "type   = \"space\""),
            "`type = \"space\"` is not one of",
        );
    }

    #[test]
    fn an_unknown_right_is_refused_and_so_is_a_repeated_one() {
        refused_for(
            &edit("rights = [\"read\", \"write\"]", "rights = [\"read\", \"own\"]"),
            "\"own\"",
        );
        refused_for(
            &edit("rights = [\"read\", \"write\"]", "rights = [\"read\", \"read\"]"),
            "twice",
        );
    }

    #[test]
    fn execute_on_an_endpoint_is_refused() {
        refused_for(
            &edit(
                "rights = [\"write\"]\nfrom   = \"sibling:store\"",
                "rights = [\"write\", \"execute\"]\nfrom   = \"sibling:store\"",
            ),
            "`execute` on an endpoint",
        );
    }

    #[test]
    fn a_route_is_supervisor_powerbox_or_a_named_sibling() {
        refused_for(
            &edit("from   = \"supervisor\"", "from   = \"kernel\""),
            "not \"supervisor\", \"powerbox\" or \"sibling:<name>\"",
        );
        refused_for(
            &edit("from   = \"supervisor\"", "from   = \"sibling:\""),
            "not \"supervisor\", \"powerbox\" or \"sibling:<name>\"",
        );
        refused_for(
            &edit("from   = \"sibling:store\"", "from   = \"sibling:example\""),
            "not its own sibling",
        );
    }

    #[test]
    fn a_count_belongs_to_its_type() {
        refused_for(&edit("frames = 2", "frames = 0"), "nothing in it");
        refused_for(&edit("frames = 2", "bytes = 8192"), "a frame set is counted in `frames`");
        refused_for(&edit("frames = 2\n", ""), "`frames` is required");
        refused_for(
            &edit("type   = \"frame\"", "type   = \"untyped\""),
            "untyped memory is stated in `bytes`",
        );
        refused_for(
            &edit(
                "type   = \"frame\"\nrights = [\"read\", \"write\"]\nfrom   = \"supervisor\"\nframes = 2",
                "type   = \"untyped\"\nrights = [\"read\", \"write\"]\nfrom   = \"supervisor\"\nbytes = 4000",
            ),
            "multiple of 4096",
        );
    }

    #[test]
    fn the_control_ring_cannot_be_declared() {
        refused_for(
            &edit("name        = \"store\"", "name        = \"control\""),
            "implicit and exactly one",
        );
    }

    #[test]
    fn a_client_ring_names_a_connectable_endpoint() {
        refused_for(
            &edit("to          = \"store\"", "to          = \"nowhere\""),
            "names no capability",
        );
        refused_for(
            &edit("to          = \"store\"", "to          = \"pages\""),
            "names a frame capability",
        );
        refused_for(
            &edit(
                "rights = [\"write\"]\nfrom   = \"sibling:store\"",
                "rights = [\"read\"]\nfrom   = \"sibling:store\"",
            ),
            "without `write`",
        );
        // A server ring names nobody and bounds its clients instead.
        refused_for(
            &edit("role        = \"client\"", "role        = \"server\""),
            "`clients` is required",
        );
        refused_for(
            &edit("role        = \"client\"", "role        = \"server\"\nclients     = 4"),
            "`to` is refused",
        );
    }

    #[test]
    fn a_ring_is_a_power_of_two_with_a_version_range() {
        refused_for(&edit("entries     = 16", "entries     = 24"), "not a power of two");
        refused_for(&edit("entries     = 16", "entries     = 1"), "not a power of two");
        refused_for(&edit("version_min = 1", "version_min = 3"), "above `version");
        refused_for(&edit("version_min = 1", "version_min = 0"), "zero is not a version");
    }

    #[test]
    fn a_feature_is_known_offered_before_required_and_not_the_control_bit() {
        refused_for(
            &edit(
                "payload     = \"inline\"",
                "payload     = \"inline\"\nfeatures    = [\"turbo\"]",
            ),
            "\"turbo\"",
        );
        refused_for(
            &edit(
                "payload     = \"inline\"",
                "payload     = \"inline\"\nfeatures    = [\"control_events\"]",
            ),
            "control ring alone",
        );
        refused_for(
            &edit(
                "payload     = \"inline\"",
                "payload     = \"inline\"\nfeatures_required = [\"admission_control\"]",
            ),
            "does not offer",
        );
        refused_for(
            &edit("payload     = \"inline\"", "payload     = \"shared_virtual\""),
            "without `shared_virtual_memory`",
        );
    }

    #[test]
    fn a_restart_policy_is_declared_and_its_fields_match_it() {
        refused_for(&edit("[restart]\npolicy = \"never\"\n", ""), "no `[restart]`");
        refused_for(
            &edit("policy = \"never\"", "policy = \"on-fault\""),
            "not one of \"never\", \"on_fault\", \"always\"",
        );
        refused_for(
            &edit("policy = \"never\"", "policy = \"never\"\nmax_restarts = 3"),
            "restarts nothing",
        );
        refused_for(
            &edit("policy = \"never\"", "policy = \"on_fault\""),
            "`backoff_first_ms` is required",
        );
        refused_for(
            &edit(
                "policy = \"never\"",
                "policy = \"always\"\nbackoff_first_ms = 100\nbackoff_max_ms = 10\nmax_restarts = 3",
            ),
            "below `backoff_first_ms",
        );
        refused_for(
            &edit(
                "policy = \"never\"",
                "policy = \"always\"\nbackoff_first_ms = 10\nbackoff_max_ms = 100\nmax_restarts = 0",
            ),
            "under another name",
        );
        let sound = edit(
            "policy = \"never\"",
            "policy = \"always\"\nbackoff_first_ms = 10\nbackoff_max_ms = 100\nmax_restarts = 3",
        );
        check("user/example/manifest.toml", &sound).unwrap_or_else(|f| panic!("{}", f.join("\n")));
    }

    #[test]
    fn a_reservation_is_declared_in_its_class_grain() {
        refused_for(
            &edit("[reservation]\nclass        = \"soft\"\nmemory_bytes = 65536\n", ""),
            "no `[reservation]`",
        );
        refused_for(&edit("memory_bytes = 65536", "memory_bytes = 65000"), "multiple of 4096");
        refused_for(
            &edit("class        = \"soft\"", "class        = \"soft\"\ncores        = 1"),
            "soft class has no core",
        );
        refused_for(
            &edit("class        = \"soft\"", "class        = \"hard\""),
            "multiple of 2097152",
        );
        let hard = edit(
            "class        = \"soft\"\nmemory_bytes = 65536",
            "class        = \"hard\"\nmemory_bytes = 2097152\ncores = 1\ncpu_period_ns = 1000000\ncpu_budget_ns = 2000000",
        );
        refused_for(&hard, "not between 1 and the period");
        let hard = hard.replacen("cpu_budget_ns = 2000000", "cpu_budget_ns = 200000", 1);
        check("user/example/manifest.toml", &hard).unwrap_or_else(|f| panic!("{}", f.join("\n")));
    }

    #[test]
    fn the_syntax_is_the_subset_and_nothing_else() {
        refused_for(&edit("name   = \"example\"", "name   = \"\"\"example\"\"\""), "no quote");
        refused_for(&edit("name   = \"example\"", "name   = \"ex\\nample\""), "backslash");
        refused_for(&edit("schema = 1", "schema = -1"), "signed");
        refused_for(&edit("schema = 1", "schema = 1\nschema = 1"), "appears twice");
        refused_for(&edit("[restart]", "[restart]\n[restart]"), "appears twice");
        refused_for(
            &edit("memory_bytes = 65536", "memory_bytes = { min = 65536 }"),
            "inline tables",
        );
        refused_for(&edit("name   = \"example\"", "component.name = \"example\""), "dotted");
        refused_for(
            &edit("rights = [\"read\", \"write\"]", "rights = [\"read\",\n  \"write\"]"),
            "same line",
        );
        refused_for(&edit("memory_bytes = 65536", "memory_bytes = 65_536_"), "between digits");
        refused_for(&edit("schema = 1", "just some words"), "`key = value`");
        // Underscores between digits are TOML and are read.
        let m = check(
            "user/example/manifest.toml",
            &edit("memory_bytes = 65536", "memory_bytes = 65_536"),
        )
        .unwrap();
        assert_eq!(m.name, "example");
    }

    #[test]
    fn the_spdx_header_is_the_first_line() {
        refused_for(&SOUND.replacen(SPDX, "# a manifest", 1), "first line");
    }

    #[test]
    fn a_syntax_error_stops_before_the_fields_are_judged() {
        // Otherwise a file with one broken line reports every field after it
        // as missing, which is noise wearing a finding's clothes.
        let f = findings(&edit("schema = 1", "schema = 1\n[[capability"));
        assert_eq!(f.len(), 1, "{f:?}");
        assert!(f[0].contains("array header"));
    }

    #[test]
    fn a_manifest_may_declare_no_capabilities_and_no_rings() {
        // `init` today holds three grants and speaks on nothing; a manifest for
        // it has empty lists and is complete.
        let text = "\
# SPDX-License-Identifier: Apache-2.0 OR MIT
schema = 1
name   = \"init\"
image  = \"user/init\"
domain = \"shared\"

[restart]
policy = \"never\"

[reservation]
class        = \"soft\"
memory_bytes = 8192
";
        let m =
            check("user/init/manifest.toml", text).unwrap_or_else(|f| panic!("{}", f.join("\n")));
        assert_eq!((m.capabilities, m.rings), (0, 0));
    }

    /// Every closed vocabulary that mirrors `abi/` is read out of `abi/`'s
    /// source and compared, because a table that nothing checks is a table
    /// that is wrong within a milestone — and `CapType` grew a variant while
    /// this module was being written.
    #[test]
    fn the_type_table_matches_the_abi() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf();
        let cap = std::fs::read_to_string(root.join("abi/src/cap.rs")).expect("abi/src/cap.rs");
        let lib = std::fs::read_to_string(root.join("abi/src/lib.rs")).expect("abi/src/lib.rs");

        let variants = block(&cap, "pub enum CapType {")
            .iter()
            .filter_map(|line| {
                let (name, rest) = line.trim().split_once(' ')?;
                (rest.starts_with("= ") && name.bytes().all(|b| b.is_ascii_alphanumeric()))
                    .then(|| snake(name))
            })
            .collect::<Vec<_>>();
        assert_eq!(variants, CAP_TYPES, "CAP_TYPES has drifted from abi::cap::CapType");

        let rights = consts(&block(&cap, "pub mod rights {"), "u8")
            .into_iter()
            .filter(|name| name != "ALL" && name != "NONE")
            .map(|name| name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert_eq!(rights, RIGHTS, "RIGHTS has drifted from abi::cap::rights");

        let features = consts(&block(&lib, "pub mod feature {"), "u64")
            .into_iter()
            .map(|name| name.to_ascii_lowercase())
            .collect::<Vec<_>>();
        assert_eq!(features, FEATURES, "FEATURES has drifted from abi::feature");
    }

    /// The lines between `open` and the first `}` at its indentation.
    fn block(text: &str, open: &str) -> Vec<String> {
        let mut lines = text.lines().skip_while(|l| !l.contains(open));
        let header = lines.next().unwrap_or_else(|| panic!("{open} not found"));
        let indent = header.len() - header.trim_start().len();
        lines
            .take_while(|l| l.trim_end() != format!("{}}}", " ".repeat(indent)))
            .map(str::to_string)
            .collect()
    }

    /// `pub const NAME: ty` inside a block, in order.
    fn consts(lines: &[String], ty: &str) -> Vec<String> {
        lines
            .iter()
            .filter_map(|line| {
                let rest = line.trim().strip_prefix("pub const ")?;
                let (name, rest) = rest.split_once(':')?;
                (rest.trim_start().starts_with(ty)
                    && name.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
                .then(|| name.to_string())
            })
            .collect()
    }

    fn snake(camel: &str) -> String {
        let mut out = String::new();
        for (i, c) in camel.chars().enumerate() {
            if c.is_ascii_uppercase() {
                if i > 0 {
                    out.push('_');
                }
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }
}
