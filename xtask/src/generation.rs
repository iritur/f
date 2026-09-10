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

/// `cargo xtask generation [--decompile | --emit DIR | --compare A B | --elsewhere | --mutate]`.
///
/// # Errors
///
/// Anything from a source that does not fit the grammar to a build that did not
/// produce a component file.
pub fn generation(args: &[String]) -> Result<(), String> {
    let option = args.first().map(String::as_str);
    let decompile = match option {
        None => false,
        Some("--decompile") => true,
        // `E2-P07`'s. One `menuentry` per installed generation, written into the
        // GRUB fragment `docs/booting-on-hardware.md` documents — the loader's
        // own menu, configuration this repository already writes, nothing
        // imported and no boot-time reader of the on-disk format.
        Some("--install") => return install(args.get(1).map(String::as_str)),
        // E2-P06's four. Each is below, beside the argument for its shape.
        Some("--emit") => return emit(args.get(1..).unwrap_or_default()),
        Some("--compare") => {
            let (Some(left), Some(right)) = (args.get(1), args.get(2)) else {
                return Err("`--compare` takes two directories, each written by `cargo xtask \
                            generation --emit`. The weekly job hands it one per runner."
                    .into());
            };
            return compare(Path::new(left), Path::new(right));
        }
        Some("--elsewhere") => return elsewhere(&[]),
        Some("--mutate") => return mutate(),
        Some(other) => return Err(format!("unknown option for generation: {other}")),
    };

    let built = compile_here(&[])?;
    let tree = checked(&built.bytes)?;

    if decompile {
        print!("{}", decompile_tree(&tree));
        return Ok(());
    }

    print_tree(&built, &tree);
    let root = fold::root(&tree);

    let module = pack_module(&root, &tree, &built.source.components)?;
    let size = std::fs::metadata(&module).map(|m| m.len()).unwrap_or(0);
    println!("\n  module     {}  ({size} bytes)", crate::relative(&module));
    assemble(&root, &module)?;
    Ok(())
}

/// Read the source, build everything it names, and encode the record tree.
///
/// `defects` are kernel features, and the only caller that passes any is
/// [`mutate`]. They reach the *frame* build and nothing else, which is what
/// makes a defect built this way land in exactly one leaf — so a comparison that
/// then names some other leaf is a comparison that is not reading what it thinks
/// it is reading.
fn compile_here(defects: &[&str]) -> Result<Compiled, String> {
    let path = crate::root().join(SOURCE);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {SOURCE}: {e}"))?;
    let source = read(SOURCE, &text)?;
    canonical(SOURCE, &source)?;

    // The frame, then the components, in that order because the frame's build
    // is the slow one and a source that is wrong should not have paid for it.
    let frame = frame_image(defects)?;
    let mut components = Vec::new();
    for name in &source.components {
        components.push(component_hash(name)?);
    }

    let bytes = compile(&source, frame, &components)?;
    Ok(Compiled { source, frame, components, bytes })
}

/// One compilation of `user/generation.toml`: the source it read, the content
/// addresses it built, and the record tree it encoded from them.
///
/// A struct rather than a tuple because the tuple had four members and two of
/// them were `[u8; 32]`-shaped, which is one transposition away from a leaf
/// printed under the wrong name — and the whole value of this command is that
/// the name beside a hash is right.
struct Compiled {
    source: Source,
    /// The frame image's content address.
    frame: [u8; 32],
    /// Every component file's, in the source's canonical order.
    components: Vec<[u8; 32]>,
    /// The encoded record tree, unchecked. [`checked`] is what believes it.
    bytes: Vec<u8>,
}

/// The checker's refusal, with the sentence that says whose defect it is.
fn checked(bytes: &[u8]) -> Result<Tree<'_>, String> {
    Tree::check(bytes).map_err(|why| {
        format!(
            "the record tree this compiler produced is not one it believes: {}\n\n\
             That is a defect in xtask/src/generation.rs and not in {SOURCE}: the source \
             was already checked for canonical order above, so anything left is the \
             encoder disagreeing with the checker about the format they share.",
            why.message()
        )
    })
}

/// The block every mode prints, so that two runs are two comparable logs.
fn print_tree(built: &Compiled, tree: &Tree<'_>) {
    println!("generation  {SOURCE}\n");
    println!("  {:<10} {:<14} {}", "frame", built.source.frame, hex(&built.frame));
    for (name, hash) in built.source.components.iter().zip(&built.components) {
        println!("  {:<10} {:<14} {}", "component", name, hex(hash));
    }
    println!("  {:<10} {:<14} {}", "topology", "", hex(&fold::topology(tree)));
    println!("  {:<10} {:<14} {}", "root", "", hex(&fold::root(tree)));
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

    // `claims/0027`'s second workload. The test beside `f-assembler` reaches the
    // cases a four-component tree does not have; this reaches the tree somebody
    // is about to boot, and the claim compares both. The row names differ from
    // the test's on purpose — they are two measurements of one property over two
    // different topologies, and averaging them into one row would hide whichever
    // of the two moved.
    let distinct: std::collections::BTreeSet<&Vec<u8>> = renderings.iter().collect();
    println!("\n  claims/0027 topology-renderings-per-root, over this tree's own generation");
    for (name, value) in [
        ("distinct_renderings_of_the_packed_module", distinct.len()),
        ("packed_module_instantiations", renderings.len()),
        ("rendered_topology_bytes_on_this_tree", renderings[0].len()),
    ] {
        println!("    {name:<42} {value}");
    }
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

/// The frame image's content address: RFC 0012's two ranges, and not the file.
///
/// # Why this is not the SHA-256 of the ELF
///
/// It was, and the change is `E2-B07`'s. RFC 0012 says two things about this
/// value in one paragraph each, and only one hash satisfies both. It is *the
/// frame hash*, defined there as SHA-256 over `__text_start .. __text_end` then
/// `__rodata_start .. __rodata_end` of the linked image, which the frame
/// recomputes over itself at boot. And it is *a duplicate of a leaf that already
/// sits under `root`*, which is this leaf. A whole-file digest satisfies the
/// second and cannot satisfy the first: a running frame has no access to its own
/// ELF — `.boot` is unmapped after the address-space switch, the section headers
/// were never loaded at all — so a `frame` field spelled that way is a field
/// nothing on the machine can ever check, which is a field nothing checks.
///
/// What the change costs, stated rather than discovered: every root this command
/// has ever printed moves, because the leaf under it does. Nothing in the tree
/// pins one, and `E2-P06`'s two-checkout comparison is unaffected — the path
/// `mutate-path-in-image` compiles in is a string literal and lands in
/// `.rodata`, inside the measured range.
///
/// What it deliberately stops covering is what RFC 0012 deliberately excludes:
/// `.boot`, `.data`, `.got`, `.bss`, `.stacks` and the ELF headers themselves. A
/// change confined to a writable initial value is invisible in this leaf now,
/// where a whole-file hash would have caught it. That is a real narrowing and it
/// is the RFC's, not this function's; the RFC names it as one of its own
/// reversal conditions, and `cargo xtask attest` is where the coverage is
/// demonstrated rather than asserted.
///
/// The ELF64 is read rather than the ELF32, and the old comment's argument for
/// the other choice does not survive the change: it was about which *file* a
/// loader is handed, and the value is no longer a digest of a file. `to_elf32`
/// rewrites headers only, so the two ranges are the same bytes at the same
/// offsets in both; the 64-bit container is the one the linker produced and the
/// one whose symbol table `crate::measure` reads the boundaries out of.
fn frame_image(defects: &[&str]) -> Result<[u8; 32], String> {
    crate::build_with(defects)?;
    let (digest, _) = crate::measure::frame(&crate::kernel_elf64())?;
    Ok(digest)
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

/// The pair, spelled once: `f.root=<64 hex> f.frame=<64 hex>`.
///
/// # Why this is a function and not three `format!`s
///
/// Because the two tokens are one statement and the frame refuses half of it —
/// `kernel/src/measure.rs` answers `HalfADeclaration` to a command line
/// carrying one without the other, on the grounds that half of a two-part
/// statement is a different statement rather than a weaker one. Two callers
/// hold the pair as bytes and compose a command line from it: [`identity`]
/// for every ordinary boot, and `rollback` for a boot that names a generation
/// other than the one just built. [`fragment`] makes the same statement for a
/// `menuentry` and spells it out of the hex it already holds. A caller that
/// composed the pair from scratch is a caller that could compose half of it, and
/// that failure arrives as a red boot with no line naming who wrote it.
///
/// *Reversal:* the day `f.frame=` stops being required beside `f.root=`, which
/// is RFC 0012's own — a measurement taken before the frame runs, so the frame
/// is handed a digest rather than told one.
pub(crate) fn tokens(root: &[u8; 32], frame: &[u8; 32]) -> String {
    format!("{}{} {}{}", f_abi::boot::KEY, hex(root), f_abi::boot::FRAME_KEY, hex(frame))
}

/// What a booting machine is told it is, and the module that makes it true.
///
/// # Why the two travel together
///
/// Because each half is a failure without the other, and the two halves were
/// written by two people who could not see each other. `f.root=` on the command
/// line is `E2-B07`'s: RFC 0012 makes the generation root the answer to *what
/// are you running*, and every boot gets asked. A module the loader offers is
/// `E2-P07`'s: the frame refuses a boot that names a generation no offered
/// module folds to, because a machine that quietly booted a different one is the
/// exact failure a rollback test exists to catch.
///
/// Put the two together and an ordinary boot has to be *handed the generation it
/// is told it is*. It was not, for the length of one merge — every boot in the
/// tree named a root, none was offered a module, and every boot went red on a
/// refusal that was correct. So this hands back both, and [`crate::emulator`]
/// puts the module on the loader's list beside the tokens on the command line.
#[derive(Clone)]
pub(crate) struct Identity {
    /// `f.root=<64 hex> f.frame=<64 hex>`, for the `-append` line.
    /// Unit: none — a command-line fragment.
    pub tokens: String,
    /// The `<root>.fcm` those tokens name, for the loader's module list.
    /// Unit: none — a path.
    pub module: PathBuf,
}

/// What a booting machine is told it is, and the boot module it is handed.
///
/// # Why a boot gets both tokens and not just the first
///
/// Because a selection is not a statement about the frame. `f.root=` says *be
/// this generation*; the frame then has thirty-two bytes it cannot check —
/// checking them means folding a record tree, which is `f-assembler`'s job and
/// RFC 0066 puts that above the frame with no caller. `f.frame=` is the half the
/// frame *can* check, and RFC 0012 is explicit that checking it is the whole of
/// what a self-measurement buys: the field it compares is the same field a swap
/// compares to decide whether it needs a reboot.
///
/// # Why this is memoised
///
/// Because `emulator` is on the path of every boot in this tree — `user` runs
/// seven, `cap` eight — and a generation compiled per boot would run the
/// component builder once per boot for a value that cannot have changed between
/// two boots of one command. The key is the feature list, because a defect
/// reaches the frame leaf and two boots with different defects are two
/// generations. A `BTreeMap` rather than the obvious other thing, because
/// `xtask` is checked by the determinism lint it implements.
///
/// # Errors
///
/// Anything [`pack`] can fail with.
pub(crate) fn identity(features: &[&str]) -> Result<Identity, String> {
    static MEMO: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, Identity>>,
    > = std::sync::OnceLock::new();
    let memo = MEMO.get_or_init(Default::default);
    let key = features.join(",");
    if let Ok(seen) = memo.lock()
        && let Some(identity) = seen.get(&key)
    {
        return Ok(identity.clone());
    }

    let packed = pack(features)?;
    let identity = Identity {
        // The frame leaf, and not a second measurement taken here. RFC 0012's
        // `frame` field is *a duplicate of a leaf that already sits under
        // `root`*, and a boot told a number this command computed on the side
        // would be a boot checking something the root does not contain.
        tokens: tokens(&packed.root, &packed.frame),
        module: packed.module,
    };

    if let Ok(mut seen) = memo.lock() {
        seen.insert(key, identity.clone());
    }
    Ok(identity)
}

/// Everything one generation is, for a caller that has to hold two of them at
/// once and say how they differ.
///
/// `E2-P07` is that caller and there is no second one yet. It exists as a type
/// rather than as four return values because three of its four fields are
/// `[u8; 32]`-shaped, which is one transposition away from a comparison that
/// passes for the wrong reason — [`Compiled`]'s own comment, one level up and
/// with more at stake, since what is being compared here is *the machine before
/// and after*.
pub(crate) struct Packed {
    /// The root the fold produced over the record tree.
    /// Unit: bytes, exactly 32 — a SHA-256.
    pub root: [u8; 32],
    /// Where the boot module was written, named `<root>.fcm`.
    /// Unit: none — a path.
    pub module: PathBuf,
    /// The whole boot module, as it was written.
    /// Unit: bytes.
    pub bytes: Vec<u8>,
    /// The frame image's content address.
    /// Unit: bytes, exactly 32 — a SHA-256.
    pub frame: [u8; 32],
    /// Every component, by the name the source gives it and the content address
    /// this build produced for it, in the source's canonical order.
    /// Unit: none — a name and a SHA-256 each.
    pub components: Vec<(String, [u8; 32])>,
}

/// Compile, check, fold and pack one generation, and hand back all of it.
///
/// The same four steps [`generation`] takes and without the printing, so that a
/// caller building two generations in one command does not have to read one out
/// of a log it also has to print. `defects` reach the frame build and nothing
/// else, which is [`compile_here`]'s contract.
///
/// # Errors
///
/// Anything [`compile_here`], [`checked`] or [`pack_module`] refuses.
pub(crate) fn pack(defects: &[&str]) -> Result<Packed, String> {
    let built = compile_here(defects)?;
    let tree = checked(&built.bytes)?;
    let root = fold::root(&tree);
    let module = pack_module(&root, &tree, &built.source.components)?;
    let bytes =
        std::fs::read(&module).map_err(|e| format!("reading {}: {e}", crate::relative(&module)))?;
    let components =
        built.source.components.iter().cloned().zip(built.components.iter().copied()).collect();
    Ok(Packed { root, module, bytes, frame: built.frame, components })
}

/// Where `--install` writes, under the build directory rather than under
/// `/etc`.
///
/// A command that rewrote a bootloader configuration as a side effect of a
/// build would be a command nobody could run twice on a machine they cared
/// about. `tools/f-on-metal.sh` is what installs on metal and it already backs
/// up `grub.cfg`, never touches `GRUB_DEFAULT`, and writes a whole file rather
/// than appending to `40_custom`; this writes the same shape of file for it to
/// copy, so the two agree by construction and only one of them can break a
/// machine.
const FRAGMENT: &str = "45_f_generations";

/// The same generations as data, for a writer that knows the machine.
///
/// # Why a second output and not a second reader of the first
///
/// [`FRAGMENT`] hardcodes `/boot/f/`, and it has to: this command runs on a
/// build host and cannot know where GRUB will see those files. On a machine
/// whose `/boot` is its own partition GRUB's own path is `/f/`, so that fragment
/// names a menu that boots nothing there — the same failure
/// `docs/postmortem/0001` records `--install` already having shipped once, in a
/// different disguise.
///
/// `tools/f-on-metal.sh` is the writer that *does* know: it resolves the path
/// against the running machine and writes every other entry on it. What it
/// cannot do is read a `.fcm`'s record tree in shell, which is where `f.frame=`
/// comes from. So the two halves are split along that line and no further —
/// this file carries the numbers and no layout, the script carries the layout
/// and computes no numbers, and neither has a copy of the other's half.
///
/// One line per generation, tab separated, `root frame module bytes`. Written
/// beside the fragment rather than instead of it: the fragment is still the
/// answer for the manual procedure on an ordinary machine, and
/// `docs/booting-on-hardware.md` says which is which.
/// Unit: none — a filename.
const MENU_DATA: &str = "generations.tsv";

/// The most generations one `menuentry` can offer.
///
/// `kernel::arch::x86_64::multiboot::MAX_MODULES` is eight and the entry
/// already spends five of them — `init.bin` and four component files — so three
/// is what is left. Refused rather than truncated: a menu entry silently
/// missing the generation somebody meant to roll back to is the failure this
/// whole task is a test for.
/// Unit: count of boot modules.
const GENERATIONS_MAX: usize = 3;

/// One boot module already packed into the build directory, as `--install`
/// found it.
///
/// A record rather than the size alone, which is what it was until `f.frame=`
/// became half of the statement a `menuentry` makes: an entry naming a
/// generation and not the frame it was compiled against is an entry the frame
/// refuses — `HalfADeclaration` in `kernel/src/measure.rs` — so the menu would
/// have been a file that installs cleanly and boots nothing.
struct Installed {
    /// The module file's size on disk, printed so the operator can see that a
    /// `menuentry` names something with bytes behind it.
    /// Unit: bytes.
    size: u64,
    /// The frame image's content address, taken out of this module's own record
    /// tree rather than from the build that happens to be in `target/`.
    /// Unit: bytes, exactly 32 — a SHA-256.
    frame: [u8; 32],
}

/// The frame leaf of an already-packed boot module.
///
/// # Why this is read back out of the file rather than taken from the build
///
/// Because `--install` writes a menu over *whatever generations are installed*,
/// and the whole point of keeping more than one is that they were built from
/// different sources. The frame hash the current build would report is the
/// current build's; an entry that carried it beside another generation's root
/// would be an entry claiming a frame that generation was never compiled
/// against, and the machine would refuse it — correctly, and for a reason
/// nobody could read off the menu.
///
/// Read through `f_abi::boot::Module` and `f_generation::record::Tree`, which
/// are the reader the frame itself uses. A second decoder of this layout in
/// `xtask` is the reversal the boot-module arrangement names for itself.
///
/// # Errors
///
/// A `.fcm` in the build directory that is not a boot module, or whose record
/// tree is not one the checker believes.
fn frame_leaf(path: &Path) -> Result<[u8; 32], String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("reading {}: {e}", crate::relative(path)))?;
    let module = f_abi::boot::Module::read(&bytes).map_err(|why| {
        format!(
            "{} is not a boot module this tree can read ({why:#x}). `cargo xtask \
             generation` is what packs one; a stray `.fcm` in that directory cannot be \
             offered.",
            crate::relative(path)
        )
    })?;
    let tree = Tree::check(module.tree())
        .map_err(|why| format!("{}: {}", crate::relative(path), why.message()))?;
    Ok(tree.frame().hash)
}

/// `cargo xtask generation --install [DIR]`.
///
/// # What "the boot menu" means, settled rather than inferred
///
/// `E2-P07`'s exit says *roll back from the boot menu* and there is no boot
/// menu in this system, which a reviewer found and `intent/0006-state/spec.md`
/// settled: the menu is **the loader's own**. On hardware that is GRUB, whose
/// configuration this repository already writes and whose source it does not
/// import — GRUB is GPLv3 and `LICENSING.md` is where that boundary lives. So
/// this writes one `menuentry` per installed generation, each carrying
/// `f.root=<64 hex>` on its `multiboot` line, and every generation on offer as
/// `module` lines so the token has something to select from.
///
/// Under QEMU there is no menu at all and none is needed: `-append` is the same
/// command line and `cargo xtask rollback` passes it. That is the whole of the
/// difference between the two, which is why one token rather than a menu format
/// is the mechanism.
///
/// # Errors
///
/// A build directory with no modules in it, or more of them than one entry can
/// offer.
fn install(into: Option<&str>) -> Result<(), String> {
    let dir = crate::target_dir().join("generation");
    let mut modules: std::collections::BTreeMap<String, Installed> =
        std::collections::BTreeMap::new();
    let entries = std::fs::read_dir(&dir).map_err(|e| {
        format!(
            "reading {}: {e}\n\n\
             There is nothing installed to write a menu for. `cargo xtask generation` \
             packs a boot module into that directory; this writes the entries that \
             offer them.",
            crate::relative(&dir)
        )
    })?;
    for entry in entries {
        let path = entry.map_err(|e| format!("reading {}: {e}", crate::relative(&dir)))?.path();
        if path.extension().is_some_and(|e| e == "fcm")
            && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
        {
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            modules.insert(stem.to_string(), Installed { size, frame: frame_leaf(&path)? });
        }
    }

    if modules.is_empty() {
        return Err(format!(
            "no boot module in {}. `cargo xtask generation` is what packs one.",
            crate::relative(&dir)
        ));
    }
    if modules.len() > GENERATIONS_MAX {
        return Err(format!(
            "{} generations are installed and one menu entry can offer {GENERATIONS_MAX}.\n\n\
             A loader hands the frame at most {} modules and this entry already spends five \
             of them on `init.bin` and the component files. Truncating the list would leave \
             a menu quietly missing the generation somebody meant to roll back to, which is \
             the failure this command exists to prevent — so delete the modules you are not \
             keeping from {} and run this again.",
            modules.len(),
            GENERATIONS_MAX + 5,
            crate::relative(&dir)
        ));
    }

    let out = match into {
        Some(path) => PathBuf::from(path).join(FRAGMENT),
        None => dir.join(FRAGMENT),
    };
    let text = fragment(&modules);
    std::fs::write(&out, &text).map_err(|e| format!("writing {}: {e}", out.display()))?;

    // The same set as data, argued at `MENU_DATA`. Written unconditionally and
    // beside the fragment, because a file that appears only when somebody
    // passed a flag is a file the script that needs it will one day not find.
    let data = out.with_file_name(MENU_DATA);
    let mut rows = String::new();
    for (root, module) in &modules {
        rows.push_str(&format!("{root}\t{}\t{root}.fcm\t{}\n", hex(&module.frame), module.size));
    }
    std::fs::write(&data, &rows).map_err(|e| format!("writing {}: {e}", data.display()))?;

    println!("generation --install\n");
    for (root, module) in &modules {
        println!(
            "  menuentry  f.root={root}  f.frame={}  ({} bytes)",
            hex(&module.frame),
            module.size
        );
    }
    println!(
        "\n  fragment   {}  ({} entries, {} bytes)\n  \
         data       {}  ({} row(s))\n\n  \
         Two outputs, and which you want depends on where GRUB sees /boot. The\n  \
         fragment names `/boot/f/` and is the manual answer on a machine whose\n  \
         /boot is a directory: copy it to /etc/grub.d/{FRAGMENT}, chmod 0755, and\n  \
         regenerate grub.cfg. Where /boot is its own partition that path is wrong\n  \
         and the entries boot nothing, so there `tools/f-on-metal.sh install\n  \
         --generations` is the answer: it reads the data file, resolves the path\n  \
         against the running machine, and writes the entries itself with the\n  \
         backups. `docs/booting-on-hardware.md` is the whole procedure.",
        crate::relative(&out),
        modules.len(),
        text.len(),
        crate::relative(&data),
        modules.len(),
    );
    Ok(())
}

/// The fragment's text: a `/etc/grub.d` script that emits one entry per
/// generation.
///
/// # Why the order is the root's and not the age's
///
/// The spec's sentence is that the default is the newest module the loader
/// offered, and *newest* is a clock. Nothing in this tree may read one outside
/// `f_env::Env` — RFC 0004 — and a generation record carries no ordinal, so
/// there is no in-band answer to *which of these is later*. Sorting by root hex
/// is arbitrary and **stable**, which is the property that matters for a file
/// under review: two runs over one build directory produce one fragment. The
/// person at the console picks the entry; `GRUB_DEFAULT` is theirs and this
/// never touches it, for `tools/f-on-metal.sh`'s reason.
///
/// *What would reverse this:* something that orders generations without a
/// clock — a predecessor hash in the record tree, which is a fifth thing in
/// `abi/store` and therefore an RFC, not a patch.
fn fragment(modules: &std::collections::BTreeMap<String, Installed>) -> String {
    let mut out = String::new();
    out.push_str(
        "#!/bin/sh\n\
         # SPDX-License-Identifier: Apache-2.0 OR MIT\n\
         #\n\
         # Generated by `cargo xtask generation --install`. One entry per installed\n\
         # generation; the root on the `multiboot` line is what selects it, and every\n\
         # generation is offered as a `module` so the frame has something to select\n\
         # from. `abi/src/boot.rs` is the grammar and `kernel/src/generation.rs` is\n\
         # the reader.\n\
         #\n\
         # `f.frame=` travels beside every root because the two are one statement:\n\
         # the frame compares it against a measurement of its own text and rodata\n\
         # before it will publish anything, and refuses a command line carrying one\n\
         # token without the other. RFC 0012; kernel/src/measure.rs is the\n\
         # comparison. Each entry carries the frame *that* generation was compiled\n\
         # against, read out of its own record tree.\n\
         #\n\
         # Entries are in ascending root order, which is arbitrary and stable: there\n\
         # is no clock in this tree and a record tree carries no ordinal, so there is\n\
         # no in-band answer to which generation is newer. GRUB_DEFAULT is yours.\n\
         cat <<'MENU'\n",
    );
    for (root, module) in modules {
        out.push_str(&format!(
            "menuentry \"F — generation {short}\" --class f {{\n    \
             echo \"F: loading generation {short}. Output on COM1 — there is no video.\"\n    \
             insmod part_gpt\n    \
             insmod part_msdos\n    \
             insmod fat\n    \
             insmod ext2\n    \
             insmod multiboot\n    \
             search --no-floppy --file --set=root /boot/f/f-kernel.elf32\n    \
             multiboot /boot/f/f-kernel.elf32 f.root={root} {frame}{declared}\n    \
             module /boot/f/init.bin\n",
            short = &root[..16],
            frame = f_abi::boot::FRAME_KEY,
            declared = hex(&module.frame),
        ));
        for name in crate::COMPONENTS {
            out.push_str(&format!("    module /boot/f/{name}.fc\n"));
        }
        for offered in modules.keys() {
            out.push_str(&format!("    module /boot/f/{offered}.fcm\n"));
        }
        out.push_str("}\n\n");
    }
    out.push_str("MENU\n");
    out
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
            // `.claude` for `crate::rust_sources`'s reason: an agent harness
            // puts git worktrees under `.claude/worktrees/`, so other checkouts
            // of this repository sit inside this one. A `generation.toml` found
            // in one of them is another tree's source, and the fixpoint below
            // would hold it to this tree's grammar.
            if matches!(name, "target" | ".git" | ".claude" | "third_party") {
                continue;
            }
            walk(&path, found)?;
        } else if name == "generation.toml" {
            found.push(path);
        }
    }
    Ok(())
}

/// The file a run leaves for another run to compare against: the record tree,
/// as bytes.
///
/// The *tree* and not the printed block, because a job that parses a log is a
/// job that can parse it wrong, and because the tree is what the fold is over —
/// so the comparison and the root are taken from one artefact rather than from
/// two things that are supposed to agree. `trace --hash` made the same choice
/// for the same reason one epoch earlier.
const TREE_FILE: &str = "tree.bin";

/// The root, as sixty-four hexadecimal characters and a newline.
///
/// Redundant with [`TREE_FILE`] — it can be recomputed from it — and written
/// anyway, because it is what a person reads out of an artefact and what a
/// workflow can compare with `test`. [`compare`] recomputes it and refuses a
/// pair whose file and tree disagree, so the redundancy is checked rather than
/// trusted.
const ROOT_FILE: &str = "root";

/// `cargo xtask generation --emit DIR [--defect FEATURE]`.
///
/// One run's half of a comparison. The weekly job runs this on each runner and
/// uploads the directory; [`compare`] is what a third job then runs over the
/// two.
fn emit(rest: &[String]) -> Result<(), String> {
    let Some(dir) = rest.first() else {
        return Err("`--emit` takes a directory to write the run's artefact into.".into());
    };
    let defect = match rest.get(1).map(String::as_str) {
        None => None,
        Some("--defect") => Some(rest.get(2).ok_or("`--defect` takes a kernel feature name")?),
        Some(other) => return Err(format!("unknown option for generation --emit: {other}")),
    };
    let defects: Vec<&str> = defect.map(|name| vec![name.as_str()]).unwrap_or_default();

    let built = compile_here(&defects)?;
    let tree = checked(&built.bytes)?;
    print_tree(&built, &tree);

    let dir = Path::new(dir);
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    std::fs::write(dir.join(TREE_FILE), tree.bytes())
        .map_err(|e| format!("writing {}: {e}", dir.join(TREE_FILE).display()))?;
    let root = hex(&fold::root(&tree));
    std::fs::write(dir.join(ROOT_FILE), format!("{root}\n"))
        .map_err(|e| format!("writing {}: {e}", dir.join(ROOT_FILE).display()))?;
    println!("\n  emitted    {}  ({} bytes of record tree)", dir.display(), tree.bytes().len());
    Ok(())
}

/// One emitted artefact, checked.
fn read_emitted(dir: &Path) -> Result<Vec<u8>, String> {
    let path = dir.join(TREE_FILE);
    let bytes = std::fs::read(&path).map_err(|e| {
        format!(
            "reading {}: {e}\n\n\
             That file is written by `cargo xtask generation --emit`. A comparison with \
             one side missing is not a comparison, so this is fatal rather than a skip: \
             the likeliest cause is a run that failed before it emitted, and reporting \
             *agreed* for a pair with one member would be the worst possible answer.",
            path.display()
        )
    })?;
    let recorded = std::fs::read_to_string(dir.join(ROOT_FILE))
        .map_err(|e| format!("reading {}: {e}", dir.join(ROOT_FILE).display()))?;
    let tree = Tree::check(&bytes).map_err(|why| {
        format!("{}: the record tree is refused: {}", path.display(), why.message())
    })?;
    let root = hex(&fold::root(&tree));
    if root != recorded.trim() {
        return Err(format!(
            "{}: the recorded root is {}, and folding the tree beside it gives {root}.\n\n\
             The two are written by one command over one tree, so they cannot disagree \
             unless the artefact was edited or the fold moved between writing and reading. \
             Either way the pair is not evidence about anything.",
            dir.display(),
            recorded.trim()
        ));
    }
    Ok(bytes)
}

/// `cargo xtask generation --compare A B`: two emitted runs, and the first place
/// they stop agreeing.
///
/// # Why this prints a leaf and not a verdict
///
/// Because *two roots differed* is the finding nobody can act on, and it is the
/// one this whole task exists to stop producing. `f_generation::diff` descends
/// leaves before roots, so what comes back names the input that moved — and when
/// nothing below the root moved, it says *that* instead of blaming an input it
/// cannot name.
fn compare(left: &Path, right: &Path) -> Result<(), String> {
    let (a, b) = (read_emitted(left)?, read_emitted(right)?);
    let (ta, tb) = (checked(&a)?, checked(&b)?);
    let (ra, rb) = (fold::root(&ta), fold::root(&tb));

    println!("generation root");
    println!("  {:<28} {}", crate::relative(left), hex(&ra));
    println!("  {:<28} {}", crate::relative(right), hex(&rb));

    let Some(divergence) = f_generation::divergence(&ta, &tb) else {
        println!("\n  agreed: {}", hex(&ra));
        return Ok(());
    };
    Err(finding(&divergence, left, right))
}

/// The divergence, as the sentence the job's log carries and its issue quotes.
///
/// Written here and not in `f-generation` because this is where an allocator is:
/// that crate is `no_std`, hands back the padded name out of the record, and
/// deliberately does not know that one of these two runs is a runner called `a`.
fn finding(divergence: &f_generation::Divergence, left: &Path, right: &Path) -> String {
    use f_generation::Divergence;

    let name = |padded: &[u8]| -> String {
        let end = padded.iter().position(|b| *b == 0).unwrap_or(padded.len());
        String::from_utf8_lossy(&padded[..end]).into_owned()
    };
    let (l, r) = (crate::relative(left), crate::relative(right));

    match divergence {
        Divergence::Leaf { left: a, right: b } => format!(
            "the generation roots differ, and the input that moved is `{}`.\n\n\
             \x20 {l:<28} {}\n\
             \x20 {r:<28} {}\n\n\
             That leaf names a content address, so the two runs were handed different \
             bytes under one name. It is the *first* leaf that differs in fold order and \
             not necessarily the only one — fix this one and run the comparison again.\n\n\
             The frame is the leaf that carries debug information, so if this is `{}` the \
             first thing to check is that `.cargo/config.toml`'s remap still reaches the \
             `x86_64-unknown-none` target: cargo replaces `build.rustflags` with the \
             target's list rather than merging them, and a remap that stopped applying \
             there applies everywhere except the one image this leaf is taken over. \
             `cargo xtask lint-remap` is the check that was supposed to catch that, and \
             its being green while this is red is itself a finding.",
            name(&a.name),
            hex(&a.hash),
            hex(&b.hash),
            name(&a.name),
        ),
        Divergence::Membership { left: a, right: b } => format!(
            "the two runs do not name the same components.\n\n\
             \x20 {l:<28} {}\n\
             \x20 {r:<28} {}\n\n\
             That is not a reproducibility finding about a build: it is two different \
             `user/generation.toml` files, which means the two runs were not at one \
             commit. Check what each one checked out before looking at anything else.",
            a.map_or("(nothing here)".into(), |leaf| name(&leaf.name)),
            b.map_or("(nothing here)".into(), |leaf| name(&leaf.name)),
        ),
        Divergence::Route { index, .. } => format!(
            "every leaf agrees and route {index} does not.\n\n\
             A route is a field of the topology and not a child of it, so this changes \
             the root while every content address under it stands still. Same conclusion \
             as a membership difference: two sources, not two builds."
        ),
        Divergence::Head => "every member and every route agrees and the topology's stored head \
             does not.\n\n\
             Only two counts live in that head and both are implied by what has already \
             been compared, so reaching this is a statement about the encoder in \
             xtask/src/generation.rs rather than about either run's inputs."
            .to_string(),
        Divergence::Compiler => "every leaf agrees and the roots do not.\n\n\
             Nothing was handed different bytes, so the *fold* is what differed. That is \
             a defect in `generation/` or in `abi/src/store.rs` — two builds of this \
             workspace that do not agree about the arithmetic over one tree — and it is \
             not a non-reproducible input. No input is named here because there is none \
             to name, which is the whole reason this case is separated from the one \
             above it."
            .to_string(),
    }
}

/// Where the second checkout goes.
///
/// Deliberately of a different length and a different shape from any checkout
/// this tree is built in, because the failure being looked for is a path
/// *appearing in* an artefact and two paths of equal length can hide one: a
/// remap that replaced the prefix with something the same width would leave two
/// images that differ nowhere a byte comparison could see, and the check would
/// pass for the wrong reason.
const ELSEWHERE: &str = "f-generation-at-an-entirely-different-checkout-path";

/// `cargo xtask generation --elsewhere`: the same expression, evaluated at two
/// paths, and one root.
///
/// # What this demonstrates that `E2-B04`'s rehearsal could not
///
/// `E2-B04` ran the fold twice in two containers over **one checkout**, and its
/// own `TODO.md` line says what that leaves untested: every difference two
/// machines can have except the one they certainly do have, which is where the
/// tree is. This is that one. It copies the working tree to a second path, runs
/// the same verb there, and requires one root — and before `.cargo/config.toml`
/// carried a remap it did not get one:
///
/// ```text
/// /work                          frame c764535 0…  root b142e37c…
/// /tmp/a-second-checkout-path     frame d03b72f8…  root 65494a4f…
/// ```
///
/// Measured on 2026-09-07, same image, same commit, same volumes, four component
/// leaves and the topology hash identical in both — so the divergence was one
/// leaf and it named itself, which is the behaviour the job is built on.
///
/// # What it still is not
///
/// Two machines. One host, one kernel, one filesystem, one uid, one clock: this
/// separates the *path* out of that bundle and says nothing about the rest. The
/// two-runner half is `.github/workflows/weekly.yml`, and it is a job rather
/// than a command for the reason `E0-R01`'s `address` job is one.
fn elsewhere(defects: &[&str]) -> Result<(), String> {
    let there = copy_tree()?;
    let (here, mirror) = artefacts();

    let mut args = vec!["xtask".to_string(), "generation".to_string(), "--emit".to_string()];
    args.push(here.display().to_string());
    for defect in defects {
        args.push("--defect".to_string());
        args.push((*defect).to_string());
    }
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    crate::sh("cargo", &borrowed)?;

    // The second run is a *child process* rather than another call into this
    // one, and that is not incidental. `crate::root()` is `env!("CARGO_MANIFEST_DIR")`
    // — the path this binary was compiled at — so an xtask built here and called
    // in a loop would compile the second tree's kernel while still believing it
    // was at the first path, and would then compare an image against itself. The
    // child rebuilds xtask in the tree it is run from, which is what makes the
    // two paths two paths.
    let mut args = vec!["xtask".to_string(), "generation".to_string(), "--emit".to_string()];
    args.push(mirror.display().to_string());
    for defect in defects {
        args.push("--defect".to_string());
        args.push((*defect).to_string());
    }
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    crate::run_in(&there, "cargo", &borrowed)?;

    println!("\ntwo checkouts, one expression\n");
    println!("  {:<12} {}", "here", crate::root().display());
    println!("  {:<12} {}", "elsewhere", there.display());
    println!();
    compare(&here, &mirror)
}

/// The two directories a two-path run leaves its artefacts in.
///
/// Spelled once rather than at each caller. [`mutate`] reads back the pair
/// [`elsewhere`] wrote, and two spellings of one path is how a comparison comes
/// to read last week's artefact and report on a run nobody made.
fn artefacts() -> (PathBuf, PathBuf) {
    let there = std::env::temp_dir().join(ELSEWHERE);
    (
        crate::target_dir().join("generation").join("here"),
        there.join("target").join("generation").join("there"),
    )
}

/// How many distinct roots the two emitted artefacts hold.
///
/// Read back out of the files rather than inferred from whether [`compare`]
/// returned `Ok`. A row that reads 1 because a function did not return an error
/// says only that the function did not return an error; this one is the size of
/// a set of roots, each of which [`read_emitted`] has already required to equal
/// a fold over the tree stored beside it. So the number is wrong only if the
/// fold is, which is the case [`finding`]'s `Compiler` arm is for.
///
/// # Errors
///
/// [`read_emitted`]'s, and [`checked`]'s.
fn distinct_roots(here: &Path, there: &Path) -> Result<usize, String> {
    let mut seen = std::collections::BTreeSet::new();
    for dir in [here, there] {
        let bytes = read_emitted(dir)?;
        let tree = checked(&bytes)?;
        seen.insert(hex(&fold::root(&tree)));
    }
    Ok(seen.len())
}

/// `cargo xtask generation --mutate`: the half that says a green comparison
/// means something.
///
/// A reproducibility check that has only ever passed is indistinguishable from
/// one that cannot fail, and this one is unusually easy to get wrong in that
/// direction — a comparison over an artefact that does not contain the build at
/// all would agree with itself forever. So the deliberate defect compiles the
/// build path into the frame image, and this requires the two-path comparison to
/// go red **and to name the frame**. Going red is not the assertion: a
/// comparison that failed for some other reason would satisfy an exit code and
/// prove nothing, which is the argument `MUTATIONS` makes for every boot.
fn mutate() -> Result<(), String> {
    let (here, there) = artefacts();

    println!("[1/2] two checkouts, honest build — the roots must agree\n");
    elsewhere(&[])?;

    // Read before the armed run overwrites both artefacts. The order is the
    // whole of why these two lines are here and not at the bottom beside the
    // rows they feed.
    let honest = distinct_roots(&here, &there)?;
    let leaves = {
        let bytes = read_emitted(&here)?;
        let tree = checked(&bytes)?;
        usize::from(tree.members()) + 1
    };

    println!("\n[2/2] with the build path compiled in — they must differ, and name the frame\n");
    let armed = elsewhere(&[PATH_DEFECT]);
    let Err(report) = armed else {
        return Err(format!(
            "the kernel built with `{PATH_DEFECT}` still produced one root at two paths.\n\n\
             That means this check cannot fail, which makes the green result above worth \
             nothing. Either the defect is no longer in the image — `#[used]` is what keeps \
             the linker from dropping a static nothing reads — or the comparison is over \
             something that does not contain the frame."
        ));
    };
    if !report.contains("the input that moved is `kernel`") {
        return Err(format!(
            "the armed comparison went red and did not name the frame:\n\n{report}\n\n\
             Red is not the assertion. `{PATH_DEFECT}` puts a path in exactly one leaf, so \
             a comparison that names a different one — or that names nothing — is reading \
             something other than what it believes it is reading, and the green run above \
             says nothing either."
        ));
    }
    let armed_roots = distinct_roots(&here, &there)?;
    println!("{report}");
    println!("\n  ...which is the required failure, and it named the frame.");

    // The path gap is a measurement and not a decoration: `ELSEWHERE`'s own
    // comment argues that two checkout paths of *equal length* would let a
    // remap that replaced one prefix with another of the same width leave two
    // images a byte comparison cannot tell apart. This row is that argument
    // made checkable, so a future edit that shortens the constant fails the
    // claim rather than quietly weakening every run under it.
    let gap = crate::root()
        .display()
        .to_string()
        .len()
        .abs_diff(std::env::temp_dir().join(ELSEWHERE).display().to_string().len());

    println!("\n  claims/0028 generation-roots-across-paths");
    for (name, value) in [
        ("generation_roots_across_two_checkout_paths", honest),
        ("armed_generation_roots_across_two_checkout_paths", armed_roots),
        ("checkout_path_length_difference_bytes", gap),
        ("leaves_folded_into_the_root", leaves),
        // Reached only after the report above was required to name `kernel` by
        // name, so this row is not the assertion — it is what says the
        // assertion ran. A run that never got here prints nothing, and a
        // threshold no workload printed is a finding rather than a silence.
        ("armed_comparisons_naming_the_frame", 1),
    ] {
        println!("    {name:<50} {value}");
    }

    println!(
        "\ngeneration --mutate: ok — the two-path comparison can fail, and the leaf it \
         names is the one the defect is in."
    );
    Ok(())
}

/// The kernel feature that makes an image a function of where it was built.
pub const PATH_DEFECT: &str = "mutate-path-in-image";

/// Copy the working tree to [`ELSEWHERE`], and say where it went.
///
/// `target/` is skipped because it is build output and because in the
/// development container it is a volume rather than a directory; `.git` is
/// skipped because it is a checkout's identity and not its content, and in a
/// worktree it is a file pointing outside the tree entirely. Everything else
/// goes, including the dotfiles, because `.cargo/config.toml` is the thing being
/// tested and a copy without it would test nothing.
///
/// The copy is refreshed rather than rebuilt: the second tree keeps its own
/// `target/` between runs, so `--mutate`'s four builds are one cold build and
/// three kernel rebuilds rather than four cold builds.
fn copy_tree() -> Result<PathBuf, String> {
    let dest = std::env::temp_dir().join(ELSEWHERE);
    let source = crate::root();
    let mut copied = 0usize;
    copy_into(&source, &dest, &mut copied)?;
    println!("  copied     {copied} file(s) to {}", dest.display());
    Ok(dest)
}

fn copy_into(from: &Path, to: &Path, copied: &mut usize) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("creating {}: {e}", to.display()))?;
    let entries =
        std::fs::read_dir(from).map_err(|e| format!("reading {}: {e}", from.display()))?;
    // Sorted for the reason `walk` above is sorted: directory order is a
    // filesystem's business, and this function's whole purpose is to leave two
    // trees that differ in nothing.
    let mut paths: Vec<PathBuf> = Vec::new();
    for entry in entries {
        paths.push(entry.map_err(|e| format!("reading {}: {e}", from.display()))?.path());
    }
    paths.sort();
    for path in paths {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let target = to.join(name);
        if path.is_dir() {
            // And `.claude`, which here is not a lint's accuracy but a copy's
            // size and meaning: `.claude/worktrees/` holds whole checkouts of
            // this repository, so copying it would copy the tree into itself
            // once per parallel worktree — and the copy is supposed to be *this*
            // tree at a different path, which a copy containing three others is
            // not.
            if matches!(name, "target" | ".git" | ".claude") {
                continue;
            }
            copy_into(&path, &target, copied)?;
        } else if path.is_file() {
            if name == ".git" {
                continue;
            }
            std::fs::copy(&path, &target)
                .map_err(|e| format!("copying {} to {}: {e}", path.display(), target.display()))?;
            *copied += 1;
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

    /// Two roots, so that the ordering and the offer list are both visible, and
    /// two *different* frame hashes, so that an entry pairing a root with the
    /// wrong generation's frame is a thing the tests below can see.
    fn installed() -> std::collections::BTreeMap<String, Installed> {
        let mut out = std::collections::BTreeMap::new();
        out.insert("b".repeat(64), Installed { size: 65_348, frame: [0xBB; 32] });
        out.insert("a".repeat(64), Installed { size: 64_000, frame: [0xAA; 32] });
        out
    }

    #[test]
    fn every_entry_carries_its_own_root_and_offers_every_generation() {
        let text = fragment(&installed());
        // One entry per generation, each naming its own root on the multiboot
        // line: an entry that offered a module and did not name it would be a
        // menu line that boots whatever the frame defaults to.
        for (root, module) in &installed() {
            // Both halves, and the frame half is *this* generation's. An entry
            // naming a root without the frame it was compiled against is one the
            // machine refuses — `HalfADeclaration` — and an entry carrying some
            // other generation's frame is one it refuses for a reason nobody can
            // read off the menu. Neither is visible until a boot.
            assert!(
                text.contains(&format!(
                    "multiboot /boot/f/f-kernel.elf32 f.root={root} {}{}",
                    f_abi::boot::FRAME_KEY,
                    hex(&module.frame)
                )),
                "no entry selects {root} and declares the frame it was built against"
            );
            // And every generation is offered by *both* entries, because a token
            // can only select from what the loader placed. An entry offering
            // one module could never roll back.
            assert_eq!(
                text.matches(&format!("module /boot/f/{root}.fcm")).count(),
                installed().len()
            );
        }
        assert_eq!(text.matches("menuentry ").count(), installed().len());
    }

    #[test]
    fn the_order_is_the_roots_and_two_runs_write_one_file() {
        // Arbitrary and stable is the property, and stable is the half that
        // matters for a file under review. There is no clock in this tree, so
        // *newest* is not available and is not pretended at.
        let text = fragment(&installed());
        assert_eq!(text, fragment(&installed()));
        let first = text.find(&"a".repeat(64)).expect("the lower root is in there");
        let second = text.find(&"b".repeat(64)).expect("the higher root is in there");
        assert!(first < second, "entries are in ascending root order");
    }

    #[test]
    fn a_fragment_is_a_grub_d_script_and_not_a_grub_cfg() {
        // `/etc/grub.d/*` are executable scripts whose *output* is the menu, so
        // a fragment that was the menu itself would be copied into place and
        // emit nothing. `tools/f-on-metal.sh` writes the same shape.
        let text = fragment(&installed());
        assert!(text.starts_with(
            "#!/bin/sh
"
        ));
        assert!(text.contains("SPDX-License-Identifier: Apache-2.0 OR MIT"));
        assert!(text.contains(
            "cat <<'MENU'
"
        ));
        assert!(text.ends_with(
            "MENU
"
        ));
    }
}
