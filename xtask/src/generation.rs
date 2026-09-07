// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `cargo xtask generation`: one expression to one root hash.
//!
//! # What this command is for
//!
//! A generation is what the machine *is*, and this is the only thing that
//! computes one. It reads `user/generation.toml`, refuses it if it is not in
//! canonical form, builds every component file the way `cargo xtask component`
//! does, hashes each one, encodes a record tree with `f_abi::store`, checks it
//! with `f_generation::record`, folds it with `f_generation::fold`, and prints
//! the root and every leaf beside its name. Then it packs the record tree and
//! the component files into one boot module named by the root.
//!
//! # Why the leaves are printed and not only the root
//!
//! Because *two roots differed* is not a finding anybody can act on. `E2-P06`
//! runs this on two runners and diffs the output; a leaf that moved names the
//! input that moved, and a root that moved with every leaf standing still names
//! the compiler. That is the difference between a job that reports a failure and
//! a job that reports a cause, and the printing is the whole of it.
//!
//! # Why a non-canonical source is refused rather than sorted
//!
//! Sorting would be friendlier and it is refused. The claim is *the same
//! expression yields the same root*; a compiler that quietly reorders makes two
//! different sources yield one root, which is a different property, and an
//! author who learns their file was rewritten by reading a hash has learned it
//! at the worst possible moment. So the canonical form is printed as a diff and
//! the author applies it.
//!
//! # Why `--decompile` is the half that matters
//!
//! A tripwire on the *records* catches nothing: no record kind changes when
//! `import`, interpolation and conditionals return to `generation.toml` under
//! the name of convenience, which is exactly how Nix's language would re-enter
//! through the compiler while a tripwire watched the artefact. The tripwire has
//! to be on the source, so [`decompile`] renders a record tree back to canonical
//! source and [`fixpoint`] — wired into `cargo xtask lint` — requires the round
//! trip to be a fixpoint on every `generation.toml` in the tree. A source
//! feature not representable in a record does not survive it.

use std::path::{Path, PathBuf};

use f_abi::manifest::NAME_MAX;
use f_abi::store::{Generation, Leaf, MEMBERS_MAX, ROUTES_MAX, Route, Topology, node};
use f_generation::fold;
use f_generation::record::Tree;

use crate::manifest;
use crate::pack::hex;

/// The schema this compiler knows. Bumped when the *source* grammar changes,
/// which is a different event from the record schema changing, and the two are
/// deliberately not one number: a source file that gains a key the records
/// already carried is a grammar change and not an ABI change.
/// Unit: none — a schema ordinal.
const SCHEMA: u64 = 1;

/// Where a generation's source lives, relative to the repository root.
const SOURCE: &str = "user/generation.toml";

/// One generation, as source says it.
///
/// `Vec` and not a set, because the *order* is the thing being checked: a
/// collection that could not be out of order could not be refused for being out
/// of order, and the refusal is the point.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Source {
    /// What the frame is called here. Its content address is computed, never
    /// written down: a source file carrying the hash of something it does not
    /// contain is wrong from the moment the code changes until somebody
    /// remembers.
    frame: String,
    /// The components, in canonical order.
    components: Vec<String>,
    /// The routes, in canonical order: (component, capability, source), each a
    /// name that must be a component this file names.
    routes: Vec<(String, String, String)>,
}

/// `cargo xtask generation [--decompile]`.
///
/// # Errors
///
/// Anything from a source that does not fit the grammar to a build that did not
/// produce a component file.
pub fn generation(option: Option<&str>) -> Result<(), String> {
    let decompile = match option {
        None => false,
        Some("--decompile") => true,
        // Named rather than accepted, so the flag is not invented twice: it is
        // `E2-P07`'s and it writes a `menuentry` per installed generation into
        // the GRUB fragment `docs/booting-on-hardware.md` documents.
        Some("--install") => {
            return Err("`--install` is E2-P07's and is not built yet. It will write one \
                        `menuentry` per installed generation into the GRUB fragment \
                        docs/booting-on-hardware.md documents."
                .into());
        }
        Some(other) => return Err(format!("unknown option for generation: {other}")),
    };

    let path = crate::root().join(SOURCE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {SOURCE}: {e}"))?;
    let source = read(SOURCE, &text)?;
    canonical(SOURCE, &source)?;

    // The frame, then the components, in that order because the frame's build
    // is the slow one and a source that is wrong should not have paid for it.
    let frame_hash = frame_image()?;
    let mut components = Vec::new();
    for name in &source.components {
        components.push(component_hash(name)?);
    }

    let bytes = compile(&source, frame_hash, &components)?;
    let tree = Tree::check(&bytes).map_err(|why| {
        format!(
            "the record tree this compiler produced is not one it believes: {}\n\n\
             That is a defect in xtask/src/generation.rs and not in {SOURCE}: the source \
             was already checked for canonical order above, so anything left is the \
             encoder disagreeing with the checker about the format they share.",
            why.message()
        )
    })?;

    if decompile {
        print!("{}", decompile_tree(&tree));
        return Ok(());
    }

    println!("generation  {SOURCE}\n");
    println!("  {:<10} {:<14} {}", "frame", source.frame, hex(&frame_hash));
    for (name, hash) in source.components.iter().zip(&components) {
        println!("  {:<10} {:<14} {}", "component", name, hex(hash));
    }
    println!("  {:<10} {:<14} {}", "topology", "", hex(&fold::topology(&tree)));
    let root = fold::root(&tree);
    println!("  {:<10} {:<14} {}", "root", "", hex(&root));

    let module = pack_module(&root, &tree, &source.components)?;
    let size = std::fs::metadata(&module).map(|m| m.len()).unwrap_or(0);
    println!("\n  module     {}  ({size} bytes)", crate::relative(&module));
    assemble(&root, &module)?;
    Ok(())
}

/// Instantiate the module this command just wrote, twice, through the reader
/// that will read it at boot.
///
/// # Why the command that writes a generation also assembles one
///
/// Because `E2-B05`'s exit — *the same root produces a byte-identical topology*
/// — is a property of **this** module and not only of a test's fixture, and the
/// cheapest honest place to demonstrate it on the real one is the command that
/// produces it. `user/assembler/tests/assemble.rs` builds a six-component
/// workload to reach the cases a four-component tree does not have; this reaches
/// the case that matters most, which is the tree somebody is about to boot.
///
/// The bus is empty here, deliberately, and the printed line says so. A build
/// machine is not the machine the generation runs on: what devices are present
/// is discovered at boot and is not in the root, so an assembly taken here can
/// say *this topology, these routes, this order* and must not pretend to say
/// which card was bound. That is the same line `f-assembler` draws between a
/// declared property and a discovered address.
fn assemble(root: &[u8; 32], module: &Path) -> Result<(), String> {
    let bytes =
        std::fs::read(module).map_err(|e| format!("reading {}: {e}", crate::relative(module)))?;
    let mut digests = Vec::new();
    let mut renderings = Vec::new();
    for _ in 0..2 {
        let assembly = f_assembler::Assembly::instantiate(root, &bytes).map_err(|why| {
            format!(
                "the boot module this command packed is not a topology f-assembler will \
                 instantiate: {} ({why:?})",
                why.message()
            )
        })?;
        digests.push(hex(&f_assembler::render::digest(&assembly)));
        renderings.push(f_assembler::render::topology(&assembly));
    }

    if renderings[0] != renderings[1] {
        return Err(
            "two instantiations of one root rendered to different bytes, which is E2-B05's exit \
             failing on this tree's own generation rather than on a fixture"
                .into(),
        );
    }
    println!(
        "  topology   {}  (instantiated twice, byte-identical over {} bytes; no bus, so nothing \
         is bound here)",
        digests[0],
        renderings[0].len()
    );
    Ok(())
}

/// Read the source through the reader `manifest.rs` already has.
///
/// One parser, not two. Every key is known or refused — R04 — which is the half
/// that keeps `import` and interpolation from arriving as keys nobody looks at.
fn read(rel: &str, text: &str) -> Result<Source, String> {
    let doc = manifest::parse(rel, text).map_err(|findings| findings.join("\n"))?;

    let mut findings = Vec::new();
    let mut refuse = |why: String| findings.push(format!("  {rel}  {why}"));

    for key in doc.top.keys() {
        if !matches!(key.as_str(), "schema" | "frame" | "component") {
            refuse(format!("`{key}` is not a key this grammar knows"));
        }
    }
    for name in doc.tables.keys() {
        refuse(format!("`[{name}]` is not a table this grammar knows"));
    }
    for name in doc.arrays.keys() {
        if name != "route" {
            refuse(format!("`[[{name}]]` is not a table this grammar knows"));
        }
    }

    let schema = match doc.top.get("schema").map(|entry| &entry.value) {
        Some(manifest::Value::Int(value)) => *value,
        Some(other) => {
            refuse(format!("`schema` is {}, and it is an integer", kind(other)));
            SCHEMA
        }
        None => {
            refuse("`schema` is missing, and it is the first thing a reader looks at".into());
            SCHEMA
        }
    };
    if schema != SCHEMA {
        refuse(format!("`schema = {schema}` is not a grammar this build knows; it knows {SCHEMA}"));
    }

    let frame = string(&doc.top, "frame", &mut refuse).unwrap_or_default();
    if !frame.is_empty() && !manifest::is_name(&frame) {
        refuse(format!("`frame = \"{frame}\"` is not a name: `[a-z0-9-]`, at most {NAME_MAX}"));
    }

    let components = match doc.top.get("component").map(|entry| &entry.value) {
        Some(manifest::Value::List(items)) => items.clone(),
        Some(other) => {
            refuse(format!("`component` is {}, and it is a list of names", kind(other)));
            Vec::new()
        }
        None => {
            refuse("`component` is missing; a generation with no components is not one".into());
            Vec::new()
        }
    };
    for name in &components {
        if !manifest::is_name(name) {
            refuse(format!(
                "`{name}` is not a name: `[a-z0-9-]`, at most {NAME_MAX}, no edge hyphen"
            ));
        }
    }
    if components.len() > MEMBERS_MAX {
        refuse(format!("{} components; a topology names at most {MEMBERS_MAX}", components.len()));
    }

    let mut routes = Vec::new();
    for (line, table) in doc.arrays.get("route").into_iter().flatten() {
        for key in table.keys() {
            if !matches!(key.as_str(), "component" | "capability" | "source") {
                refuse(format!("line {line}: `{key}` is not a key a route has"));
            }
        }
        let at = |key: &str, refuse: &mut dyn FnMut(String)| {
            let value = match table.get(key).map(|entry| &entry.value) {
                Some(manifest::Value::Str(value)) => value.clone(),
                Some(other) => {
                    refuse(format!("line {line}: `{key}` is {}, and it is a name", kind(other)));
                    String::new()
                }
                None => {
                    refuse(format!("line {line}: a route states `{key}`"));
                    String::new()
                }
            };
            if !value.is_empty() && !manifest::is_name(&value) {
                refuse(format!("line {line}: `{key} = \"{value}\"` is not a name"));
            }
            value
        };
        let component = at("component", &mut refuse);
        let capability = at("capability", &mut refuse);
        let source = at("source", &mut refuse);
        routes.push((component, capability, source));
    }
    if routes.len() > ROUTES_MAX {
        refuse(format!("{} routes; a topology declares at most {ROUTES_MAX}", routes.len()));
    }

    // Indices are resolved here so that a dangling name is a *source* finding
    // naming the name, rather than a record-tree refusal naming a number the
    // author never wrote.
    for (component, capability, from) in &routes {
        for (role, name) in [("component", component), ("source", from)] {
            if !name.is_empty() && !components.contains(name) {
                refuse(format!(
                    "route `{capability}`: `{role} = \"{name}\"` is not a component this file names"
                ));
            }
        }
    }

    if findings.is_empty() {
        return Ok(Source { frame, components, routes });
    }
    Err(format!(
        "{} problem(s) in {rel}:\n{}\n\n\
         The grammar is closed: a key this build does not know is refused rather than \
         ignored — R04 — because a key that is ignored is a statement the author believes \
         the machine read. `generation/src/lib.rs` says what the expression deliberately \
         is not.",
        findings.len(),
        findings.join("\n")
    ))
}

/// A word for a value of the wrong shape.
fn kind(value: &manifest::Value) -> &'static str {
    match value {
        manifest::Value::Str(_) => "a string",
        manifest::Value::Int(_) => "an integer",
        manifest::Value::Bool(_) => "a boolean",
        manifest::Value::List(_) => "a list",
    }
}

/// One string-valued key, or a finding.
fn string(table: &manifest::Table, key: &str, refuse: &mut dyn FnMut(String)) -> Option<String> {
    match table.get(key).map(|entry| &entry.value) {
        Some(manifest::Value::Str(value)) => Some(value.clone()),
        Some(other) => {
            refuse(format!("`{key}` is {}, and it is a name", kind(other)));
            None
        }
        None => {
            refuse(format!("`{key}` is missing"));
            None
        }
    }
}

/// Refuse a source that is not in canonical form, printing the canonical form
/// as a diff.
///
/// The comparison is of the *rendered* forms, so whitespace, comments and the
/// order of the three top-level keys are not part of canonicity — they carry no
/// record, and a canonical form that also fixed a column would make a formatting
/// change a hash change, which it is not. What is part of canonicity is the
/// order of the components, the order of the routes, and the absence of
/// duplicates.
fn canonical(rel: &str, source: &Source) -> Result<(), String> {
    let sorted = sorted(source)?;
    if sorted == *source {
        return Ok(());
    }
    let theirs = render(source);
    let ours = render(&sorted);
    Err(format!(
        "{rel} is not in canonical form.\n\n{}\n\n\
         Components sort bytewise on the zero-padded 32-byte name; routes sort on\n\
         (component, capability). It is refused rather than sorted for you, because\n\
         *the same expression yields the same root* is the claim, and a compiler that\n\
         quietly reorders makes two different sources yield one root instead — and an\n\
         author who learns their file was rewritten by reading a hash has learned it\n\
         too late. Apply the right-hand side.",
        diff(&theirs, &ours)
    ))
}

/// The same source, in canonical order. Duplicates are refused here rather than
/// sorted away, because two entries naming one thing are two beliefs and a
/// compiler has no business picking one.
fn sorted(source: &Source) -> Result<Source, String> {
    let mut components = source.components.clone();
    components.sort_by_key(|name| padded(name));
    for pair in components.windows(2) {
        if pair[0] == pair[1] {
            return Err(format!("`{}` is named twice in the component list", pair[0]));
        }
    }

    let index = |name: &str| components.iter().position(|other| other == name).unwrap_or(0);
    let mut routes = source.routes.clone();
    routes.sort_by_key(|(component, capability, _)| (index(component), padded(capability)));
    for pair in routes.windows(2) {
        if pair[0].0 == pair[1].0 && pair[0].1 == pair[1].1 {
            return Err(format!(
                "`{}` is routed to `{}` twice; a component needs a capability from one source",
                pair[0].0, pair[0].1
            ));
        }
    }

    Ok(Source { frame: source.frame.clone(), components, routes })
}

/// A name as the record holds it: NUL-padded to thirty-two bytes.
///
/// The padded form is the normative sort key, because it is what the record
/// holds and what the fold hashes. On an alphabet with no NUL the padded and
/// trimmed orders agree; saying which is normative costs a line and settles the
/// argument before anybody has it.
fn padded(name: &str) -> [u8; NAME_MAX] {
    let mut out = [0u8; NAME_MAX];
    let bytes = name.as_bytes();
    let end = bytes.len().min(NAME_MAX);
    out[..end].copy_from_slice(&bytes[..end]);
    out
}

/// The canonical source form of a generation.
///
/// One renderer, used by the diff above and by [`decompile`] below, so that
/// *canonical* means one thing. The key order is chosen and not alphabetical:
/// `schema` first because a reader who cannot parse the rest still needs it, and
/// in a route the sort key first, so the file reads in the order it is sorted in.
fn render(source: &Source) -> String {
    let mut out = String::new();
    out.push_str(&format!("schema = {SCHEMA}\n"));
    out.push_str(&format!("frame = \"{}\"\n", source.frame));
    let names: Vec<String> = source.components.iter().map(|n| format!("\"{n}\"")).collect();
    out.push_str(&format!("component = [{}]\n", names.join(", ")));
    for (component, capability, from) in &source.routes {
        out.push_str("\n[[route]]\n");
        out.push_str(&format!("component = \"{component}\"\n"));
        out.push_str(&format!("capability = \"{capability}\"\n"));
        out.push_str(&format!("source = \"{from}\"\n"));
    }
    out
}

/// A line diff, `-` for what is there and `+` for what should be.
///
/// Line-by-line and not a minimal edit script, because the two sides are two
/// renderings of one file and the useful reading is *this line, that line*
/// rather than the shortest way between them.
fn diff(theirs: &str, ours: &str) -> String {
    let left: Vec<&str> = theirs.lines().collect();
    let right: Vec<&str> = ours.lines().collect();
    let mut out = Vec::new();
    for n in 0..left.len().max(right.len()) {
        match (left.get(n), right.get(n)) {
            (Some(a), Some(b)) if a == b => out.push(format!("   {a}")),
            (a, b) => {
                if let Some(a) = a {
                    out.push(format!("  -{a}"));
                }
                if let Some(b) = b {
                    out.push(format!("  +{b}"));
                }
            }
        }
    }
    out.join("\n")
}

/// Encode a record tree with `f_abi::store`, and check it with `f-generation`.
///
/// The same codec the machine decodes with, which is the whole of *one
/// implementation*: an encoder written here against a format described
/// elsewhere is the second reader the spec spends a page refusing to have.
fn compile(source: &Source, frame: [u8; 32], components: &[[u8; 32]]) -> Result<Vec<u8>, String> {
    let head = Topology {
        members: u16::try_from(source.components.len()).map_err(|_| "too many components")?,
        routes: u16::try_from(source.routes.len()).map_err(|_| "too many routes")?,
    };
    let index =
        |name: &str| source.components.iter().position(|other| other == name).unwrap_or(0) as u16;

    let mut out = Vec::new();
    out.extend_from_slice(&Generation::to_bytes());
    out.extend_from_slice(
        &Leaf { kind: node::BYTES, hash: frame, name: padded(&source.frame) }.to_bytes(),
    );
    out.extend_from_slice(&head.to_bytes());
    for (component, capability, from) in &source.routes {
        let route = Route {
            component: index(component),
            source: index(from),
            capability: padded(capability),
        };
        out.extend_from_slice(&route.to_bytes());
    }
    for (name, hash) in source.components.iter().zip(components) {
        let leaf = Leaf { kind: node::COMPONENT, hash: *hash, name: padded(name) };
        out.extend_from_slice(&leaf.to_bytes());
    }
    Ok(out)
}

/// A record tree, back to canonical source.
fn decompile_tree(tree: &Tree<'_>) -> String {
    render(&from_tree(tree))
}

/// The source a record tree describes.
fn from_tree(tree: &Tree<'_>) -> Source {
    let label = |padded: &[u8; NAME_MAX]| {
        let end = padded.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
        String::from_utf8_lossy(&padded[..end]).into_owned()
    };
    let components: Vec<String> =
        (0..tree.members()).filter_map(|n| tree.member(n)).map(|leaf| label(&leaf.name)).collect();
    let name_of = |index: u16| components.get(index as usize).cloned().unwrap_or_default();
    let routes = (0..tree.routes())
        .filter_map(|n| tree.route(n))
        .map(|route| (name_of(route.component), label(&route.capability), name_of(route.source)))
        .collect();
    Source { frame: label(&tree.frame().name), components, routes }
}

/// The frame image's content address.
///
/// The ELF32 container and not the ELF64 one, because the ELF32 is what a loader
/// is handed and *what are you running* is a question about the thing that ran.
/// `to_elf32` rewrites headers only, so the two differ by a container and not by
/// a byte of code — which is exactly why naming which one is hashed is worth a
/// sentence rather than being left to whoever reads the path.
fn frame_image() -> Result<[u8; 32], String> {
    crate::build()?;
    let path = crate::kernel_elf32();
    let bytes =
        std::fs::read(&path).map_err(|e| format!("reading {}: {e}", crate::relative(&path)))?;
    Ok(f_hash::sha256(&bytes))
}

/// One component file's content address.
///
/// Built the way `cargo xtask component` builds it — the same manifest checker,
/// the same linker, one blob of record and image — because a leaf naming bytes
/// this tree did not produce would be a leaf naming nothing.
fn component_hash(name: &str) -> Result<[u8; 32], String> {
    if !crate::COMPONENTS.contains(&name) {
        return Err(format!(
            "`{name}` is not a component this tree builds. `COMPONENTS` in \
             xtask/src/main.rs is what decides what is built; a generation naming \
             something else would carry a leaf for a component file nothing produces."
        ));
    }
    let (path, _, _) = crate::component_image(name)?;
    let bytes =
        std::fs::read(&path).map_err(|e| format!("reading {}: {e}", crate::relative(&path)))?;
    Ok(f_hash::sha256(&bytes))
}

/// Pack the record tree and the component files into one boot module.
///
/// # Why the layout is not written here any more
///
/// It was, with a comment naming the condition that should move it: the day
/// something read it. `E2-B05`'s assembler is that reader, so the layout is
/// `f_abi::boot::Module` now and this function writes what that type reads —
/// one definition rather than two that agree until they do not. Everything the
/// old comment argued about *why a module at all* is in that type's own
/// documentation, which is where a reader who has just decoded one will look.
///
/// The one thing that stays here is the file's *name*: `<root>.fcm` in the
/// build directory. A module is named by the fold over what is inside it, which
/// is the whole of how `f.root=` selects one.
fn pack_module(root: &[u8; 32], tree: &Tree<'_>, names: &[String]) -> Result<PathBuf, String> {
    let mut files = Vec::new();
    for name in names {
        let path = crate::component_path(name);
        files.push(
            std::fs::read(&path).map_err(|e| format!("reading {}: {e}", crate::relative(&path)))?,
        );
    }

    if files.len() > f_abi::boot::MODULE_FILES_MAX {
        return Err(format!(
            "{} component files is more than the {} f_abi::boot::Module will decode",
            files.len(),
            f_abi::boot::MODULE_FILES_MAX
        ));
    }

    let mut out = Vec::new();
    out.extend_from_slice(&f_abi::boot::MODULE_MAGIC.to_le_bytes());
    out.extend_from_slice(
        &u32::try_from(tree.bytes().len()).map_err(|_| "a vast tree")?.to_le_bytes(),
    );
    out.extend_from_slice(&u32::try_from(files.len()).map_err(|_| "too many files")?.to_le_bytes());
    for file in &files {
        out.extend_from_slice(
            &u32::try_from(file.len()).map_err(|_| "a vast component")?.to_le_bytes(),
        );
    }
    out.extend_from_slice(tree.bytes());
    for file in &files {
        out.extend_from_slice(file);
    }

    // The writer believing its own output, through the reader that will believe
    // it at boot. Cheap, and it is the check that keeps `xtask` from shipping a
    // module the assembler refuses — which is a failure that would otherwise be
    // found by a boot rather than by the command that produced the bytes.
    f_abi::boot::Module::read(&out).map_err(|why| {
        format!(
            "the boot module this command packed is not one f_abi::boot::Module believes \
             ({why:#x}). That is a defect in xtask/src/generation.rs and not in {SOURCE}: the \
             writer here and the reader there share one layout and have disagreed about it."
        )
    })?;

    let dir = crate::target_dir().join("generation");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.fcm", hex(root)));
    std::fs::write(&path, &out).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(path)
}

/// The round-trip fixpoint, over every `generation.toml` in the tree.
///
/// # What this catches that nothing else does
///
/// A source feature that no record can carry. Nothing about the *records*
/// changes when `import`, interpolation or a conditional returns to
/// `generation.toml` — the records are downstream of the compiler — so a
/// tripwire on the artefact watches the wrong thing entirely, which is precisely
/// how Nix's language would re-enter here under the name of convenience. The
/// tripwire is on the source: compile it, decompile the records, and require the
/// result to be the source. Anything the records cannot express is gone by then.
///
/// The content addresses are left zero here, and that is not a shortcut: the
/// round trip is about what the *shape* of a source can express, a hash is not a
/// source feature, and building four components and a kernel inside a lint would
/// make `cargo xtask lint` cost a full build. `cargo xtask generation` is what
/// runs it with the real ones.
///
/// # Errors
///
/// A source that is not canonical, or one whose round trip is not a fixpoint.
pub fn fixpoint() -> Result<(), String> {
    let mut checked = 0;
    for path in sources()? {
        let rel = crate::relative(&path);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        let source = read(&rel, &text)?;
        canonical(&rel, &source)?;

        let zero = vec![[0u8; 32]; source.components.len()];
        let bytes = compile(&source, [0; 32], &zero)?;
        let tree = Tree::check(&bytes)
            .map_err(|why| format!("{rel}: the record tree is refused: {}", why.message()))?;

        let rendered = decompile_tree(&tree);
        let again = read(&rel, &rendered)?;
        if again != source {
            return Err(format!(
                "{rel} does not survive the round trip: what it says and what its records \
                 can say are not the same thing.\n\n{}\n\n\
                 That is the check's whole purpose. A source feature no record kind carries \
                 — an `import`, an interpolation, a conditional — is gone by the time the \
                 records are decompiled, and this is where it is noticed.",
                diff(&render(&source), &rendered)
            ));
        }
        if render(&again) != rendered {
            return Err(format!("{rel}: the canonical renderer is not idempotent"));
        }
        checked += 1;
    }

    println!("lint-generations: ok  ({checked} generation(s) round-trip to a fixpoint)");
    Ok(())
}

/// Every `generation.toml` in the tree.
///
/// Walked rather than listed, for the reason `PORTABILITY` gives about lists
/// written beside the thing they describe: a second generation added next to the
/// first would join a hand-written list only if somebody remembered, and the
/// check that is skipped silently is the check that is not there.
fn sources() -> Result<Vec<PathBuf>, String> {
    let mut found = Vec::new();
    walk(&crate::root(), &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(dir: &Path, found: &mut Vec<PathBuf>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("reading {}: {e}", dir.display()))?;
    // Collected and sorted rather than used as `read_dir` hands them over:
    // directory order is a filesystem's business and differs between two
    // machines with the same files, which is exactly the difference `E2-P06`
    // exists to find and would then find here instead.
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("reading {}: {e}", dir.display()))?.path());
    }
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if path.is_dir() {
            if matches!(name, "target" | ".git" | "third_party") {
                continue;
            }
            walk(&path, found)?;
        } else if name == "generation.toml" {
            found.push(path);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CANONICAL: &str = "\
schema = 1
frame = \"kernel\"
component = [\"store\", \"virtio-blk\", \"virtio-gpu\", \"virtio-net\"]

[[route]]
component = \"store\"
capability = \"block\"
source = \"virtio-blk\"

[[route]]
component = \"store\"
capability = \"net\"
source = \"virtio-net\"
";

    #[test]
    fn the_renderer_is_a_fixpoint_on_its_own_output() {
        let source = read("fixture", CANONICAL).expect("the fixture parses");
        assert_eq!(render(&source), CANONICAL);
        assert!(canonical("fixture", &source).is_ok());
    }

    #[test]
    fn a_tree_decompiles_to_the_source_it_was_compiled_from() {
        let source = read("fixture", CANONICAL).expect("the fixture parses");
        let zero = vec![[0u8; 32]; source.components.len()];
        let bytes = compile(&source, [0; 32], &zero).expect("the fixture compiles");
        let tree = Tree::check(&bytes).expect("the fixture is canonical");
        assert_eq!(decompile_tree(&tree), CANONICAL);
    }

    #[test]
    fn a_source_out_of_order_is_refused_with_the_canonical_form() {
        let shuffled = CANONICAL.replace(
            "[\"store\", \"virtio-blk\", \"virtio-gpu\", \"virtio-net\"]",
            "[\"virtio-net\", \"store\", \"virtio-gpu\", \"virtio-blk\"]",
        );
        let source = read("fixture", &shuffled).expect("it parses; it is only out of order");
        let refusal = canonical("fixture", &source).expect_err("a shuffled source was accepted");
        assert!(refusal.contains("not in canonical form"), "{refusal}");
        // The canonical form is printed, which is what makes the refusal
        // actionable rather than merely correct.
        assert!(refusal.contains("+component = [\"store\""), "{refusal}");
    }

    #[test]
    fn routes_out_of_order_are_refused_too() {
        let swapped = CANONICAL.replace("\"block\"", "\"zebra\"");
        let source = read("fixture", &swapped).expect("it parses");
        assert!(canonical("fixture", &source).is_err(), "routes were not ordered");
    }

    #[test]
    fn a_duplicate_is_refused_rather_than_merged() {
        let twice = CANONICAL.replace("\"virtio-gpu\", ", "\"virtio-blk\", ");
        let source = read("fixture", &twice).expect("it parses");
        let refusal = canonical("fixture", &source).expect_err("a duplicate was accepted");
        assert!(refusal.contains("named twice"), "{refusal}");
    }

    /// R04, on the grammar. Each of these is a way the language grows back.
    #[test]
    fn a_key_this_grammar_does_not_know_is_refused_and_never_ignored() {
        for line in ["import = \"other.toml\"\n", "[[include]]\nfile = \"x\"\n"] {
            let text = format!("{CANONICAL}{line}");
            let refusal = read("fixture", &text).expect_err("an unknown key was ignored");
            assert!(refusal.contains("not a"), "{refusal}");
        }
    }

    #[test]
    fn a_route_naming_a_component_this_file_does_not_is_refused_by_name() {
        let text = CANONICAL.replace("source = \"virtio-net\"", "source = \"virtio-scsi\"");
        let refusal = read("fixture", &text).expect_err("a dangling route was accepted");
        assert!(refusal.contains("virtio-scsi"), "{refusal}");
    }

    #[test]
    fn the_repositorys_own_generation_is_canonical_and_round_trips() {
        let path = crate::root().join(SOURCE);
        let text = std::fs::read_to_string(&path).expect("user/generation.toml is committed");
        let source = read(SOURCE, &text).expect("it fits the grammar");
        canonical(SOURCE, &source).expect("it is canonical");

        let zero = vec![[0u8; 32]; source.components.len()];
        let bytes = compile(&source, [0; 32], &zero).expect("it compiles");
        let tree = Tree::check(&bytes).expect("the tree is believed");
        assert_eq!(read(SOURCE, &decompile_tree(&tree)).expect("it re-reads"), source);
    }
}
