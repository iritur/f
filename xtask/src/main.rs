// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Build orchestration, and the place where written policy becomes a check
//! that can fail a build.
//!
//! Three of the commands here exist because a policy nobody can enforce is a
//! preference: `lint-determinism`, `lint-licensing` and `lint-unsafe`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const KERNEL_TARGET: &str = "x86_64-unknown-none";

/// Direct sources of nondeterminism. Any of these outside an allowed path
/// breaks the seed-plus-commit reproduction contract, which every other layer
/// of the test apparatus rests on.
///
/// See `docs/rfc/0004-determinism-substrate.md`.
const FORBIDDEN: &[(&str, &str)] = &[
    ("rdtsc", "read time through f_env::Env, not the counter directly"),
    ("SystemTime::now", "read time through f_env::Env"),
    ("Instant::now", "read time through f_env::Env"),
    ("thread_rng", "draw randomness from f_env::Env"),
    ("random()", "draw randomness from f_env::Env"),
    ("HashMap::new", "iteration order is seeded per process; use BTreeMap"),
    ("HashSet::new", "iteration order is seeded per process; use BTreeSet"),
];

/// Paths permitted to contain a forbidden construct, each for a stated reason.
/// Adding an entry here is a reviewable diff, which is the point.
const DETERMINISM_ALLOW: &[(&str, &str)] = &[
    ("kernel/src/arch/x86_64/mod.rs", "the single legitimate hardware time source"),
    (
        "bench/",
        "the harness measures the system and is not part of it; a clock is what \
         an instrument is. Revisit at M2, when a hardware Env exists for the \
         host and the harness can read time through the substrate like \
         everything else",
    ),
];

/// Paths that are the *checker* rather than the checked.
///
/// A policy check contains the pattern it looks for, so without this it reports
/// itself: `lint-licensing` matched on the literal `third_party` in its own
/// source, and `lint-unsafe` matched on the literal it greps for. Neither was a
/// violation; both were the tool describing its own job.
///
/// This exempts only the *textual* checks. `xtask` carries
/// `[lints] workspace = true` like every other crate, so `unsafe_code =
/// "forbid"` still applies to it at compile time — the enforcement that
/// actually matters is unaffected.
const TOOLING: &[(&str, &str)] = &[(
    "xtask/",
    "build tooling: it runs outside the system under test, and it contains the \
     needles the policy checks search for",
)];

/// True if `rel` names the checker rather than the checked.
fn is_tooling(rel: &str) -> bool {
    TOOLING.iter().any(|(path, _)| rel.starts_with(path))
}

/// Crates permitted to contain `unsafe`. This list is the trusted computing
/// base that the < 5% metric in the architecture document measures.
const UNSAFE_ALLOW: &[&str] = &["abi/", "ring/", "kernel/"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    let result = match cmd {
        "build" => build(),
        "run" => run(),
        "fault" => fault(args.get(1).map(String::as_str)),
        "test" => test(),
        "lint" => lint_all(),
        "lint-determinism" => lint_determinism(),
        "lint-licensing" => lint_licensing(),
        "lint-unsafe" => lint_unsafe(),
        "claims" => claims_list(),
        "claim" => claim_run(args.get(1).map(String::as_str)),
        "bench" => bench(args.get(1).map(String::as_str)),
        "coverage" => coverage(),
        "todo" => todo_list(args.get(1).map(String::as_str)),
        "help" | "--help" | "-h" => {
            help();
            Ok(())
        }
        other => Err(format!("unknown command: {other}\n\nTry `cargo xtask help`.")),
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("\nxtask: {message}");
            ExitCode::FAILURE
        }
    }
}

fn help() {
    println!(
        "\
cargo xtask <command>

  build              Build the kernel for {KERNEL_TARGET}
  run                Boot the kernel in QEMU and report its exit status
  fault [kind]       Boot it into a deliberate fault and check the report:
                     pf, ud or df
  test               Workspace tests on both x86-64 and AArch64
  lint               Every policy check below, in order

  lint-determinism   No direct source of nondeterminism outside the allow-list
  lint-licensing     SPDX headers present; no import of third_party from the
                     permissive tree
  lint-unsafe        No `unsafe` outside the frame crates

  claims             List the claims registry and whether each one gates
  claim <name>       Run one claim's workload and report against its threshold
  bench [name]       Run a benchmark binary directly
  coverage           Host tests with coverage instrumentation

  todo [epoch]       What in TODO.md is ready to start, and what is waiting on
                     what. The list is a dependency graph, not a sequence.
"
    );
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

fn sh(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed", args.join(" ")))
    }
}

fn capture(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} {} failed", args.join(" ")));
    }
    String::from_utf8(out.stdout).map_err(|e| format!("{program} printed non-UTF-8: {e}"))
}

/// A tool from the pinned toolchain's own sysroot.
///
/// `llvm-tools` is a component in `rust-toolchain.toml`, so a tool found this
/// way is pinned with the compiler. Taking the same tool from `PATH` would make
/// the build depend on whatever binutils the machine happens to carry, which is
/// exactly the ambient dependency the development container exists to remove.
fn llvm_tool(name: &str) -> Result<PathBuf, String> {
    let sysroot = capture("rustc", &["--print", "sysroot"])?;
    let version = capture("rustc", &["-vV"])?;
    let host = version
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .ok_or("rustc -vV did not report a host triple")?
        .to_string();

    let path = Path::new(sysroot.trim())
        .join("lib")
        .join("rustlib")
        .join(host)
        .join("bin")
        .join(name);

    if path.exists() {
        Ok(path)
    } else {
        Err(format!(
            "{name} not found at {}\n\n\
             It comes from the `llvm-tools` component, which rust-toolchain.toml\n\
             names. If this is missing the toolchain was installed without its\n\
             components: `rustup toolchain install` in the workspace root.",
            path.display()
        ))
    }
}

/// The kernel, as the linker produced it.
fn kernel_elf64() -> PathBuf {
    root().join("target").join(KERNEL_TARGET).join("debug").join("f-kernel")
}

/// The kernel, in the container format the loader will accept.
fn kernel_elf32() -> PathBuf {
    kernel_elf64().with_extension("elf32")
}

fn build() -> Result<(), String> {
    sh(
        "cargo",
        &["build", "-p", "f-kernel", "--target", KERNEL_TARGET, "-Zbuild-std=core,compiler_builtins"],
    )?;
    to_elf32()
}

/// Rewrite the ELF container from 64-bit to 32-bit.
///
/// QEMU's multiboot loader refuses an `ELFCLASS64` image — "give a 32bit one" —
/// because multiboot 1 predates long mode. Nothing in the image changes: the
/// code is still the same bytes, the 64-bit half still runs after the boot
/// stub's far jump, and every address in the image is below 4 GiB because the
/// linker script places it at 1 MiB. Only the headers describing it are
/// rewritten.
///
/// This is the one post-link step in the build, and it is the first of a
/// family: E5's real machine will want an image assembled for Limine or UEFI
/// rather than a bare ELF, and that step lands here too.
fn to_elf32() -> Result<(), String> {
    let objcopy = llvm_tool("llvm-objcopy")?;
    let src = kernel_elf64();
    let dst = kernel_elf32();

    let status = Command::new(&objcopy)
        .args(["-O", "elf32-i386"])
        .arg(&src)
        .arg(&dst)
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run llvm-objcopy: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "llvm-objcopy could not rewrite {} as a 32-bit ELF",
            relative(&src)
        ))
    }
}

fn run() -> Result<(), String> {
    build()?;
    let kernel = kernel_elf32();
    if !kernel.exists() {
        return Err(format!("kernel image not found at {}", kernel.display()));
    }

    // `isa-debug-exit` turns a kernel run into something an integration test can
    // assert on: the kernel chooses its own exit code and QEMU reports it.
    let status = Command::new("qemu-system-x86_64")
        .args([
            "-kernel",
            kernel.to_str().ok_or("kernel path is not valid UTF-8")?,
            // Pinned, not defaulted. The kernel now prints the loader's memory
            // map, so the machine's size is part of its output — and an
            // emulator default that moves between versions would move the
            // boot log with it, quietly breaking the one M0 contract that
            // matters: the same commit produces the same run, byte for byte.
            "-m",
            "128M",
            "-serial",
            "stdio",
            "-display",
            "none",
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-no-reboot",
        ])
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run qemu-system-x86_64: {e}"))?;

    // QEMU reports (value << 1) | 1, so Success(0x10) arrives as 33.
    match status.code() {
        Some(33) => {
            println!("\nM0 ok");
            Ok(())
        }
        Some(35) => Err("kernel reported failure — see the serial log above".into()),
        Some(other) => Err(format!("qemu exited {other}; expected 33 or 35")),
        None => Err("qemu terminated by signal".into()),
    }
}

/// Boot into a deliberate fault and check that the kernel reports it.
///
/// # Why this is a command rather than a test
///
/// The exception report is the one piece of the kernel that only runs when
/// something has already gone wrong. It cannot be exercised by using the kernel
/// normally, so it is either exercised on purpose or discovered to be broken at
/// the worst possible moment — which is what happened twice while the address
/// space was being built, when a fault with no handler was a silent reset and
/// the only way to find out why was to ask the emulator.
///
/// A failing exit code is the *expected* result here, which is why this is not
/// folded into `run`.
fn fault(kind: Option<&str>) -> Result<(), String> {
    let kind = kind.unwrap_or("pf");
    if !matches!(kind, "pf" | "ud" | "df" | "nx" | "wx" | "stack") {
        return Err(format!(
            "unknown fault kind: {kind}\n\n\
             pf  a write to an unmapped address\n\
             ud  an invalid opcode\n\
             df  a fault with no usable stack, which is the one that needs the\n\
             \x20   interrupt stack table to be reportable at all\n\
             nx  an execute from the direct map, which must fault because the\n\
             \x20   direct map is data. On a machine whose firmware has turned\n\
             \x20   no-execute off there is nothing to provoke, and the kernel\n\
             \x20   says so rather than pretending it was tested"
        ));
    }

    build()?;
    let kernel = kernel_elf32();

    let status = Command::new("qemu-system-x86_64")
        .args([
            "-kernel",
            kernel.to_str().ok_or("kernel path is not valid UTF-8")?,
            "-m",
            "128M",
            "-append",
        ])
        .arg(format!("fault={kind}"))
        .args([
            "-serial",
            "stdio",
            "-display",
            "none",
            "-device",
            "isa-debug-exit,iobase=0xf4,iosize=0x04",
            "-no-reboot",
        ])
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run qemu-system-x86_64: {e}"))?;

    match status.code() {
        // The kernel faulted, reported, and chose its own exit code. Which is
        // the whole point: a machine that reaches this has an exception path
        // that works.
        Some(35) => {
            println!("\nfault reported, and the kernel chose how to die");
            Ok(())
        }
        Some(33) => Err(format!(
            "the kernel finished normally — `fault={kind}` did not fault, so \
             nothing about the exception path was tested"
        )),
        Some(0) => Err(
            "the machine reset without reporting: a fault whose handler faults \
             is a triple fault, and a triple fault has no output. Either the \
             descriptor tables are not installed or the handler cannot run \
             where it was called from."
                .into(),
        ),
        Some(other) => Err(format!("qemu exited {other}; expected 35")),
        None => Err("qemu terminated by signal".into()),
    }
}

fn test() -> Result<(), String> {
    // Host tests exercise the ring and the substrate under the host memory
    // model. That is necessary and not sufficient — see lint output.
    // The whole workspace except the kernel, which cannot be built for the
    // host at all. Naming crates individually is how `f-bench` and `f-init`
    // came to have tests that nothing ran: the list stopped matching the
    // workspace the moment a crate was added, and silently.
    sh("cargo", &["test", "--workspace", "--exclude", "f-kernel"])?;
    println!(
        "\nnote: x86-64 total-store-order hides weak-memory ordering bugs.\n      \
         An AArch64 job is required before the ring tests mean anything."
    );
    Ok(())
}

fn lint_all() -> Result<(), String> {
    lint_determinism()?;
    lint_licensing()?;
    lint_unsafe()?;
    // Two invocations, because the workspace has two worlds in it. Everything
    // except the kernel is checked for the host; the kernel is checked for the
    // bare-metal target it actually runs on, which is the only configuration in
    // which checking it means anything.
    sh(
        "cargo",
        &["clippy", "--workspace", "--exclude", "f-kernel", "--all-targets", "--", "-D", "warnings"],
    )?;
    sh(
        "cargo",
        &[
            "clippy",
            "-p",
            "f-kernel",
            "--target",
            KERNEL_TARGET,
            "-Zbuild-std=core,compiler_builtins",
            "--",
            "-D",
            "warnings",
        ],
    )
}

fn rust_sources() -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !matches!(name, "target" | ".git" | "third_party" | "docs") {
                    walk(&path, out)?;
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(&root(), &mut out).map_err(|e| format!("walking the tree: {e}"))?;
    out.sort();
    Ok(out)
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root())
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn lint_determinism() -> Result<(), String> {
    let mut findings = Vec::new();

    for path in rust_sources()? {
        let rel = relative(&path);
        if is_tooling(&rel)
            || DETERMINISM_ALLOW.iter().any(|(allowed, _)| rel.starts_with(allowed))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", rel))?;

        for (line_no, line) in text.lines().enumerate() {
            // A mention in a comment is documentation, not a call site.
            let code = line.split("//").next().unwrap_or("");
            for (needle, why) in FORBIDDEN {
                if code.contains(needle) {
                    findings.push(format!("  {}:{}  {needle} — {why}", rel, line_no + 1));
                }
            }
        }
    }

    if findings.is_empty() {
        println!("lint-determinism: ok");
        return Ok(());
    }
    Err(format!(
        "determinism substrate violated in {} place(s):\n{}\n\n\
         Every source of nondeterminism must reach the system through f_env::Env.\n\
         If a new call site is genuinely legitimate, add it to DETERMINISM_ALLOW\n\
         in xtask with a reason — that is a reviewable diff, which is the point.",
        findings.len(),
        findings.join("\n")
    ))
}

fn lint_licensing() -> Result<(), String> {
    let mut missing = Vec::new();
    let mut leaked = Vec::new();

    for path in rust_sources()? {
        let rel = relative(&path);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", rel))?;

        if !text.starts_with("// SPDX-License-Identifier:") {
            missing.push(rel.clone());
        }
        // The permissive tree may not depend on an imported tree by anything
        // other than the ring protocol. See LICENSING.md.
        //
        // The SPDX check above applies everywhere, tooling included. This one
        // cannot: the checker's own source contains the string it searches for.
        if !is_tooling(&rel)
            && (text.contains("use third_party") || text.contains("third_party::"))
        {
            leaked.push(rel);
        }
    }

    let mut problems = String::new();
    if !missing.is_empty() {
        problems.push_str(&format!(
            "missing SPDX header in {} file(s):\n  {}\n",
            missing.len(),
            missing.join("\n  ")
        ));
    }
    if !leaked.is_empty() {
        problems.push_str(&format!(
            "\npermissive tree imports third_party in {} file(s):\n  {}\n\n\
             The licence boundary and the isolation boundary are the same boundary.\n\
             Imported code is reachable only over a ring. See LICENSING.md.",
            leaked.len(),
            leaked.join("\n  ")
        ));
    }

    if problems.is_empty() {
        println!("lint-licensing: ok");
        Ok(())
    } else {
        Err(problems)
    }
}

fn lint_unsafe() -> Result<(), String> {
    let mut findings = Vec::new();

    for path in rust_sources()? {
        let rel = relative(&path);
        if is_tooling(&rel) || UNSAFE_ALLOW.iter().any(|allowed| rel.starts_with(allowed)) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", rel))?;
        for (line_no, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            if code.contains("unsafe ") || code.contains("unsafe{") {
                findings.push(format!("  {}:{}", rel, line_no + 1));
            }
        }
    }

    if findings.is_empty() {
        println!("lint-unsafe: ok  (frame: {})", UNSAFE_ALLOW.join(" "));
        return Ok(());
    }
    Err(format!(
        "`unsafe` outside the frame in {} place(s):\n{}\n\n\
         The frame is {}. Everything else inherits `unsafe_code = \"forbid\"`\n\
         from the workspace. Widening the frame is a deliberate architectural\n\
         change and must go through an RFC.",
        findings.len(),
        findings.join("\n"),
        UNSAFE_ALLOW.join(", ")
    ))
}

/// One entry from `TODO.md`.
struct Task {
    id: String,
    status: char,
    size: String,
    title: String,
    needs: Vec<String>,
    epoch: String,
    standing: bool,
}

/// Does this token have the shape of a task id?
fn looks_like_id(token: &str) -> bool {
    let mut parts = token.splitn(2, '-');
    let (head, tail) = match (parts.next(), parts.next()) {
        (Some(h), Some(t)) if !h.is_empty() && !t.is_empty() => (h, t),
        _ => return false,
    };
    head.starts_with(['E', 'A'])
        && head.chars().all(|c| c.is_ascii_alphanumeric())
        && tail.chars().all(|c| c.is_ascii_alphanumeric())
        && tail.chars().any(|c| c.is_ascii_digit())
}

/// Pull task ids out of a `needs:` line, ignoring the prose around them.
fn ids_in(line: &str) -> Vec<String> {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
        .filter(|t| looks_like_id(t))
        .map(str::to_string)
        .collect()
}

fn parse_todo() -> Result<Vec<Task>, String> {
    let path = root().join("TODO.md");
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("reading TODO.md: {e}"))?;

    let mut tasks: Vec<Task> = Vec::new();
    let mut epoch = String::from("(none)");

    for line in text.lines() {
        if let Some(heading) = line.strip_prefix("## ") {
            epoch = heading.split(char::is_whitespace).next().unwrap_or(heading).to_string();
            if heading.starts_with("Always") {
                epoch = "always".to_string();
            }
            continue;
        }

        // A continuation line belongs to the task above it.
        if line.starts_with("  ") {
            if let Some(task) = tasks.last_mut() {
                let trimmed = line.trim_start();
                if let Some(rest) = trimmed.strip_prefix("*needs:*") {
                    task.needs.extend(ids_in(rest));
                } else if trimmed.starts_with("*cadence:*") {
                    task.standing = true;
                }
            }
            continue;
        }

        let Some(rest) = line.strip_prefix("- [") else { continue };
        let mut chars = rest.chars();
        let Some(status) = chars.next() else { continue };
        let Some(rest) = rest.get(1..).and_then(|r| r.strip_prefix("] ")) else { continue };
        let Some(rest) = rest.strip_prefix("**") else { continue };
        let Some((id, rest)) = rest.split_once("**") else { continue };

        // The size is the first backticked token, when there is one.
        let (size, title) = match rest.trim_start().strip_prefix('`').and_then(|r| r.split_once('`')) {
            Some((size, title)) => (size.to_string(), title.trim().to_string()),
            None => (String::from("?"), rest.trim().to_string()),
        };

        tasks.push(Task {
            id: id.to_string(),
            status,
            size,
            title,
            needs: Vec::new(),
            epoch: epoch.clone(),
            standing: false,
        });
    }

    Ok(tasks)
}

/// How many tasks each task transitively unblocks.
///
/// The number a reader cannot work out by looking: `needs:` points backwards,
/// so the file tells you what a task is waiting for and never what is waiting
/// on it. Counting forwards is what separates the critical path from the
/// twenty other things that are also, technically, available.
fn transitive_dependents<'a>(
    tasks: &'a [Task],
    by_id: &BTreeMap<&'a str, &'a Task>,
) -> BTreeMap<&'a str, usize> {
    // Reverse the edges once: dependents[x] is everything that names x.
    let mut dependents: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for task in tasks {
        for need in &task.needs {
            if let Some((id, _)) = by_id.get_key_value(need.as_str()) {
                dependents.entry(id).or_default().push(task.id.as_str());
            }
        }
    }

    let mut counts = BTreeMap::new();
    for task in tasks {
        // Breadth-first, with a seen set, so a diamond in the graph is counted
        // once and a cycle terminates instead of hanging. A cycle is a bug in
        // the file rather than a shape to support, but a planning tool that
        // hangs on a typo is worse than one that reports a number.
        let mut seen: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
        let mut queue = vec![task.id.as_str()];
        while let Some(id) = queue.pop() {
            for &next in dependents.get(id).map(Vec::as_slice).unwrap_or(&[]) {
                if seen.insert(next) {
                    queue.push(next);
                }
            }
        }
        counts.insert(task.id.as_str(), seen.len());
    }
    counts
}

/// What is ready to start, and what is waiting on what.
///
/// # Why this is a command and not a reading
///
/// `TODO.md` is a dependency graph written down as a document, and a document
/// does not answer "what can I start now" — a reader has to hold every `needs:`
/// in their head and check each one against a status marker somewhere else in
/// the file. That is exactly the kind of question a machine should answer, and
/// the answer changes every time a box is ticked.
///
/// It also validates the graph, which prose cannot: a `needs:` naming a task
/// that does not exist is a lie the file would otherwise keep telling.
fn todo_list(filter: Option<&str>) -> Result<(), String> {
    let tasks = parse_todo()?;

    let mut by_id: BTreeMap<&str, &Task> = BTreeMap::new();
    let mut duplicates = Vec::new();
    for task in &tasks {
        if by_id.insert(task.id.as_str(), task).is_some() {
            duplicates.push(task.id.clone());
        }
    }
    if !duplicates.is_empty() {
        return Err(format!(
            "duplicate task id(s) in TODO.md: {}\n\nIds are permanent and unique; \
             two tasks sharing one means a reference is ambiguous.",
            duplicates.join(", ")
        ));
    }

    let mut dangling = Vec::new();
    for task in &tasks {
        for need in &task.needs {
            if !by_id.contains_key(need.as_str()) {
                dangling.push(format!("{} needs {}, which does not exist", task.id, need));
            }
        }
    }
    if !dangling.is_empty() {
        return Err(format!(
            "TODO.md references tasks that are not in it:\n  {}",
            dangling.join("\n  ")
        ));
    }

    let done = |id: &str| by_id.get(id).is_some_and(|t| t.status == 'x' || t.status == '~');

    let mut ready = Vec::new();
    let mut blocked = Vec::new();
    let mut doing = Vec::new();
    let (mut n_done, mut n_standing) = (0usize, 0usize);

    for task in &tasks {
        if filter.is_some_and(|f| !task.epoch.eq_ignore_ascii_case(f)) {
            continue;
        }
        if task.standing {
            n_standing += 1;
            continue;
        }
        match task.status {
            'x' | '~' => n_done += 1,
            '>' => doing.push(task),
            _ => {
                let waiting: Vec<&str> =
                    task.needs.iter().filter(|n| !done(n)).map(String::as_str).collect();
                if waiting.is_empty() {
                    ready.push(task);
                } else {
                    blocked.push((task, waiting.join(", ")));
                }
            }
        }
    }

    if !doing.is_empty() {
        println!("in progress");
        for task in &doing {
            println!("  {:<8} {:<2} {}", task.id, task.size, truncate(&task.title, 62));
        }
        println!();
    }

    // Ready is not the same as next, and the difference is the whole question.
    // What a person cannot do by eye is count how much each available task
    // unblocks, so that is what this sorts by: a task holding up eleven others
    // is the critical path, and a task holding up nothing can wait for a wet
    // afternoon however urgent it feels.
    let unblocks = transitive_dependents(&tasks, &by_id);
    ready.sort_by_key(|t| (std::cmp::Reverse(unblocks.get(t.id.as_str()).copied().unwrap_or(0)), t.id.clone()));

    let critical: Vec<_> =
        ready.iter().filter(|t| unblocks.get(t.id.as_str()).copied().unwrap_or(0) > 0).collect();
    let leaves: Vec<_> =
        ready.iter().filter(|t| unblocks.get(t.id.as_str()).copied().unwrap_or(0) == 0).collect();

    println!("ready to start — {} task(s)\n", ready.len());

    if !critical.is_empty() {
        println!("  on the critical path — each of these is holding up other work");
        for task in &critical {
            let n = unblocks.get(task.id.as_str()).copied().unwrap_or(0);
            println!(
                "    {:<8} {:<2} unblocks {:>2}   {}",
                task.id,
                task.size,
                n,
                truncate(&task.title, 52)
            );
        }
        println!();
    }

    if !leaves.is_empty() {
        println!("  unblocks nothing — real work, but nothing is waiting on it");
        for task in &leaves {
            println!("    {:<8} {:<2}              {}", task.id, task.size, truncate(&task.title, 52));
        }
    }

    println!("\nwaiting — {} task(s)", blocked.len());
    for (task, on) in blocked.iter().take(12) {
        println!("    {:<8} waiting on {on}", task.id);
    }
    if blocked.len() > 12 {
        println!("    … and {} more", blocked.len() - 12);
    }

    println!("\ndone {n_done} · standing {n_standing}");
    println!(
        "\nThe four movements are not four phases. `docs/the-long-plan.html` \
         section 01 has the ordering rules; the short version is that a decision \
         belongs immediately before the work it would be expensive to redo \
         without, and nowhere earlier."
    );
    Ok(())
}

fn truncate(text: &str, max: usize) -> String {
    // Strip the markdown emphasis that makes the file readable and the terminal
    // noisy, then cut on a character boundary rather than a byte one.
    let plain: String = text.replace("**", "").replace('`', "");
    if plain.chars().count() <= max {
        return plain;
    }
    let cut: String = plain.chars().take(max - 1).collect();
    format!("{cut}…")
}

/// The value of one `key = value` line in a claim file.
///
/// Not a TOML parser, and deliberately not: the claim registry reads a handful
/// of flat scalars, and a dependency on a real parser would be the first
/// third-party crate in the tree for the sake of six fields. It knows two
/// things `split('=')` did not — that a `#` outside quotes starts a comment,
/// and that quotes come off in pairs.
///
/// What it is blind to, stated so the next person does not discover it the
/// hard way: table headers, arrays, multi-line `"""` strings, and any key
/// whose value spans lines. A claim needing those needs a parser, not a longer
/// version of this.
fn toml_scalar(line: &str) -> Option<String> {
    let (_, value) = line.split_once('=')?;

    // Tracking the quote state is the whole difference between this and
    // `split('#').next()`, which would cut `notes = "a # b"` in half.
    let mut quoted = false;
    let mut end = value.len();
    for (at, c) in value.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => {
                end = at;
                break;
            }
            _ => {}
        }
    }

    // `trim` also takes the carriage return off a CRLF file, which the claim
    // registry currently is.
    let value = value[..end].trim();
    let unquoted = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(value);
    Some(unquoted.to_string())
}

/// Find `key` in a claim file and return its value.
///
/// The key must be followed by whitespace and an `=`, so `status` is not
/// answered by `statement`. The previous `starts_with` match would have been,
/// had the two ever been ordered the other way round in the file.
fn toml_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim_start();
        let rest = line.strip_prefix(key)?;
        rest.trim_start().starts_with('=').then(|| toml_scalar(line))?
    })
}

fn claims_list() -> Result<(), String> {
    let dir = root().join("claims");
    let mut found: BTreeMap<String, String> = BTreeMap::new();

    let entries = std::fs::read_dir(&dir).map_err(|e| format!("reading claims/: {e}"))?;
    for entry in entries {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_stem().unwrap_or_default().to_string_lossy().to_string();
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            let status = toml_field(&text, "status").unwrap_or_else(|| "unknown".into());
            found.insert(name, status);
        }
    }

    if found.is_empty() {
        println!("claims: none registered yet");
        println!("        The first entry is timer jitter p99, at milestone M2.");
        return Ok(());
    }

    println!("claims:");
    for (name, status) in &found {
        println!("  {name:<32} {status}");
    }
    println!("\nEvery number published in docs/design must correspond to an entry here.");
    Ok(())
}

/// Run one claim's workload.
///
/// A `pending` claim runs its workload and reports, but does not gate — the
/// distinction matters, because a number produced before the machinery it
/// describes exists is a sanity check, not evidence.
fn claim_run(name: Option<&str>) -> Result<(), String> {
    let Some(name) = name else {
        return Err("usage: cargo xtask claim <name>   (see `cargo xtask claims`)".into());
    };

    let dir = root().join("claims");
    let file = std::fs::read_dir(&dir)
        .map_err(|e| format!("reading claims/: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| {
            p.file_stem()
                .is_some_and(|s| s.to_string_lossy().ends_with(name))
        })
        .ok_or_else(|| format!("no claim named {name} in claims/"))?;

    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let field = |key: &str| toml_field(&text, key);

    let status = field("status").unwrap_or_else(|| "unknown".into());
    let milestone = field("milestone").unwrap_or_else(|| "?".into());

    println!("claim     {name}");
    println!("status    {status}");
    println!("milestone {milestone}");
    println!("baseline  {}", field("system").unwrap_or_else(|| "unset".into()));
    println!();

    // The workload binary is named after the claim, minus the registry prefix.
    let bin = name.replace('-', "_");
    let bin = bin.strip_prefix("ring_submit_latency").map_or(bin.clone(), |_| "ring_submit".into());

    sh("cargo", &["run", "--release", "-p", "f-bench", "--bin", &bin])?;

    match status.as_str() {
        "gating" => {
            println!("\nthis claim gates the build; a regression here fails CI");
            Ok(())
        }
        "pending" => {
            println!(
                "\nstatus is `pending`: the workload ran, but the machinery this\n\
                 claim describes does not exist yet. Not evidence. Not gating."
            );
            Ok(())
        }
        _ => Ok(()),
    }
}

fn bench(name: Option<&str>) -> Result<(), String> {
    match name {
        Some(bin) => sh("cargo", &["run", "--release", "-p", "f-bench", "--bin", bin]),
        None => {
            println!("available benchmarks:");
            println!("  ring_submit    claim 0001, ring-submit-latency");
            println!("\nusage: cargo xtask bench <name>");
            Ok(())
        }
    }
}

/// Host tests with coverage instrumentation.
///
/// Wired at M0 deliberately. Fuzzing without coverage feedback is close to
/// worthless, and adding instrumentation to a mature kernel is painful — it
/// costs almost nothing while the kernel is two thousand lines.
/// See `docs/design/proving-ground.html` layer 4.
fn coverage() -> Result<(), String> {
    println!("running host tests with coverage instrumentation");
    let status = Command::new("cargo")
        .args(["test", "-p", "f-abi", "-p", "f-env", "-p", "f-ring", "-p", "f-bench"])
        .env("RUSTFLAGS", "-Cinstrument-coverage")
        .env("LLVM_PROFILE_FILE", "target/coverage/f-%p-%m.profraw")
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;

    if !status.success() {
        return Err("instrumented tests failed".into());
    }
    println!(
        "\nraw profiles in target/coverage/.\n\
         Summarise with `cargo install cargo-llvm-cov` and `cargo llvm-cov report`.\n\
         The fuzzing harness at phase 01 consumes the same instrumentation."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::toml_field;

    /// The shapes the claim registry actually contains, carriage returns
    /// included, because the file on disk has them.
    const SAMPLE: &str = concat!(
        "name       = \"ring-submit-latency\"\r\n",
        "status     = \"pending\"          # pending | tracked | gating\r\n",
        "milestone  = \"M5\"\r\n",
        "statement  = \"\"\"\r\n",
        "\r\n",
        "[hardware]\r\n",
        "runner = \"runner-class-A\"       # pinned bare metal, thermally stable\r\n",
        "notes  = \"a # inside quotes is not a comment\"\r\n",
    );

    #[test]
    fn an_inline_comment_is_not_part_of_the_value() {
        // The bug this replaced reported `pending"          # pending | tracked
        // | gating` as the status of every claim in the registry.
        assert_eq!(toml_field(SAMPLE, "status").as_deref(), Some("pending"));
    }

    #[test]
    fn a_hash_inside_quotes_is_not_a_comment() {
        assert_eq!(
            toml_field(SAMPLE, "notes").as_deref(),
            Some("a # inside quotes is not a comment")
        );
    }

    #[test]
    fn a_key_is_not_answered_by_a_longer_one() {
        assert_eq!(toml_field(SAMPLE, "state"), None);
        assert_eq!(toml_field(SAMPLE, "run"), None);
    }

    #[test]
    fn a_carriage_return_does_not_survive_into_the_value() {
        assert_eq!(toml_field(SAMPLE, "milestone").as_deref(), Some("M5"));
        assert_eq!(toml_field(SAMPLE, "name").as_deref(), Some("ring-submit-latency"));
    }

    #[test]
    fn a_missing_key_is_absent_rather_than_empty() {
        assert_eq!(toml_field(SAMPLE, "joules_per_op"), None);
    }
}
