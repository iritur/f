// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Build orchestration, and the place where written policy becomes a check
//! that can fail a build.
//!
//! Four of the commands here exist because a policy nobody can enforce is a
//! preference: `lint-determinism`, `lint-licensing`, `lint-unsafe` and
//! `lint-percpu`.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

/// The target the kernel is built for.
///
/// A built-in target and not a JSON file in `targets/`, which is a decision
/// rather than an omission. `targets/x86_64-f.json` was shipped at M0, was
/// never built by anything, and was deleted at E0-D09. Everything it stated
/// that the built-in does not is two codegen flags — `relocation-model=static`
/// and `code-model=kernel` — and those live in `.cargo/config.toml` beside the
/// paragraph explaining why the image does not link without them. One copy,
/// with the reason next to it. A second copy that no build consumes cannot
/// fail loudly: the target-spec JSON schema moves between nightlies, this tree
/// pins a nightly, and a file nothing compiles rots silently until somebody
/// switches to it and inherits a codegen configuration nobody has run.
///
/// Reverse this when the target needs something the built-in plus rustflags
/// cannot express — a different data layout, another linker flavour, a change
/// to `max-atomic-width` — or when the machine named at E5 wants a target the
/// toolchain does not ship. The build already passes `-Zbuild-std`, so a
/// custom target costs nothing extra there and the switch is this constant
/// plus a path.
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
         an instrument is. E0-B08 built the hardware Env the note here used to \
         wait for, and it is the kernel's: a counter read through kernel/src/arch. \
         This harness runs on the host, where there is still no Env of any kind, \
         so the entry stands until something on the host needs one — which is the \
         revisit condition now, in place of a milestone that has since arrived",
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

/// The tree whose mutable state must be sharded by core.
///
/// Only the kernel. A library crate's statics are that library's business and
/// are tested on the host; this is about the one program where two cores run
/// the same code over the same memory.
const PERCPU_SCOPE: &str = "kernel/";

/// Where a mutable `static` is the point rather than a violation.
const PERCPU_ALLOW: &[(&str, &str)] = &[(
    "kernel/src/percpu.rs",
    "the shard itself: `PerCpu` is the type every other mutable static has to \
     be spelled as, so it is the one place that may hold the cell",
)];

/// What a mutable `static` looks like when it is not spelled `static mut`.
///
/// Interior mutability in a `static` is global mutable state wearing a type
/// that makes it legal, and the type is the only clue. This list is names, not
/// semantics — a wrapper of its own would slip past it, which is a limit worth
/// stating rather than a hole worth pretending is closed.
///
/// Most specific first, because the first match is the one reported and
/// `UnsafeCell<T>` contains `Cell<`. The general form is last, where it catches
/// what the named ones did not.
const SHARED_STATE: &[&str] =
    &["UnsafeCell", "RefCell", "OnceCell", "OnceLock", "Mutex", "RwLock", "Atomic", "Cell<"];

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    let result = match cmd {
        "build" => build(),
        "run" => run(),
        "fault" => fault(args.get(1).map(String::as_str)),
        "user" => user(args.get(1).map(String::as_str)),
        "cap" => cap(args.get(1).map(String::as_str)),
        "init" => init_image().map(|path| println!("{}", relative(&path))),
        "mutate" => mutate(),
        "panic" => panic_path(),
        "reproduce" => match args.get(1).map(String::as_str) {
            Some("--trace") => trace_only(),
            Some(other) => Err(format!("unknown option for reproduce: {other}")),
            None => reproduce(),
        },
        "timer" => timer(args.get(1).map(String::as_str)),
        "test" => test(),
        "verify" => verify(),
        "lint" => lint_all(),
        "lint-determinism" => lint_determinism(),
        "lint-licensing" => lint_licensing(),
        "lint-unsafe" => lint_unsafe(),
        "lint-percpu" => lint_percpu(),
        "lint-mutations" => lint_mutations(),
        "lint-claims" => lint_claims(),
        "lint-units" => lint_units(),
        "lint-callbacks" => lint_callbacks(),
        "lint-claim-owners" => lint_claim_owners(),
        "lint-snapshot" => lint_snapshot(),
        "unsafe" => unsafe_report(args.get(1).map(String::as_str) == Some("--by-file")),
        "release" => release(args.get(1).map(String::as_str) == Some("--dry-run")),
        "history" => match args.get(1).map(String::as_str) {
            Some("append") => history_append(),
            Some(other) => Err(format!("unknown option for history: {other}")),
            None => history(),
        },
        "claims" => match args.get(1).map(String::as_str) {
            Some("--render") => render_claims(),
            Some(other) => Err(format!("unknown option for claims: {other}")),
            None => claims_list(),
        },
        "claim" => claim_run(args.get(1).map(String::as_str)),
        "bench" => bench(args.get(1).map(String::as_str)),
        "evals" => evals_list(),
        "eval" => eval_run(args.get(1).map(String::as_str)),
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
                     pf, ud, df, nx, wx or stack
  user [kind]        Boot into a process that violates one isolation property
                     on purpose and check the kernel survives it: kernel, null,
                     text, stack, priv, call or exit. All seven with no argument
  cap [kind]         Boot into a process that tries to escape its capabilities
                     and check the frame refuses it: grant, unowned, forge,
                     stale, rights, type, flood or unmap. All eight with no
                     argument
  init               Build user/init into the flat image the loader hands over,
                     and check it is one
  mutate             Build the kernel with a deliberate defect, boot it, and
                     require the boot to go red — then require the same boot to
                     go green without it
  timer [seconds]    Run the 1 kHz timer and print a jitter histogram. Sixty
                     seconds by default. A measurement, not an assertion
  test               Workspace tests on both x86-64 and AArch64
  verify             lint, then test, then boot, then mutate. The one command a
                     session runs to check its own work before a human is asked
                     to
  lint               Every policy check below, in order

  lint-determinism   No direct source of nondeterminism outside the allow-list
  lint-licensing     SPDX headers present; no import of third_party from the
                     permissive tree
  lint-unsafe        No `unsafe` outside the frame crates
  lint-percpu        No kernel-global mutable state outside `PerCpu`
  lint-mutations     No deliberate defect is on by default
  lint-claims        No document cites a claim value the claim no longer has
  lint-units         R03: every public abi field states its unit
  lint-callbacks     R05: no interface registers a callback
  lint-claim-owners  R09: every claim names the document that owns it
  lint-snapshot      claims/snapshot.json holds what the registry holds

  unsafe             The number A-05 reports: lines inside `unsafe` as a share
                     of the frame crates and of the whole tree, against
                     RFC 0001's under-5% target and 10% reversal trigger

  reproduce          Two runs of this commit must produce one trace hash,
                     and one unseeded read of time must break that
  reproduce --trace  Print this run's trace hash and nothing else

  panic              Three endings CI must tell apart: a clean boot, a
                     deliberate panic, and a boot that never finishes

  release --dry-run  The release manifest, and what is missing from it

  history            The measurement history, one record per commit
  history append     Add this commit's record. Run on main, never on a branch

  claims             List the claims registry, and write claims/snapshot.json
  claims --render    Rewrite every cited claim value in docs/ from the registry
  claim <name>       Run one claim's workload and report against its threshold
  bench [name]       Run a benchmark binary directly
  coverage           Host tests with coverage instrumentation

  evals              List the agent eval suite and what each task defends
  eval [name]        Run the suite, or one task, and report the pass rate
                     against the floor in evals/suite.toml

  todo [epoch]       What in TODO.md is ready to start, and what is waiting on
                     what. The list is a dependency graph, not a sequence.
"
    );
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().to_path_buf()
}

/// Where cargo puts build output, which is not always `./target`.
///
/// `CARGO_TARGET_DIR` moves it, and moving it is not an exotic configuration:
/// it is the whole performance story of the development container on Windows,
/// where `target/` lives in a named volume rather than in the bind mount
/// because a Rust target directory is tens of thousands of small files and
/// every one of them crossing the filesystem boundary is a syscall nobody
/// needs. `docker/compose.yaml` says so where it mounts it.
///
/// Assuming `./target` was wrong in three different ways and each failed
/// differently, which is why this is one function rather than three fixes.
/// `kernel_elf64` pointed at an image the build had not written, so the boot
/// failed claiming the kernel had not been built. `init_dir` put the component
/// image somewhere the loader was not told about. And the coverage run set
/// `LLVM_PROFILE_FILE` to a *relative* path, which is resolved by each test
/// binary against its own working directory rather than by cargo against the
/// workspace — so the profiles scattered into whichever crate directory the
/// harness happened to run in, and `bench/target/coverage/` in a tree whose
/// `.gitignore` only covers the root one is the evidence it left behind.
///
/// A relative `CARGO_TARGET_DIR` is resolved against the **current working
/// directory**, not against the workspace root. That is what cargo documents
/// and does, and the two differ the moment anyone runs `cargo xtask` from a
/// subdirectory — cargo would write the image one place and this would look
/// for it in another, which is the same class of failure this function exists
/// to remove. Matching cargo is the whole job; being tidier than cargo here
/// would be a second source of truth.
fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(value) if !value.is_empty() => {
            let path = PathBuf::from(value);
            if path.is_absolute() {
                path
            } else {
                std::env::current_dir().unwrap_or_else(|_| root()).join(path)
            }
        }
        _ => root().join("target"),
    }
}

fn sh(program: &str, args: &[&str]) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if status.success() { Ok(()) } else { Err(format!("{program} {} failed", args.join(" "))) }
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

/// [`capture`], with environment variables set for the child.
///
/// Separate rather than a fifth argument on `capture` because every other
/// caller wants the ambient environment, and threading an always-empty slice
/// through them would be noise at each site to save one function here.
fn capture_with(program: &str, args: &[&str], env: &[(&str, &str)]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args).envs(env.iter().copied()).current_dir(root());
    let out = command.output().map_err(|e| format!("could not run {program}: {e}"))?;
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

    let path =
        Path::new(sysroot.trim()).join("lib").join("rustlib").join(host).join("bin").join(name);

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
    target_dir().join(KERNEL_TARGET).join("debug").join("f-kernel")
}

/// The kernel, in the container format the loader will accept.
fn kernel_elf32() -> PathBuf {
    kernel_elf64().with_extension("elf32")
}

fn build() -> Result<(), String> {
    build_with(&[])
}

/// Build the kernel with one of its deliberate defects turned on.
///
/// The only caller that passes anything is [`mutate`], and the feature list is
/// a parameter rather than a flag because there will be more than one defect:
/// each property that cannot have a fixture needs a build that breaks it, and
/// they have to be buildable one at a time. A build with two defects in it is
/// caught by whichever one the boot notices first, which is the failure E0-B11
/// recorded about fixtures and which applies here for the same reason.
fn build_with(features: &[&str]) -> Result<(), String> {
    let mut args = vec![
        "build",
        "-p",
        "f-kernel",
        "--target",
        KERNEL_TARGET,
        "-Zbuild-std=core,compiler_builtins",
    ];
    let list = features.join(",");
    if !features.is_empty() {
        args.push("--features");
        args.push(&list);
    }
    sh("cargo", &args)?;
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
        Err(format!("llvm-objcopy could not rewrite {} as a 32-bit ELF", relative(&src)))
    }
}

/// Where the `init` image is built, and where the boot loader is told to find
/// it.
///
/// Its own target directory, because it is compiled with different flags from
/// everything else in the workspace — see [`init_image`] — and two sets of flags
/// sharing a target directory is two full rebuilds every time the build
/// alternates between them.
fn init_dir() -> PathBuf {
    target_dir().join("init")
}

/// The `init` image, as the loader will hand it over: a flat blob, no headers.
fn init_bin() -> PathBuf {
    init_dir().join("init.bin")
}

/// Where a component's text is mapped. `kernel::process::TEXT`.
///
/// Stated here as well as there and in `user/init/link.ld` because the three
/// are linked separately and there is nothing to share a constant through. The
/// check in [`init_image`] is what makes the duplication safe: it reads the
/// address the linker actually used.
const INIT_TEXT: u64 = 0x0040_0000;

/// How large a component's image may be.
///
/// One page, because the frame maps one page of text for it. A component that
/// outgrows this needs a loader that maps as many pages as its headers ask for,
/// which is E5 and a real ELF loader; until then the bound is real and the
/// build says so rather than the boot.
const INIT_MAX: u64 = 4096;

/// Build `user/init` into a flat image and check that it is one.
///
/// # Why this is not `cargo build`
///
/// Three reasons, and each of them is why the step exists rather than being a
/// flag on the kernel's own build.
///
/// The flags differ. The kernel is linked into the top two gibibytes and uses
/// the `kernel` code model; a component sits at four mebibytes and uses the
/// small one, and `.cargo/config.toml` cannot express both for one target. So
/// `RUSTFLAGS` is set here, which replaces the target's configured flags
/// wholesale, and the build gets a target directory of its own so the two do
/// not invalidate each other.
///
/// The crate is a library, not a binary. `user/init` forbids unsafe code, so it
/// cannot write `#[unsafe(no_mangle)]` and therefore cannot be a binary with an
/// entry point. It is linked here instead, by `user/init/link.ld`, which finds
/// the entry by the section its function was compiled into.
///
/// And the result has to be checked. Three things are asserted, all of which
/// would otherwise be discovered at boot as a process that does something
/// inexplicable: that the symbol at the image's first byte is the entry rather
/// than whatever the linker happened to place there, that there is no writable
/// data — the text page is mapped read-only, so a mutable global would fault on
/// first write — and that the whole thing fits in the one page the frame maps.
fn init_image() -> Result<PathBuf, String> {
    let lld = llvm_tool("rust-lld")?;
    let objcopy = llvm_tool("llvm-objcopy")?;
    let nm = llvm_tool("llvm-nm")?;

    let dir = init_dir();
    let target = dir.to_str().ok_or("the init target directory is not valid UTF-8")?.to_string();

    // `relocation-model=static` for the same reason the kernel uses it: this is
    // a fixed-address image, and without it the crate compiles as position
    // independent and wants a relocation table nothing will process.
    let status = Command::new("cargo")
        .args([
            "build",
            "-p",
            "f-init",
            "--target",
            KERNEL_TARGET,
            // Not `--release`. The root Cargo.toml says at length why a
            // component's image gets a profile of its own, and the short
            // version is that link-time optimisation leaves an rlib with no
            // machine code in it — which links to an empty image and looks
            // like the entry point having moved.
            "--profile",
            "init",
            "-Zbuild-std=core,compiler_builtins",
            "--target-dir",
            &target,
        ])
        .env("RUSTFLAGS", "-C relocation-model=static")
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !status.success() {
        return Err("building user/init failed".into());
    }

    let archive = dir.join(KERNEL_TARGET).join("init").join("libf_init.rlib");
    if !archive.exists() {
        return Err(format!(
            "user/init did not produce {}\n\n\
             That is the library `link.ld` links. If it is missing, the crate has\n\
             stopped being a library — see the note in user/init/Cargo.toml.",
            relative(&archive)
        ));
    }

    let elf = dir.join("init.elf");
    // One library and nothing else on the command line, which is a claim about
    // the component rather than a shortcut: everything it calls across a crate
    // boundary is `#[inline]` in `f_abi::door`, so a copy is compiled into this
    // crate and there is nothing left to resolve. If that stopped being true the
    // linker would say so — an undefined symbol is an error here, not a warning
    // — which is why this is safe to state rather than to check.
    //
    // `--whole-archive` because nothing refers to anything: the entry is called
    // by the kernel, so without it the linker would pull in no members at all
    // and produce an empty image. `--gc-sections` then takes back everything the
    // entry does not reach, and the `KEEP()` in the script is what stops it
    // taking the entry too.
    let status = Command::new(&lld)
        .args(["-flavor", "gnu", "-T", "user/init/link.ld", "--gc-sections", "--whole-archive"])
        .arg("-o")
        .arg(&elf)
        .arg(&archive)
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run rust-lld: {e}"))?;
    if !status.success() {
        return Err("linking user/init against user/init/link.ld failed".into());
    }

    // The symbol at the first byte. `link.ld` places the entry there by naming
    // the section pattern its function is compiled into; this is what says the
    // pattern still matches. A toolchain that changes how it names sections
    // makes this fail with a sentence, rather than making a boot jump into the
    // middle of some other function.
    let nm = nm.to_str().ok_or("llvm-nm's path is not valid UTF-8")?.to_string();
    let elf_path = elf.to_str().ok_or("the init elf path is not valid UTF-8")?.to_string();
    let symbols = capture(&nm, &["--defined-only", "--numeric-sort", &elf_path])?;
    let at_start: Vec<&str> = symbols
        .lines()
        .filter_map(|line| {
            let (address, rest) = line.split_once(' ')?;
            let address = u64::from_str_radix(address.trim(), 16).ok()?;
            if address == INIT_TEXT { rest.split_whitespace().nth(1) } else { None }
        })
        .collect();
    if !at_start.iter().any(|name| name.contains("9component5start")) {
        return Err(format!(
            "the first byte of the init image is not `component::start`.\n\n\
             What is there: {}\n\n\
             `user/init/link.ld` places the entry by matching the section its\n\
             function is compiled into, and the pattern has stopped matching —\n\
             most likely because the toolchain changed how it names them. The\n\
             pattern is in that file and the reasoning is in\n\
             user/init/src/component.rs.",
            if at_start.is_empty() { "nothing".to_string() } else { at_start.join(", ") }
        ));
    }

    // Nothing writable. The frame maps this page read-only and executable, so a
    // mutable global is a fault on the first write to it — with no message, in a
    // component that has no way to print one. `llvm-nm` names the section class
    // of every symbol in one letter, and four of those letters are writable.
    let writable: Vec<&str> = symbols
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let class = fields.nth(1)?;
            let name = fields.next()?;
            matches!(class, "d" | "D" | "b" | "B" | "g" | "G" | "s" | "S").then_some(name)
        })
        .collect();
    if !writable.is_empty() {
        return Err(format!(
            "the init image has writable data: {}\n\n\
             Its text page is mapped read-only, so the first write to any of these\n\
             is a page fault in a component with no way to report one. A component\n\
             that genuinely needs writable state has to be given a frame for it —\n\
             which is a capability, and which E1's quota is about.",
            writable.join(", ")
        ));
    }
    let objcopy = objcopy.to_str().ok_or("llvm-objcopy's path is not valid UTF-8")?.to_string();
    let bin = init_bin();
    let bin_path = bin.to_str().ok_or("the init image path is not valid UTF-8")?.to_string();
    capture(&objcopy, &["-O", "binary", &elf_path, &bin_path])?;

    let bytes = std::fs::metadata(&bin)
        .map_err(|e| format!("could not measure the init image: {e}"))?
        .len();
    if bytes == 0 {
        return Err("the init image is empty: the linker discarded everything".into());
    }
    if bytes > INIT_MAX {
        return Err(format!(
            "the init image is {bytes} bytes and the frame maps one page ({INIT_MAX}) for it.\n\n\
             A component that outgrows a page needs a loader that reads its headers, \n\
             which is E5. Until then this is a real bound."
        ));
    }

    Ok(bin)
}

/// Boot the kernel and return the exit status QEMU reported.
///
/// One definition of the machine, every caller. It was two copies until the
/// timer needed a third, and three copies of a machine definition is three
/// chances for a run to be compared against a differently-shaped one.
///
/// `append` is the kernel command line, and `features` are the deliberate
/// defects to build the kernel with — empty for every caller but [`mutate`].
/// When `capture` is set the serial output is collected and returned as well as
/// printed, which is what lets a caller assert on *how* a boot went wrong
/// rather than only that it did.
/// What to do with the emulator's serial output.
///
/// Three, not a bool, because the two capturing cases differ in a way a caller
/// cares about. A claim that boots ten times wants the log — it parses a line
/// out of it — and does not want two hundred lines of identical boot banner on
/// the terminal. Every other capturing caller is capturing precisely so a
/// failure can be read.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Capture {
    /// Let it go to the terminal; keep nothing.
    Off,
    /// Keep it and print it.
    Printed,
    /// Keep it and say nothing.
    Quiet,
}

/// How a run of the emulator ended.
///
/// Three outcomes, not two, because a harness that models "finished" and
/// "did not finish yet" has nowhere to put a boot that will never finish. The
/// distinction is the whole of E0-P12: CI has to be able to tell a clean exit,
/// a deliberate stop, and a hang apart from each other, and a hang is the one
/// the kernel cannot report on its own behalf.
#[derive(Debug, PartialEq, Eq)]
enum Ending {
    /// QEMU exited and reported this code.
    Exited(i32),
    /// QEMU was terminated by a signal without reporting.
    Signalled,
    /// The run outlived its budget and the harness killed it.
    TimedOut(u64),
}

impl Ending {
    /// The exit code, when there was one.
    fn code(&self) -> Option<i32> {
        match self {
            Self::Exited(code) => Some(*code),
            Self::Signalled | Self::TimedOut(_) => None,
        }
    }
}

impl fmt::Display for Ending {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Exited(code) => write!(f, "exited {code}"),
            Self::Signalled => write!(f, "terminated by a signal"),
            Self::TimedOut(seconds) => write!(f, "still running after {seconds}s, and killed"),
        }
    }
}

/// How long a boot may take before the harness stops waiting for it.
///
/// Generous, and generous on purpose. This is not a performance bound and must
/// never become one — the container emulates the machine in software, CI
/// runners are shared, and a boot that is merely slow must not be reported as a
/// hang. What it bounds is the difference between *slow* and *never*, and a
/// wrong answer in the tight direction turns a green run red for no reason.
///
/// A run that asks for a long timer window gets that window plus this, because
/// `timer=60` is sixty seconds of intended work and the budget is for
/// everything around it.
const BOOT_TIMEOUT: u64 = 180;

fn boot(append: Option<&str>) -> Result<Option<i32>, String> {
    match machine(append, &[], Capture::Off)?.0 {
        Ending::TimedOut(seconds) => Err(format!(
            "the boot was still running after {seconds}s and was killed\n\n\
             Nothing here reports a hang, so this is the harness noticing rather\n\
             than the kernel. `cargo xtask panic` exercises the same path against\n\
             a fixture that hangs on purpose."
        )),
        ending => Ok(ending.code()),
    }
}

/// [`boot`], returning how the run ended rather than only its code.
///
/// For the one caller that has to be able to assert a *timeout* happened, which
/// [`boot`] deliberately turns into an error because for every other caller a
/// hang is a failure and not a result.
fn boot_ending(append: Option<&str>, seconds: u64) -> Result<(Ending, String), String> {
    machine_with(append, &[], Capture::Printed, seconds)
}

/// [`boot`], with the serial log.
fn boot_captured(append: Option<&str>, features: &[&str]) -> Result<(Option<i32>, String), String> {
    let (ending, log) = machine(append, features, Capture::Printed)?;
    match ending {
        Ending::TimedOut(seconds) => Err(format!(
            "the boot was still running after {seconds}s and was killed\n\n\
             The log up to that point is above."
        )),
        ending => Ok((ending.code(), log)),
    }
}

/// Build a kernel and run it. The one place the emulator is described.
fn machine(
    append: Option<&str>,
    features: &[&str],
    capture: Capture,
) -> Result<(Ending, String), String> {
    machine_with(append, features, capture, BOOT_TIMEOUT)
}

/// [`machine`], capturing the log and printing none of it.
fn machine_quiet(append: Option<&str>) -> Result<(Ending, String), String> {
    machine_with(append, &[], Capture::Quiet, BOOT_TIMEOUT)
}

/// [`machine`], with a budget of its own.
///
/// Separate so that the one caller who is deliberately waiting on something
/// that will never finish can say how long it is prepared to wait, without
/// every other caller having to state a number it does not care about.
fn machine_with(
    append: Option<&str>,
    features: &[&str],
    capture: Capture,
    timeout: u64,
) -> Result<(Ending, String), String> {
    build_with(features)?;
    let kernel = kernel_elf32();
    if !kernel.exists() {
        return Err(format!("kernel image not found at {}", kernel.display()));
    }
    let init = init_image()?;

    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-kernel", kernel.to_str().ok_or("kernel path is not valid UTF-8")?]);

    // The first boot module, which from E0-B10 is `user/init`. Multiboot 1 calls
    // these modules and QEMU's own loader spells the first one `-initrd`; the
    // kernel sees a validated extent and nothing about how it arrived.
    //
    // Passed on every boot, including the ones that provoke something. The
    // provocations run the kernel's own adversary, which is a different program
    // — see `kernel::process::Plan` — and the component runs first regardless,
    // because "a second process cannot use the first one's handles" is a
    // property every boot should be checking rather than a special run.
    qemu.args(["-initrd", init.to_str().ok_or("the init image path is not valid UTF-8")?]);

    if let Some(append) = append {
        qemu.args(["-append", append]);
    }

    // Pinned, not defaulted. The kernel prints the loader's memory map, so the
    // machine's size is part of its output — and an emulator default that moves
    // between versions would move the boot log with it, quietly breaking the one
    // M0 contract that matters: the same commit produces the same run, byte for
    // byte.
    //
    // The processor model is deliberately *not* pinned, and that is worth a
    // sentence because the timer would like it to be. QEMU's TCG backend refuses
    // `tsc-deadline` and `x2apic` by name — it says so on stderr and clears the
    // bits — so asking for them buys a warning on every run and changes nothing.
    // The kernel detects what it was given and uses the mechanism that is there.
    //
    // `isa-debug-exit` turns a kernel run into something an integration test can
    // assert on: the kernel chooses its own exit code and QEMU reports it.
    //
    // `-smp 2` is pinned for the same reason the memory size is: the kernel
    // prints how many cores it started, so the machine's core count is part of
    // its output. Two rather than one because from E0-B10 the process runs on a
    // core that is not the one holding the timer, and two rather than more
    // because a second core is what makes that sentence true — every core past
    // it would be started, counted, and left with nothing to do.
    qemu.args([
        "-smp",
        "2",
        "-m",
        "128M",
        "-serial",
        "stdio",
        "-display",
        "none",
        "-device",
        "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-no-reboot",
    ]);

    qemu.current_dir(root());

    // Spawned rather than run to completion, because a boot that never ends has
    // to be a result this function can return. `status()` and `output()` both
    // wait forever, which makes a hang the harness's problem to survive rather
    // than its problem to report — and in CI it presents as a job that timed
    // out somewhere during "build", with no log and no clue.
    if capture != Capture::Off {
        qemu.stdout(Stdio::piped());
    }
    let mut child = qemu.spawn().map_err(|e| format!("could not run qemu-system-x86_64: {e}"))?;

    // The reader runs on its own thread because a piped child can fill the pipe
    // and block on a write while this thread sleeps waiting for it to exit — a
    // deadlock that only appears once the log grows past the buffer, which is to
    // say once somebody adds a line.
    let reader = child.stdout.take().map(|mut out| {
        std::thread::spawn(move || {
            let mut buffer = String::new();
            let _ = out.read_to_string(&mut buffer);
            buffer
        })
    });

    // Counted sleeps rather than a deadline read off a clock, and not in order
    // to route around the determinism lint — `xtask/` is exempt from it. It is
    // that no clock is needed here and reaching for one would be worse. Sleep
    // drift makes the real budget *at least* the nominal one, which errs
    // towards waiting too long; a deadline computed from a monotonic reading
    // can expire early on a loaded machine and call a slow boot a hang. This
    // bound separates "slow" from "never", and only one of its two failure
    // directions is survivable.
    const TICK_MS: u64 = 20;
    let mut ticks = timeout.saturating_mul(1000 / TICK_MS);

    let ending = loop {
        match child.try_wait().map_err(|e| format!("waiting for qemu: {e}"))? {
            Some(status) => break status.code().map_or(Ending::Signalled, Ending::Exited),
            None if ticks == 0 => {
                // Killed rather than left running. A harness that reports a
                // timeout and leaks the process behind it turns one hang into a
                // machine that gets slower all afternoon.
                let _ = child.kill();
                let _ = child.wait();
                break Ending::TimedOut(timeout);
            }
            None => {
                ticks -= 1;
                std::thread::sleep(Duration::from_millis(TICK_MS));
            }
        }
    };

    // Captured *and* printed. A harness that swallowed the log would be one
    // whose failures could not be read, and the whole reason to capture it is
    // to assert on a line in it.
    let log = match reader {
        Some(handle) => {
            let log = handle.join().unwrap_or_default();
            if capture == Capture::Printed {
                print!("{log}");
            }
            log
        }
        None => String::new(),
    };

    Ok((ending, log))
}

fn run() -> Result<(), String> {
    // QEMU reports (value << 1) | 1, so Success(0x10) arrives as 33.
    match boot(None)? {
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

    match boot(Some(&format!("fault={kind}")))? {
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
        Some(0) => Err("the machine reset without reporting: a fault whose handler faults \
             is a triple fault, and a triple fault has no output. Either the \
             descriptor tables are not installed or the handler cannot run \
             where it was called from."
            .into()),
        Some(other) => Err(format!("qemu exited {other}; expected 35")),
        None => Err("qemu terminated by signal".into()),
    }
}

/// The deliberate defects this kernel can be built with, and the boot each of
/// them has to be caught by.
///
/// One so far. It exists because the fifth property of the capability negative
/// suite — *a process cannot make the kernel panic by trying* — is the one that
/// cannot have a fixture of the shape the other four have: a table that panics
/// takes the machine down rather than being caught, and there is no host
/// harness for kernel logic to catch it in.
///
/// So the mutation is a build. Each entry names the feature to turn on, the
/// boot that must find it, and the sentence the log has to contain — because
/// "the boot went red" is not the assertion. A boot that went red for some
/// other reason would satisfy an exit code and prove nothing.
/// The deliberate defect that makes two runs of one commit disagree.
///
/// Separate from [`MUTATIONS`] because it is a different kind of defect and
/// belongs to a different command. Every mutation in that table makes a boot go
/// *red*, and `mutate` asserts exactly that. This one makes a boot go green
/// twice with two different answers, which no exit code and no assertion in the
/// tree can see — only a comparison of two runs can.
const TRACE_DEFECT: &str = "mutate-unseeded-time";

/// Every feature in the tree that is a deliberate defect.
///
/// One list, because `lint-mutations` has one job — no defect is ever on by
/// default — and a second list is how the second defect gets forgotten.
const DEFECTS: &[&str] = &[TRACE_DEFECT, "mutate-unchecked-index"];

/// The seed every reproduction run uses.
///
/// Fixed and stated here rather than defaulted inside the kernel, because the
/// contract is about a *pair* — `(seed, commit)` — and a pair with an implicit
/// half is a pair nobody can quote. When the kernel takes a seed on its command
/// line this becomes the value passed, and the contract does not change.
const TRACE_SEED: &str = "0xf00dbeefcafe1234";

/// A stable hash of an execution trace.
///
/// FNV-1a, and the choice is deliberate rather than lazy. What this has to be
/// is *identical on two machines at one commit*, which rules out anything the
/// standard library reserves the right to change and anything seeded per
/// process — `DefaultHasher` is both. It does not have to be
/// collision-resistant: nothing adversarial produces these traces, and a
/// content-addressed *release* is a different problem with a different answer
/// (`sha256`, E0-R01). What it has to be is written down, which is why it is
/// eight lines here rather than a dependency.
fn trace_hash(log: &str) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in log.as_bytes() {
        // Carriage returns are stripped. The serial console emits CRLF and a
        // pipe on one host may normalise where another does not, which would
        // make two identical executions hash differently for a reason that has
        // nothing to do with the kernel.
        if *byte == b'\r' {
            continue;
        }
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// One trace, printed as the single line a comparison needs.
fn trace(features: &[&str]) -> Result<u64, String> {
    let (ending, log) = machine_with(None, features, Capture::Quiet, BOOT_TIMEOUT)?;
    match ending {
        Ending::Exited(33) => Ok(trace_hash(&log)),
        other => {
            print!("{log}");
            Err(format!("the traced boot {other}; expected 33"))
        }
    }
}

/// The reproduction check.
///
/// # What is being claimed
///
/// That two runs of the same `(seed, commit)` produce a byte-identical
/// execution trace. Locally that is two boots on one machine, which is the
/// weaker half; the CI job runs the same command on two runners and compares
/// the hashes, which is the claim the task actually makes.
///
/// # Why the defect half is not optional
///
/// A reproduction check that has only ever passed is indistinguishable from one
/// that cannot fail, and this one is unusually easy to get wrong in that
/// direction: if the trace were hashed over something that does not vary — a
/// constant banner, an empty string — it would agree with itself forever. So
/// the command also builds the kernel with one unseeded read of the timestamp
/// counter on the boot path and *requires* the two runs to disagree.
///
/// That defect is worth looking at, because it is the shape of the bug this
/// whole apparatus exists for. It does not make the kernel fail. It boots,
/// every assertion holds, it prints `M0 ok`, it exits 33. Every other check in
/// this tree is green on it. The only thing wrong is that two runs no longer
/// agree — and until this command existed, nothing would ever have said so.
fn reproduce() -> Result<(), String> {
    println!("reproduction check — seed {TRACE_SEED}\n");

    println!("[1/2] the same commit, twice — the traces must agree");
    let first = trace(&[])?;
    let second = trace(&[])?;
    println!("  run 1  {first:#018x}");
    println!("  run 2  {second:#018x}");
    if first != second {
        return Err("two runs of this commit produced different traces.\n\n\
             This is the determinism contract failing, and it is the failure every\n\
             other layer of the test apparatus rests on: a seed stops being a bug\n\
             report, the simulator stops reproducing, and a claim stops being\n\
             re-derivable. Something on the boot path is reading a clock, a counter\n\
             or an address that the seeded `Env` does not own. RFC 0004."
            .to_string());
    }
    println!("\n  agreed: {first:#018x}");

    println!("\n[2/2] with one unseeded read of time — the traces must disagree");
    let a = trace(&[TRACE_DEFECT])?;
    let b = trace(&[TRACE_DEFECT])?;
    println!("  run 1  {a:#018x}");
    println!("  run 2  {b:#018x}");
    if a == b {
        return Err(format!(
            "the kernel built with `{TRACE_DEFECT}` still reproduced itself.\n\n\
             That means this check cannot fail, which makes the green result above\n\
             worth nothing. Either the defect is no longer reached on the boot path,\n\
             or the trace is being hashed over something that does not contain it."
        ));
    }
    println!("\n  disagreed, as required — the check can fail");

    println!(
        "\nreproduce: ok — {first:#018x}\n\
         Two boots on one machine. The pair that matters is two runners, and that\n\
         is the CI job: same image, same commit, same seed, hashes compared."
    );
    Ok(())
}

/// Print one trace hash and nothing else.
///
/// For the CI job, where two runners each produce a line and a third job
/// compares them. Nothing else is printed, so the artefact is the hash rather
/// than a log a comparison would have to parse.
fn trace_only() -> Result<(), String> {
    println!("{:#018x}", trace(&[])?);
    Ok(())
}

const MUTATIONS: &[(&str, &str, &str, &str)] = &[(
    "mutate-unchecked-index",
    "cap=forge",
    "KERNEL PANIC",
    "the capability table subscripts a handle's index instead of checking it",
)];

/// Build the kernel with one defect in it, boot it, and require the boot to go
/// red — then build it without and require the same boot to go green.
///
/// # Why this is a command and not a test
///
/// For the reason `fault` is: what it asserts is a *failure*, and the only
/// place a kernel failure is observable is the exit code of a machine. The
/// difference from `fault` is what is being broken. `fault` provokes the
/// hardware into a fault the kernel is supposed to report; this breaks the
/// kernel's own code and requires the suite to notice.
///
/// # Why both halves
///
/// Neither means anything alone. A red boot with a defect proves nothing if the
/// same boot is red without one — that is a broken build, not a caught defect —
/// and a green boot without a defect proves nothing about whether the suite can
/// fail. The pair is the smallest thing that is evidence, and it is the second
/// half of E0-P08's exit criterion: every property holds, *and* each has a
/// mutation that makes it fail.
///
/// The mutated boot runs first so that the tree is left holding a clean build.
fn mutate() -> Result<(), String> {
    for (feature, provocation, expected, what) in MUTATIONS {
        println!("\n--- {feature}: {what}");

        println!("\n[1/2] with the defect — the boot must go red");
        let (code, log) = boot_captured(Some(provocation), &[feature])?;
        match code {
            Some(33) => {
                return Err(format!(
                    "`{provocation}` passed on a kernel built with `{feature}`.\n\n\
                     That is the property failing rather than holding: the defect is\n\
                     {what}, and the suite did not notice. Either the boot no longer\n\
                     reaches the defect, or the check that would have caught it has\n\
                     stopped being made."
                ));
            }
            // 37 is `Exit::Panic`, which is what this mutation actually
            // produces: the defect removes a bounds check, so the boot dies in
            // the indexing rather than in a check that decided something was
            // wrong. 35 is a kernel that reported a failed assertion, and 0 is
            // a machine that reset without reporting at all — both are still
            // accepted, because a future mutation could legitimately produce
            // either and this list is meant to grow.
            //
            // Before E0-P12 a panic *was* 35, so this arm could not tell the
            // three apart. The log assertion below is what carried the whole
            // weight; it still does, and now it is not carrying it alone.
            Some(37) | Some(35) | Some(0) => {}
            Some(other) => return Err(format!("qemu exited {other}; expected a failure")),
            None => return Err("qemu terminated by signal".into()),
        }
        if !log.contains(expected) {
            return Err(format!(
                "`{provocation}` on a kernel built with `{feature}` went red, and not for\n\
                 the reason it was supposed to: the log does not contain `{expected}`.\n\n\
                 A boot that fails some other way satisfies the exit code and proves\n\
                 nothing. The serial log is above."
            ));
        }
        println!("\n{feature}: caught — the boot went red with `{expected}`");

        println!("\n[2/2] without it — the same boot must go green");
        match boot(Some(provocation))? {
            Some(33) => println!("\n{feature}: and the same boot passes without the defect"),
            Some(35) => {
                return Err(format!(
                    "`{provocation}` fails on a kernel with no defect in it, so the red\n\
                     boot above says nothing about `{feature}`. Fix the build first."
                ));
            }
            Some(other) => return Err(format!("qemu exited {other}; expected 33")),
            None => return Err("qemu terminated by signal".into()),
        }
    }

    println!("\nall {} mutation(s) caught", MUTATIONS.len());
    Ok(())
}

/// A deliberate defect must never be on by default.
///
/// The feature exists so that a build can be broken on purpose. The one way
/// that could reach an image nobody meant to break is a default feature list,
/// so this reads the manifest and refuses one — which is the same shape as the
/// other four policy lints: a rule that lives only in a comment is a rule
/// somebody edits around.
fn lint_mutations() -> Result<(), String> {
    let manifest = root().join("kernel").join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("could not read {}: {e}", relative(&manifest)))?;

    let mut in_features = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_features = trimmed == "[features]";
            continue;
        }
        if !in_features {
            continue;
        }
        let Some((name, value)) = trimmed.split_once('=') else { continue };
        if name.trim() != "default" {
            continue;
        }
        for feature in DEFECTS {
            if value.contains(feature) {
                return Err(format!(
                    "kernel/Cargo.toml has `{feature}` in its default features.\n\n\
                     That feature is a deliberate defect. It is meant to be turned on by\n\
                     `cargo xtask mutate` for exactly two boots and by nothing else; on by\n\
                     default it is a kernel that panics on a hostile handle, shipped."
                ));
            }
        }
    }

    println!("lint-mutations: ok  ({} deliberate defect(s), none on by default)", DEFECTS.len());
    Ok(())
}

/// Every isolation property a process is asked to violate, and what it is.
///
/// The list is here rather than in the kernel's help text because it is what
/// this command iterates: `cargo xtask user` runs all of them, which is the
/// form the suite is worth having in. Six of the seven must fail — that is the
/// point of them — and the seventh must not, which is what stops the other six
/// from passing for the wrong reason.
const PROVOCATIONS: &[(&str, &str)] = &[
    ("kernel", "read the kernel's direct map"),
    ("null", "write to the page at address zero"),
    ("text", "write to its own text"),
    ("stack", "execute its own stack"),
    ("priv", "run an instruction only ring 0 may"),
    ("call", "make a call the frame does not have"),
    ("exit", "nothing at all: ask to end, and be believed"),
];

/// Boot into a process that deliberately violates one isolation property, or
/// all of them in turn.
///
/// # Why this expects success where `fault` expects failure
///
/// They are opposite assertions about the same event. `cargo xtask fault`
/// provokes the *kernel* into faulting and expects the machine to die
/// reporting it, because a kernel that carries on after one of those has not
/// noticed. This provokes a *process* into faulting and expects the machine to
/// finish normally, because the entire claim of phase 00 is that a process
/// doing something it was not permitted to do is an event the frame handles
/// rather than an event it suffers.
///
/// A run that ends at 35 is therefore a failure here and a success there. The
/// kernel decides which it was, not this command: it knows which exception the
/// provocation was supposed to raise and refuses to finish if it got a
/// different one — or none.
fn user(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> =
                    PROVOCATIONS.iter().map(|(name, what)| format!("  {name:<8} {what}")).collect();
                return Err(format!("unknown provocation: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- user={name}: {what}");
        }
        match boot(Some(&format!("user={name}")))? {
            // The process ended, the kernel did not, and the kernel agreed that
            // what ended the process is what the provocation named. All three
            // are asserted in the kernel; this is the exit code that says so.
            Some(33) => println!("\nuser={name}: the process ended and the kernel did not"),
            Some(35) => {
                return Err(format!(
                    "the kernel refused to finish after `user={name}`. Either a protection \
                     did not hold — which is the failure this exists to find — or the \
                     process did not do what it was told. The serial log above says which."
                ));
            }
            Some(0) => {
                return Err(format!(
                    "the machine reset with no output during `user={name}`. A fault taken at \
                     ring 3 whose handler cannot run is a triple fault, so this is the \
                     descriptor tables, the task state segment's ring-0 stack, or the \
                     address space the handler was entered from."
                ));
            }
            Some(other) => return Err(format!("qemu exited {other}; expected 33")),
            None => return Err("qemu terminated by signal".into()),
        }
    }

    if all {
        println!("\nall {} provocations held", PROVOCATIONS.len());
    }
    Ok(())
}

/// Every capability property a process is asked to violate, and what it is.
///
/// The negative suite from `docs/design/ring-scene-boot.html` section 15, M4,
/// written as runs: *a process cannot name a capability it was not given,
/// cannot forge a handle, cannot use a revoked handle, cannot exceed granted
/// rights, and cannot make the kernel panic by trying.* That is E0-P08's exit
/// criterion and E0-B11's.
///
/// Eight rather than five. `grant` is the positive control, without which a
/// frame that refused every capability call would pass all five; `flood` is
/// what "cannot make the kernel panic" turns into once the obvious attempts
/// are refused, which is filling the table and seeing what happens at the
/// bound; and `unmap` is the one the exit criterion did not name because at M4
/// it could not be run.
///
/// `unmap` is the odd one and worth reading the kernel's side of. The other
/// seven are refused by the capability table and the process carries on. This
/// one is not refused at all: the process revokes a capability it is entitled
/// to revoke, and then reads a page that revoke withdrew — so what stops it is
/// a page fault rather than an error code. "Cannot use a revoked handle" and
/// "cannot use the memory a revoked handle mapped" are two properties, and
/// until there was a second core to shoot a translation down on, only the first
/// of them held. E0-B10.
///
/// The kernel is the judge, as with `user`. It knows exactly which refusal each
/// attempt earns and refuses to finish if it answered a different number of
/// calls, or the same number for different reasons — so a run that is turned
/// down for the wrong reason is a failed boot rather than a passing one.
const ESCAPES: &[(&str, &str)] = &[
    ("grant", "use its capabilities correctly: nothing is refused"),
    ("unowned", "name a slot the frame never filled"),
    ("forge", "sweep the handle space, in range and past it"),
    ("stale", "use a capability after the tree it hangs from was revoked"),
    ("rights", "ask for rights its capability does not carry"),
    ("type", "present a capability of the wrong kind for the operand"),
    ("flood", "derive until the table is full"),
    ("unmap", "read a page after the capability that mapped it was revoked"),
];

/// Boot into a process that tries to escape its capabilities, or all of them in
/// turn.
///
/// Every one of these must end at 33 — the process ended, the kernel did not.
/// That is the difference from `fault` and the sameness with `user`: an
/// authority escape is an event the frame handles, and a kernel that dies
/// answering one has failed the property rather than enforced it.
fn cap(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => ESCAPES.iter().collect(),
        Some(name) => {
            let found = ESCAPES.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> =
                    ESCAPES.iter().map(|(name, what)| format!("  {name:<8} {what}")).collect();
                return Err(format!("unknown capability escape: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- cap={name}: {what}");
        }
        match boot(Some(&format!("cap={name}")))? {
            Some(33) => println!("\ncap={name}: the frame answered it and the kernel did not die"),
            Some(35) => {
                return Err(format!(
                    "the kernel refused to finish after `cap={name}`. Either the frame answered \
                     a capability call it should have refused — which is the failure this exists \
                     to find — or it refused one it should have answered, which is the same \
                     failure pointing the other way. The serial log above says which."
                ));
            }
            Some(0) => {
                return Err(format!(
                    "the machine reset with no output during `cap={name}`. A capability call \
                     that faults inside the frame is the fifth property failing: a process \
                     cannot make the kernel panic by trying, and this one did."
                ));
            }
            Some(other) => return Err(format!("qemu exited {other}; expected 33")),
            None => return Err("qemu terminated by signal".into()),
        }
    }

    if all {
        println!("\nall {} capability properties held", ESCAPES.len());
    }
    Ok(())
}

/// The three endings CI has to tell apart, each produced by a fixture.
///
/// # Why all three and not just the panic
///
/// A panic assertion on its own proves nothing. If a clean boot also exited 37,
/// the assertion would pass and mean nothing; if a hang also exited 37, the
/// same. What is being established is that the three are *distinguishable*, and
/// that is a claim about a set rather than about any one member — the same
/// shape as `mutate`, where a red boot with a defect proves nothing unless the
/// same boot is green without one.
///
/// The timeout budget for the hang is deliberately small. This is the one place
/// where waiting the full [`BOOT_TIMEOUT`] would buy nothing: the fixture is
/// known to spin forever, so every second past the point the kernel has clearly
/// started is a second of gate time spent confirming it is still spinning.
const HANG_BUDGET: u64 = 20;

fn panic_path() -> Result<(), String> {
    println!("three endings, three fixtures. CI has to tell them apart.\n");

    // 1. A clean boot. The control, and the reason the other two mean anything.
    println!("--- clean: the ordinary boot ---");
    let (ending, log) = boot_ending(None, BOOT_TIMEOUT)?;
    if ending != Ending::Exited(33) {
        return Err(format!("a clean boot {ending}; expected exit 33"));
    }
    if !log.contains("M0 ok") {
        return Err("a clean boot exited 33 without reporting M0 ok".into());
    }
    println!("\nclean: exited 33, and said M0 ok");

    // 2. A panic. Exit 37, from `Exit::Panic`, and a log that names it.
    println!("\n--- panic: a deliberate panic on the boot path ---");
    let (ending, log) = boot_ending(Some("panic"), BOOT_TIMEOUT)?;
    if ending != Ending::Exited(37) {
        return Err(format!(
            "the panic fixture {ending}; expected exit 37\n\n\
             37 is `Exit::Panic`. If this reports 35 the panic handler is still\n\
             exiting with `Exit::Failure`, and CI cannot tell a panic from a\n\
             kernel that reported a failed assertion."
        ));
    }
    if !log.contains("KERNEL PANIC") {
        return Err("the panic fixture exited 37 with no panic report in the log\n\n\
             The exit code alone says a panic happened and nothing about where.\n\
             The handler is supposed to print before it exits."
            .into());
    }
    // The message, not only the banner. A handler that cannot format its
    // argument prints the banner and then nothing useful, which is the failure
    // most worth catching here — see `deliberate_stop`.
    if !log.contains("KiB reported usable") {
        return Err("the panic report reached the banner and not the message\n\n\
             The fixture panics with a formatted value precisely so that this\n\
             assertion covers the formatting machinery and not just the branch."
            .into());
    }
    println!("\npanic: exited 37, reported KERNEL PANIC, and formatted its message");

    // 3. A hang. No exit code at all, and the harness has to be the one to say
    //    so — this is the ending the kernel cannot report on its own behalf.
    println!("\n--- hang: a boot that never finishes ---");
    let (ending, _) = boot_ending(Some("hang"), HANG_BUDGET)?;
    match ending {
        Ending::TimedOut(seconds) => {
            println!("\nhang: still running after {seconds}s, killed by the harness");
        }
        other => {
            return Err(format!(
                "the hang fixture {other}; expected the harness to time out\n\n\
                 A fixture that spins forever and yet exits means either the\n\
                 fixture is not reached or something is exiting on its behalf."
            ));
        }
    }

    println!(
        "\npanic path ok — 33 clean, 37 panic, timeout hang.\n\
         Three endings, mutually distinguishable, each from a fixture."
    );
    Ok(())
}

/// Run the timer for a while and print the jitter histogram it produced.
///
/// # Why this is a command and not part of `verify`
///
/// Because it takes a minute, and because its output is a measurement. `verify`
/// asserts; this one reports. Every boot already proves the timer path works —
/// a hundred ticks, and a run that does not get all hundred fails — so what is
/// left for this command is the distribution, which nothing asserts on and
/// nothing should.
///
/// # What a number from here is worth
///
/// Not much, and the reason is in `intent/0001-the-first-timer/spec.md`. QEMU's
/// TCG backend refuses TSC-deadline outright, so the mechanism the milestone
/// names is not the one that runs here; and the APIC timer it falls back on is
/// emulated against a host clock QEMU does not control. What comes out is a
/// distribution of the emulator. `claims/0002-timer-jitter.toml` says what
/// would have to be true for a number to be quotable, and the development
/// container sets `F_ENVIRONMENT=container` so that the claims harness already
/// knows this is not it.
fn timer(seconds: Option<&str>) -> Result<(), String> {
    let seconds = seconds.unwrap_or("60");
    let parsed: u32 = seconds.parse().map_err(|_| format!("not a number of seconds: {seconds}"))?;
    if parsed == 0 {
        return Err("a zero-second measurement has nothing in it".into());
    }

    println!("booting for a {parsed}-second run at 1 kHz; this takes about that long\n");

    match boot(Some(&format!("timer={parsed}")))? {
        Some(33) => {
            println!("\ntimer ok — the histogram above is of this machine, not of a claim");
            Ok(())
        }
        Some(35) => Err("the kernel reported failure — see the serial log above".into()),
        Some(other) => Err(format!("qemu exited {other}; expected 33 or 35")),
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

    // The half of the AArch64 job that does not need an AArch64 machine.
    //
    // CI runs the tests on an arm runner, which is where the ordering means
    // anything and which nothing local substitutes for. But most of what that
    // job has ever caught is not an ordering bug at all: it is code that does
    // not *compile* off x86-64, and a compile is a compile on any host. This
    // check would have caught the one that got through — a component calling
    // through a door whose one instruction is `#[cfg(target_arch = "x86_64")]`
    // — and it costs two seconds.
    //
    // A bare-metal target rather than a hosted one, because it is the AArch64
    // target `rust-toolchain.toml` pins and so the only one guaranteed to be
    // installed. The crates checked are the four the arm job tests.
    sh(
        "cargo",
        &[
            "check",
            "-p",
            "f-abi",
            "-p",
            "f-env",
            "-p",
            "f-ring",
            "-p",
            "f-init",
            "--target",
            "aarch64-unknown-none",
        ],
    )?;

    println!(
        "\nnote: x86-64 total-store-order hides weak-memory ordering bugs.\n      \
         The AArch64 crates compile here; whether the ring's ordering holds on\n      \
         one is the arm job's to say, and nothing local substitutes for it."
    );
    Ok(())
}

/// The whole local loop, in the order that fails cheapest first.
///
/// # Why this is one command
///
/// A session that cannot check its own work sends its work to a human to be
/// checked, and the human becomes the test suite. So there has to be exactly one
/// command, it has to cover everything CI covers that can run locally, and its
/// healthy output has to be unmistakable — which is what the last line is for.
///
/// The order is deliberate: the policy lints take seconds and rule out the
/// changes that cannot merge whatever else is true of them, the tests take a
/// minute, and the boot needs QEMU. Failing in that order is failing cheapest
/// first.
///
/// It is not identical to CI. CI additionally runs the tests on AArch64, which
/// is where the ring tests mean anything, and the litmus job in release mode.
/// Nothing local can substitute for those, and pretending otherwise is worse
/// than the gap.
fn verify() -> Result<(), String> {
    lint_all()?;
    test()?;
    run()?;
    // Before `mutate`, and for the same reason `mutate` is in the loop at all.
    // Everything above this line establishes that the tree is green; these two
    // establish that a tree which was not would be *noticed*. This one covers
    // the reporting channel itself — a clean exit, a panic and a hang have to
    // arrive at CI as three different things, and a kernel cannot report the
    // third on its own behalf.
    panic_path()?;
    // The determinism contract, and the check every other layer rests on. It
    // is here rather than only in CI because the failure it catches is one
    // nothing else in this loop can see: a boot that goes green twice with two
    // different answers passes `run`, `user`, `cap` and `mutate` alike.
    reproduce()?;
    // Last, and part of the loop rather than beside it. It is the half of
    // E0-P08 that says the suite can fail: everything above proves the
    // properties hold on this tree, and this proves that a tree where one of
    // them did not would be caught. It leaves a clean build behind it.
    mutate()?;
    println!("\nverify: all green");
    println!(
        "         Local only. The AArch64 tests and the litmus job run in CI and\n         \
         cover the class of bug an x86 host cannot see."
    );
    Ok(())
}

fn lint_all() -> Result<(), String> {
    lint_determinism()?;
    lint_licensing()?;
    lint_unsafe()?;
    lint_percpu()?;
    lint_mutations()?;
    lint_claims()?;
    // The three rules from `docs/what-must-be-stated.html` section 15 that
    // could be made executable. The other nine are review, and CONTRIBUTING.md
    // says which is which — a rule listed as mechanised that is not is worse
    // than one honestly listed as review.
    lint_units()?;
    lint_callbacks()?;
    lint_claim_owners()?;
    // A generated file that is committed is a claim about the generator, and
    // the only moment it can be checked cheaply is before anything regenerates
    // it. `xtask claims` rewrites the snapshot by design, so this has to come
    // first or it grades its own homework.
    lint_snapshot()?;
    // The same check the CI policy job runs. It lives here because a local
    // `lint` that is a subset of the gate teaches people the gate is passing
    // when it is not — which is how a formatting failure reached CI on a tree
    // whose three policy lints were all green.
    sh("cargo", &["fmt", "--all", "--", "--check"])?;
    // Two invocations, because the workspace has two worlds in it. Everything
    // except the kernel is checked for the host; the kernel is checked for the
    // bare-metal target it actually runs on, which is the only configuration in
    // which checking it means anything.
    sh(
        "cargo",
        &[
            "clippy",
            "--workspace",
            "--exclude",
            "f-kernel",
            "--all-targets",
            "--",
            "-D",
            "warnings",
        ],
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
    fn walk(dir: &Path, build: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                // `build` as well as the name: with CARGO_TARGET_DIR set to
                // something inside the tree, the output directory is not
                // called `target` and every lint in this file would otherwise
                // read generated sources and report findings against them.
                if !matches!(name, "target" | ".git" | "third_party" | "docs") && path != build {
                    walk(&path, build, out)?;
                }
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    let build = target_dir();
    walk(&root(), &build, &mut out).map_err(|e| format!("walking the tree: {e}"))?;
    out.sort();
    Ok(out)
}

fn relative(path: &Path) -> String {
    path.strip_prefix(root()).unwrap_or(path).to_string_lossy().replace('\\', "/")
}

/// R03. Every quantity crossing the ABI states its unit, its epoch and its
/// zero.
///
/// # Why an explicit marker rather than a vocabulary of unit words
///
/// A vocabulary check — does this doc comment mention nanoseconds, bytes,
/// indices — passes on a sentence that happens to contain the word and fails on
/// a sentence that says the same thing differently. It is a lint that trains
/// people to include a keyword, which is worse than no lint because it looks
/// like coverage.
///
/// `Unit:` is a marker somebody has to write on purpose. It is greppable, it
/// cannot be satisfied by accident, and — the part that matters — it makes the
/// *dimensionless* case explicit too. `Unit: none` is a statement that this
/// field is an identifier rather than a quantity, which is exactly the claim
/// R03 exists to force somebody to make out loud. `deadline: u64` shipped with
/// no unit, no epoch and no zero, in the one crate whose entire purpose is to
/// be correct against code written by somebody else, and it did so because
/// nobody had to say anything.
///
/// The epoch and the zero are not separately checked, and that is a stated
/// limit rather than an oversight: they are only meaningful for some units, and
/// a lint demanding all three of an index would be teaching people to write
/// three words to get past it. What this catches is the field nobody said
/// anything about.
fn unit_findings(rel: &str, text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    for (index, line) in lines.iter().enumerate() {
        let trimmed = line.trim_start();
        // A public struct field: `pub name: Type,` at field indentation. Not a
        // `pub fn`, `pub const`, `pub struct` or `pub mod`.
        let Some(rest) = trimmed.strip_prefix("pub ") else { continue };
        if rest.starts_with("fn ")
            || rest.starts_with("const ")
            || rest.starts_with("struct ")
            || rest.starts_with("enum ")
            || rest.starts_with("mod ")
            || rest.starts_with("use ")
            || rest.starts_with("unsafe ")
            || rest.starts_with("type ")
        {
            continue;
        }
        let Some((name, _)) = rest.split_once(':') else { continue };
        let name = name.trim();
        if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
            continue;
        }

        // Walk back over this field's doc comment and attributes.
        let mut doc = String::new();
        let mut cursor = index;
        while cursor > 0 {
            cursor -= 1;
            let above = lines[cursor].trim_start();
            if above.starts_with("///") {
                doc.insert_str(0, above.trim_start_matches("///"));
                doc.insert(0, '\n');
            } else if above.starts_with('#') || above.is_empty() {
                continue;
            } else {
                break;
            }
        }

        if !doc.to_ascii_lowercase().contains("unit:") {
            findings.push(format!("  {rel}:{}  `{name}` states no unit", index + 1));
        }
    }
    findings
}

/// R05. Nothing is delivered asynchronously.
///
/// Every event is a ring entry drained at a polling point, which is what keeps
/// the determinism contract whole and is the reason this system never needs the
/// concept of async-signal-safety. A callback is the shape that quietly
/// reverses it: once one interface takes a function to call later, the argument
/// for the next one is that the first one exists.
///
/// The check is textual and names what it looks for, which bounds what it can
/// claim. A callback smuggled through a type alias or a trait object built
/// elsewhere goes past it. That is the same limit `SHARED_STATE` states about
/// itself, and it is worth stating rather than pretending is closed: this
/// catches the construct being written, not every possible spelling of it.
fn callback_findings(rel: &str, text: &str) -> Vec<String> {
    /// Spellings of "call this later", and what each one is.
    const SHAPES: &[(&str, &str)] = &[
        ("dyn Fn", "a boxed closure is a callback with a vtable"),
        ("impl Fn", "an interface taking a closure is an interface delivering asynchronously"),
        ("extern \"C\" fn", "a function pointer across the ABI is a callback the peer installs"),
        ("callback", "named as one"),
        ("register_handler", "installing a handler is installing a callback"),
        ("on_event", "an event hook is a delivery this system does not have"),
    ];

    let mut findings = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let code = line.split("//").next().unwrap_or("");
        // Public surface only. A closure inside an implementation is an
        // ordinary closure; what R05 is about is what an *interface* offers.
        if !code.contains("pub ") {
            continue;
        }
        for (shape, why) in SHAPES {
            if code.contains(shape) {
                findings.push(format!("  {rel}:{}  `{shape}` — {why}", index + 1));
            }
        }
    }
    findings
}

/// R09. Every headline claim names the subsystem that owns it.
///
/// Energy was in the first paragraph of the thesis and had no owning subsystem
/// across five design documents, which is how half a claim goes missing without
/// anybody deciding to drop it. A claim with an owner is a claim somebody can
/// be asked about.
///
/// The owning document is required to *exist*, because a citation nobody can
/// follow is the failure wearing the fix's clothes.
fn claim_owner_findings(rel: &str, text: &str) -> Vec<String> {
    let mut findings = Vec::new();

    let Some(document) = toml_field(text, "document") else {
        findings
            .push(format!("  {rel}  no [owner] document — R09, every claim names what owns it"));
        return findings;
    };
    if toml_field(text, "section").is_none() {
        findings.push(format!("  {rel}  [owner] names a document but no section"));
    }
    if !root().join(&document).exists() {
        findings.push(format!("  {rel}  [owner] cites {document}, which does not exist"));
    }
    findings
}

/// R03, over the one crate whose layout is load-bearing against code we do not
/// control.
fn lint_units() -> Result<(), String> {
    let mut findings = Vec::new();
    for path in rust_sources()? {
        let rel = relative(&path);
        if !rel.starts_with("abi/") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        findings.extend(unit_findings(&rel, &text));
    }

    if findings.is_empty() {
        println!("lint-units: ok  (every public abi field states a unit)");
        return Ok(());
    }
    Err(format!(
        "{} public field(s) in abi/ state no unit:\n{}\n\n\
         R03: every quantity crossing the ABI states its unit, its epoch and its\n\
         zero. `deadline: u64` shipped with none of the three, in the one crate\n\
         whose whole purpose is to be correct against somebody else's code.\n\n\
         Add `Unit: <what>` to the doc comment. A field that is an identifier\n\
         rather than a quantity says `Unit: none` and why — that is a claim\n\
         worth making out loud rather than a hole worth leaving.",
        findings.len(),
        findings.join("\n")
    ))
}

/// R05, over the interface crates.
fn lint_callbacks() -> Result<(), String> {
    let mut findings = Vec::new();
    for path in rust_sources()? {
        let rel = relative(&path);
        // The crates that define what a peer may ask for. `env/` is excluded
        // deliberately: `Env` is a trait the *system* implements and calls into,
        // which is dependency injection rather than delivery, and a rule that
        // could not tell the two apart would be a rule nobody could satisfy.
        if !(rel.starts_with("abi/") || rel.starts_with("ring/")) {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        findings.extend(callback_findings(&rel, &text));
    }

    if findings.is_empty() {
        println!("lint-callbacks: ok  (no interface registers a callback)");
        return Ok(());
    }
    Err(format!(
        "{} interface(s) deliver asynchronously:\n{}\n\n\
         R05: every event is a ring entry drained at a polling point. That is what\n\
         keeps the determinism contract whole, and it is why this system never\n\
         needs the concept of async-signal-safety.\n\n\
         The replacement is an opcode and a completion, not a smaller callback.",
        findings.len(),
        findings.join("\n")
    ))
}

/// R09, over the registry.
fn lint_claim_owners() -> Result<(), String> {
    let mut findings = Vec::new();
    let files = claim_files()?;
    for path in &files {
        let rel = relative(path);
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
        findings.extend(claim_owner_findings(&rel, &text));
    }

    if findings.is_empty() {
        println!("lint-claim-owners: ok  ({} claim(s) name an owner)", files.len());
        return Ok(());
    }
    Err(format!(
        "{} claim(s) name no owner:\n{}\n\n\
         R09: every headline claim names the subsystem that owns it. Energy was in\n\
         the first paragraph of the thesis and had no owning subsystem across five\n\
         design documents — which is how half a claim goes missing without anybody\n\
         deciding to drop it.\n\n\
         Add an [owner] table naming a document that exists and a section in it.",
        findings.len(),
        findings.join("\n")
    ))
}

fn lint_determinism() -> Result<(), String> {
    let mut findings = Vec::new();

    for path in rust_sources()? {
        let rel = relative(&path);
        if is_tooling(&rel) || DETERMINISM_ALLOW.iter().any(|(allowed, _)| rel.starts_with(allowed))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", rel))?;

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
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", rel))?;

        if !text.starts_with("// SPDX-License-Identifier:") {
            missing.push(rel.clone());
        }
        // The permissive tree may not depend on an imported tree by anything
        // other than the ring protocol. See LICENSING.md.
        //
        // The SPDX check above applies everywhere, tooling included. This one
        // cannot: the checker's own source contains the string it searches for.
        if !is_tooling(&rel) && (text.contains("use third_party") || text.contains("third_party::"))
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
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", rel))?;
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

/// The share of the tree RFC 0001 aims to stay under.
/// Unit: percent of code lines.
const UNSAFE_TARGET: f64 = 5.0;

/// The share at which RFC 0001 says the partition is not real and the decision
/// reverses.
/// Unit: percent of code lines.
const UNSAFE_REVERSAL: f64 = 10.0;

/// Report lines inside `unsafe` as a share of the code that could contain them.
///
/// `A-05` reports this number every release and, until this verb existed, the
/// method was whoever remembered — a rule kept by attention, which is exactly
/// what `lint-unsafe` exists to prevent for the same policy. A number nobody
/// can recompute is not a measurement, it is a memory.
///
/// # What counts
///
/// A **code line** is a line with something left on it once comments, string
/// literals and character literals have been taken out. Blank and comment-only
/// lines are not code: counting them would make this number improve every time
/// somebody wrote a paragraph, and this tree is written to be argued with.
///
/// A code line is **inside `unsafe`** when it is in the body of an `unsafe`
/// block or an `unsafe fn`, or is an `unsafe impl` or `unsafe trait` header.
/// The line that opens a block counts, because `unsafe { *p }` is the whole
/// obligation on one line; so does a signature, because `unsafe fn` is a
/// promise the caller has to keep and the reader has to read.
///
/// # Why a scanner and not a parser
///
/// Because the parser would be a syntax crate, in the one tool whose job is to
/// police what the tree depends on, for a number that changes by tenths. The
/// scanner understands comments, strings, raw strings, and the difference
/// between `'a'` and the lifetime in `Producer<'m>` — the four things that make
/// a naive `grep` wrong. It does not understand macros that expand to `unsafe`,
/// and there are none.
///
/// *Reversal:* the first time this number is disputed at a boundary that
/// matters — a release argument, or a fallback's cost at `E5-D02` — replace it
/// with a real parse and record the difference between the two answers.
///
/// # Why this reports and does not gate
///
/// Because RFC 0001's trigger is *"exceeds 10% of the codebase by phase 02"*,
/// and the phase is half the condition. At phase 00 this tree is a kernel and
/// almost nothing else, so the share is high and says little; what the trigger
/// is about is the trajectory once E1 puts drivers above the frame. A verb that
/// went red today would be a gate with no path to green, and this file's own
/// history says what happens to those. `A-05` is where the number is argued,
/// every release, out loud.
///
/// *Reversal:* phase 02, where the same number stops being a trajectory and
/// becomes the verdict RFC 0001 describes. At that point this becomes a gate
/// and `lint_all` gains a line.
///
/// # Errors
///
/// Only if a file outside the frame contains `unsafe`, which `lint-unsafe`
/// refuses first — two tools disagreeing about where the frame is would make
/// this number quietly wrong rather than loudly.
fn unsafe_report(by_file: bool) -> Result<(), String> {
    // One row per frame crate, in the order the allow-list names them, so the
    // report and the policy cannot drift into two different lists.
    let mut rows: Vec<(&str, usize, usize)> =
        UNSAFE_ALLOW.iter().map(|crate_dir| (*crate_dir, 0, 0)).collect();
    let (mut frame_unsafe, mut frame_code) = (0usize, 0usize);
    let (mut tree_unsafe, mut tree_code) = (0usize, 0usize);
    let mut files: Vec<(String, usize, usize)> = Vec::new();

    for path in rust_sources()? {
        let rel = relative(&path);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        let (inside, code) = unsafe_share(&text);

        if inside != 0 {
            files.push((rel.clone(), inside, code));
        }
        tree_unsafe += inside;
        tree_code += code;

        if let Some(row) = rows.iter_mut().find(|(dir, _, _)| rel.starts_with(dir)) {
            row.1 += inside;
            row.2 += code;
            frame_unsafe += inside;
            frame_code += code;
        } else if inside != 0 {
            // `lint-unsafe` is the check that this cannot happen. Saying so
            // here rather than silently adding the lines to the denominator:
            // two tools disagreeing about where the frame is would make this
            // number quietly wrong rather than loudly.
            return Err(format!(
                "{rel} has {inside} line(s) inside `unsafe` outside the frame.\n\
                 `cargo xtask lint-unsafe` should have refused this first."
            ));
        }
    }

    println!(
        "unsafe: {} of the frame, {} of the tree",
        pct(frame_unsafe, frame_code),
        pct(tree_unsafe, tree_code)
    );
    println!();

    if by_file {
        // Sorted by how much each file contributes, because A-05 asks why the
        // number moved and the answer is almost always one file.
        files.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        for (rel, inside, code) in &files {
            println!("  {:<40} {:>5} of {:>5}  {:>6}", rel, inside, code, pct(*inside, *code));
        }
        println!();
    }

    for (dir, inside, code) in &rows {
        println!(
            "  {:<10} {:>6} of {:>6} code line(s)  {:>6}",
            dir,
            inside,
            code,
            pct(*inside, *code)
        );
    }
    println!(
        "  {:<10} {:>6} of {:>6} code line(s)  {:>6}",
        "frame",
        frame_unsafe,
        frame_code,
        pct(frame_unsafe, frame_code)
    );
    println!(
        "  {:<10} {:>6} of {:>6} code line(s)  {:>6}",
        "tree",
        tree_unsafe,
        tree_code,
        pct(tree_unsafe, tree_code)
    );
    println!();

    let share = share_of(tree_unsafe, tree_code);
    if share < UNSAFE_TARGET {
        println!("  Under RFC 0001's {UNSAFE_TARGET}% target. The reversal trigger is");
        println!("  {UNSAFE_REVERSAL}% by phase 02. A-05 reports this at every release.");
        return Ok(());
    }

    println!(
        "  Over RFC 0001's {UNSAFE_TARGET}% target, and over the {UNSAFE_REVERSAL}% figure in"
    );
    println!("  its reversal condition — which is a phase-02 condition and not");
    println!("  a phase-00 one, so this is a trajectory and not yet a verdict.");
    println!();
    println!("  What the number is made of matters more than the number: this");
    println!("  tree is almost entirely a kernel, and page tables, an APIC and");
    println!("  port I/O are unsafe nearly line for line. The denominator is what");
    println!("  E1 changes, by putting drivers above the frame. A-05 reports this");
    println!("  at every release; the thing to say is which way it moved and why.");
    Ok(())
}

/// The share, as a number. Zero code is zero share and never a division.
fn share_of(part: usize, whole: usize) -> f64 {
    if whole == 0 { 0.0 } else { part as f64 * 100.0 / whole as f64 }
}

/// The share, as something to print.
fn pct(part: usize, whole: usize) -> String {
    format!("{:.1}%", share_of(part, whole))
}

/// Code lines inside `unsafe`, and code lines, for one file.
///
/// The brace depth is tracked rather than the text indented-matched, because a
/// closing brace on a line of its own is the ordinary case and a text match
/// would end the region at the first one whatever it closed.
fn unsafe_share(text: &str) -> (usize, usize) {
    let mut code = 0usize;
    let mut inside = 0usize;
    let mut depth: i32 = 0;
    // The depths at which an `unsafe` region opened. A stack, because an
    // `unsafe fn` may contain an `unsafe` block, and the inner one closing does
    // not end the outer one.
    let mut opened: Vec<i32> = Vec::new();
    // An `unsafe` keyword seen but not yet followed by its brace. Kept across
    // lines, because a signature may wrap; cleared at a `;`, because
    // `unsafe fn f();` in a trait has no body to attach to.
    let mut pending = false;
    let mut carry = Carry::default();

    for line in text.lines() {
        let bare = strip_to_code(line, &mut carry);
        let bare = bare.trim();
        if bare.is_empty() {
            continue;
        }
        code += 1;

        let mut here = !opened.is_empty();
        let bytes = bare.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'{' => {
                    depth += 1;
                    if pending {
                        opened.push(depth);
                        pending = false;
                        here = true;
                    }
                }
                b'}' => {
                    if opened.last() == Some(&depth) {
                        opened.pop();
                    }
                    depth -= 1;
                }
                b';' => pending = false,
                _ => {
                    if bytes[i..].starts_with(b"unsafe") && is_word(bytes, i, 6) {
                        // `#[unsafe(no_mangle)]` is the 2024 attribute form and
                        // opens no block. Reading it as one attached the next
                        // brace in the file — the body of the function being
                        // annotated — and counted `kmain` entire. Found by the
                        // number itself: the kernel came out at 32%, which was
                        // the first thing about the answer that looked wrong.
                        if bytes.get(i + 6) == Some(&b'(') {
                            i += 6;
                            continue;
                        }
                        pending = true;
                        here = true;
                        i += 6;
                        continue;
                    }
                }
            }
            i += 1;
        }

        if here {
            inside += 1;
        }
    }

    (inside, code)
}

/// Whether the `len` bytes at `at` stand alone as a word.
///
/// Without this, `unsafe_code` and `unsafe_op_in_unsafe_fn` in attributes read
/// as the keyword, and every crate that suppresses a lint about `unsafe` would
/// be counted as using it.
fn is_word(bytes: &[u8], at: usize, len: usize) -> bool {
    let before = at == 0 || !is_word_byte(bytes[at - 1]);
    let after = bytes.get(at + len).is_none_or(|b| !is_word_byte(*b));
    before && after
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// What a line is in the middle of when the one before it ended.
///
/// Both halves are load-bearing and the second was found the hard way: this
/// file's own error messages are string literals continued across lines with a
/// trailing backslash, and one of them contains the words `lint-unsafe`. A
/// scanner that started each line fresh read that as the keyword, in the tool
/// whose whole job is to say where the keyword is.
#[derive(Clone, Copy, Default)]
struct Carry {
    comment: bool,
    string: Option<Quote>,
}

/// The kind of string a line ended inside.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Quote {
    /// An ordinary literal, where a backslash escapes the next byte.
    Escaped,
    /// A raw literal closed by a quote and this many `#`, where it does not.
    Raw(usize),
}

/// One line with its comments, strings and character literals removed.
///
/// A quoted brace is not a brace and a commented `unsafe` is not one either.
/// Every case here is one this tree actually contains: `kprintln!` format
/// strings carry braces, doc comments carry the word, string literals run over
/// several lines, and `Producer<'m>` is the lifetime that a naive
/// character-literal rule would read as an unterminated quote and swallow the
/// rest of the line for.
fn strip_to_code(line: &str, carry: &mut Carry) -> String {
    let mut out = String::with_capacity(line.len());
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if carry.comment {
            if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                carry.comment = false;
                i += 2;
            } else {
                i += 1;
            }
            continue;
        }

        if let Some(quote) = carry.string {
            match run_to_close(bytes, i, quote) {
                Some(end) => {
                    carry.string = None;
                    i = end;
                    // A placeholder, so a line that is only a string is still
                    // code rather than blank.
                    out.push('"');
                }
                None => return out,
            }
            continue;
        }

        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => break,
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                carry.comment = true;
                i += 2;
            }
            b'r' if !word_byte_before(bytes, i) && raw_string_hashes(bytes, i).is_some() => {
                let hashes = raw_string_hashes(bytes, i).unwrap_or(0);
                let opened = i + 1 + hashes + 1;
                match run_to_close(bytes, opened, Quote::Raw(hashes)) {
                    Some(end) => i = end,
                    None => {
                        carry.string = Some(Quote::Raw(hashes));
                        out.push('"');
                        return out;
                    }
                }
                out.push('"');
            }
            b'"' => match run_to_close(bytes, i + 1, Quote::Escaped) {
                Some(end) => {
                    i = end;
                    out.push('"');
                }
                None => {
                    carry.string = Some(Quote::Escaped);
                    out.push('"');
                    return out;
                }
            },
            b'\'' => match char_literal_end(bytes, i) {
                Some(end) => {
                    i = end;
                    out.push('\'');
                }
                // A lifetime. The quote is not an opening quote and the rest of
                // the line is ordinary code.
                None => {
                    i += 1;
                }
            },
            b => {
                out.push(b as char);
                i += 1;
            }
        }
    }

    out
}

/// Where a string open at `from` closes on this line, or `None` if it does not.
fn run_to_close(bytes: &[u8], from: usize, quote: Quote) -> Option<usize> {
    let mut i = from;
    while i < bytes.len() {
        match quote {
            Quote::Escaped if bytes[i] == b'\\' => {
                // A backslash at the end of the line is a continuation, and the
                // literal carries on. Stepping over two bytes takes the index
                // past the end, which is the same answer.
                i += 2;
                continue;
            }
            Quote::Escaped if bytes[i] == b'"' => return Some(i + 1),
            Quote::Raw(hashes)
                if bytes[i] == b'"' && (1..=hashes).all(|k| bytes.get(i + k) == Some(&b'#')) =>
            {
                return Some(i + 1 + hashes);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn word_byte_before(bytes: &[u8], at: usize) -> bool {
    at > 0 && is_word_byte(bytes[at - 1])
}

/// The number of `#` in a raw string starting at `at`, if one starts there.
fn raw_string_hashes(bytes: &[u8], at: usize) -> Option<usize> {
    let mut j = at + 1;
    let mut hashes = 0;
    while bytes.get(j) == Some(&b'#') {
        hashes += 1;
        j += 1;
    }
    if bytes.get(j) == Some(&b'"') { Some(hashes) } else { None }
}

/// Where a character literal starting at `at` ends, or `None` for a lifetime.
fn char_literal_end(bytes: &[u8], at: usize) -> Option<usize> {
    if bytes.get(at + 1) == Some(&b'\\') {
        // An escape: scan for the closing quote, which is at most a few bytes
        // away — `'\u{1F600}'` is the longest form.
        let window = bytes.len().min(at + 12);
        let closing = bytes[at + 2..window].iter().position(|b| *b == b'\'');
        return closing.map(|offset| at + 2 + offset + 1);
    }
    if bytes.get(at + 2) == Some(&b'\'') { Some(at + 3) } else { None }
}

/// No kernel-global mutable state outside `PerCpu`.
///
/// # What this enforces and why it is a lint
///
/// `docs/design/ring-scene-boot.html` section 14: all kernel state is per-CPU
/// from the very first allocation, behind a `PerCpu<T>`, even while only one
/// core is running — because retrofitting the shard onto state that is already
/// reached as a global is a refactor that touches every call site, and it
/// arrives on the same day as the first SMP bug.
///
/// A decision like that is kept by nobody unless something fails when it is
/// broken. The failure mode without this check is not a bad review: it is a
/// `static mut` added at three in the morning that works perfectly on one core
/// and is discovered at M3, when the second one starts and the symptom is
/// memory corruption rather than a compile error.
///
/// # What it cannot see
///
/// Names. A `static` holding a type that wraps an `UnsafeCell` under some other
/// identifier is invisible here, as is state hidden behind a pointer into
/// memory the allocator handed out. This is a check on the spelling that makes
/// global mutable state *legal*, which is the spelling every accidental
/// instance of it uses.
fn lint_percpu() -> Result<(), String> {
    let mut findings = Vec::new();

    for path in rust_sources()? {
        let rel = relative(&path);
        if !rel.starts_with(PERCPU_SCOPE)
            || PERCPU_ALLOW.iter().any(|(allowed, _)| rel.starts_with(allowed))
        {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", rel))?;

        for (line_no, line) in text.lines().enumerate() {
            let code = line.split("//").next().unwrap_or("");
            // `&'static mut T` is a reference with a lifetime, not a mutable
            // global, and this lint reported one as the other the first time
            // the kernel wrote a function returning one. Stripping the lifetime
            // before looking is the narrowest fix: a textual lint cannot parse,
            // so what it can do is know the one spelling that is not what it is
            // looking for.
            let code = code.replace("'static ", "");
            let code = code.as_str();
            if code.contains("static mut ") {
                findings.push(format!("  {}:{}  `static mut`", rel, line_no + 1));
                continue;
            }
            // Only the declaration, so that a `static` holding a `PerCpu<T>`
            // whose *slot* type contains a cell is not reported: the slot is
            // private to one core, which is the whole point.
            let Some(declaration) = code.split_once("static ") else { continue };
            if declaration.1.contains("PerCpu<") {
                continue;
            }
            // The first match, not every match: one static is one finding, and
            // a count that says four when three lines are wrong is a lint
            // nobody trusts the second time.
            if let Some(name) = SHARED_STATE.iter().find(|name| declaration.1.contains(*name)) {
                findings.push(format!("  {}:{}  `static` holding {name}", rel, line_no + 1));
            }
        }
    }

    if findings.is_empty() {
        println!("lint-percpu: ok  (mutable kernel state is sharded)");
        return Ok(());
    }
    Err(format!(
        "kernel-global mutable state in {} place(s):\n{}\n\n\
         Kernel state is per-CPU from the first allocation, behind `PerCpu<T>` —\n\
         see kernel/src/percpu.rs and ring-scene-boot section 14. Two cores\n\
         never reach the same slot, which is why nothing here needs a lock.\n\
         A static that genuinely must be shared is an architectural change and\n\
         needs an RFC, not an allow-list entry.",
        findings.len(),
        findings.join("\n")
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
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading TODO.md: {e}"))?;

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
        let (size, title) =
            match rest.trim_start().strip_prefix('`').and_then(|r| r.split_once('`')) {
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
    ready.sort_by_key(|t| {
        (std::cmp::Reverse(unblocks.get(t.id.as_str()).copied().unwrap_or(0)), t.id.clone())
    });

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
            println!(
                "    {:<8} {:<2}              {}",
                task.id,
                task.size,
                truncate(&task.title, 52)
            );
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

/// One line of the release manifest.
struct Content {
    /// What the release contract calls it.
    name: &'static str,
    /// Where it comes from, when it exists.
    source: ContentSource,
    /// The task that produces it, when it does not exist yet.
    owed_to: Option<&'static str>,
}

/// Where a manifest entry's content comes from.
enum ContentSource {
    /// A file in the tree.
    File(&'static str),
    /// Every file under a directory with this extension.
    Tree(&'static str, &'static str),
    /// Produced by the build rather than read from the tree.
    Built(&'static str),
    /// Nothing produces it yet.
    Absent,
}

/// The eight things a release contains.
///
/// In the order `docs/the-long-plan.html` section 08 lists them, and named the
/// way it names them, so the table there and this list can be read against each
/// other. An entry disappearing from one and not the other is the failure this
/// ordering exists to make obvious.
const CONTENTS: &[Content] = &[
    Content {
        name: "the source, at a tag",
        source: ContentSource::Built("git archive of HEAD"),
        owed_to: None,
    },
    Content {
        name: "the claims snapshot",
        source: ContentSource::File("claims/snapshot.json"),
        owed_to: None,
    },
    Content {
        name: "the baseline configuration",
        source: ContentSource::Absent,
        owed_to: Some(
            "E1-D06. claims/0001 names `linux-6.x-tuned` in prose, which is exactly \
             the decay the contract warns about: prose ages into a stock \
             comparison without anybody deciding it should, because prose \
             cannot be re-run",
        ),
    },
    Content {
        name: "the seed corpus and scenario set",
        source: ContentSource::Absent,
        owed_to: Some("E1-P01, the deterministic simulator, and E1-P03's sweeps"),
    },
    Content {
        name: "a content-addressed system image",
        source: ContentSource::Built("target/<target>/debug/f-kernel.elf32"),
        owed_to: None,
    },
    Content {
        name: "the dependency manifest and provenance",
        source: ContentSource::File("Cargo.lock"),
        owed_to: None,
    },
    Content {
        name: "the honest-status page",
        source: ContentSource::File("docs/TESTING-STATUS.md"),
        owed_to: None,
    },
    Content {
        name: "the decision record",
        source: ContentSource::Tree("docs/rfc", "md"),
        owed_to: None,
    },
];

/// `sha256sum`, when the machine has it.
///
/// Not a hard requirement and not vendored. The release *package* is content
/// addressed and E0-R01 owns producing it; what this command needs a hash for
/// is to show that the image it would ship is a specific one. A machine without
/// coreutils gets the manifest with the hash column absent and a line saying
/// why, which is more useful than either refusing to run or growing a hash
/// implementation in the build tooling.
fn sha256(path: &Path) -> Option<String> {
    let out = Command::new("sha256sum").arg(path).current_dir(root()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()?.split_whitespace().next().map(str::to_string)
}

fn release(dry_run: bool) -> Result<(), String> {
    if !dry_run {
        return Err("`cargo xtask release` builds the package, and that is E0-R01.\n\n\
             What exists today is `cargo xtask release --dry-run`, which prints the\n\
             manifest this would produce and names the task owing each missing\n\
             piece. RELEASING.md states the contract the package has to satisfy."
            .into());
    }

    // Not `unwrap_or("unknown")`, which is what these two were. The version and
    // the commit are the only fields saying *which tree this is*; a manifest
    // printing `unknown` for both is not a degraded manifest, it is a confident
    // statement about nothing. That degradation was live rather than
    // theoretical — git refuses a container's foreign-owned working tree, so
    // this job would have printed a clean-looking manifest for an unidentified
    // tree, and passed.
    let identify = |what: &str, args: &[&str]| {
        capture("git", args).map(|out| out.trim().to_string()).map_err(|e| {
            format!(
                "cannot read the {what} from git: {e}\n\n\
                 A release manifest names the tree it describes, so this is fatal rather \
                 than `unknown`.\n\
                 In a container this is usually git refusing a working tree owned by \
                 another uid. docker/Dockerfile marks the tree safe; an image built \
                 before that does not."
            )
        })
    };
    let describe = identify("version", &["describe", "--tags", "--always", "--dirty"])?;
    let commit = identify("commit", &["rev-parse", "HEAD"])?;

    println!("release manifest (dry run)\n");
    println!("  version   {describe}");
    println!("  commit    {commit}");
    println!("  contract  RELEASING.md\n");

    let mut missing = 0usize;
    for content in CONTENTS {
        match &content.source {
            ContentSource::File(path) => {
                let full = root().join(path);
                if full.exists() {
                    let size = std::fs::metadata(&full).map(|m| m.len()).unwrap_or(0);
                    let hash = sha256(&full).unwrap_or_else(|| "-".into());
                    println!("  [ok]  {:<36} {path}", content.name);
                    println!("        {size} bytes  sha256 {}", &hash[..hash.len().min(16)]);
                } else {
                    missing += 1;
                    println!("  [--]  {:<36} {path} does not exist", content.name);
                }
            }
            ContentSource::Tree(dir, extension) => {
                let count = std::fs::read_dir(root().join(dir))
                    .map(|entries| {
                        entries
                            .filter_map(Result::ok)
                            .filter(|e| e.path().extension().is_some_and(|x| x == *extension))
                            .count()
                    })
                    .unwrap_or(0);
                if count == 0 {
                    missing += 1;
                    println!("  [--]  {:<36} {dir}/ is empty", content.name);
                } else {
                    println!("  [ok]  {:<36} {dir}/  {count} file(s)", content.name);
                }
            }
            ContentSource::Built(how) => {
                println!("  [ok]  {:<36} built: {how}", content.name);
            }
            ContentSource::Absent => {
                missing += 1;
                println!("  [--]  {:<36} nothing produces this yet", content.name);
                if let Some(owed) = content.owed_to {
                    println!("        owed to {owed}");
                }
            }
        }
    }

    println!("\n  {} of {} contents present", CONTENTS.len() - missing, CONTENTS.len());

    // The gates, listed rather than run. Running `verify` from inside a dry run
    // would make a manifest print cost a full boot, and the point of this
    // command is that it is cheap enough to run while thinking.
    println!(
        "\nwhat would stop this release (RELEASING.md, and not checked here):\n\
        \x20 1. `cargo xtask verify` not green\n\
        \x20 2. a gating claim red, or a claim with no reproduction from a clean checkout\n\
        \x20 3. a document number `cargo xtask lint` cannot trace to the registry\n\
        \x20 4. docs/TESTING-STATUS.md not re-read against the tree\n\
        \x20 5. an RFC reversed this cycle that was edited rather than superseded"
    );

    if missing > 0 {
        println!(
            "\nThis is not yet a releasable package, and the missing rows say why.\n\
             E0-R01 builds it; E0-R04 is release 0.1."
        );
    }
    Ok(())
}

/// The measurement history.
///
/// # Why a file in the tree, and why `main` writes it
///
/// The task this satisfies asks for a history that survives a rebase, and that
/// requirement rules out the obvious design. A history every branch appends to
/// conflicts on every rebase — each side has added a line at the end of the
/// same file — and worse, a rebase *rewrites* the commits those lines name, so
/// the surviving history refers to objects that no longer exist.
///
/// So branches do not write it. `cargo xtask history append` is run by the
/// post-merge job on `main`, against a commit that is already permanent. A
/// feature branch can be rebased any number of times without touching this
/// file, because it never had a line in it to conflict over.
///
/// The cost, stated: a measurement taken on a branch is not in the history
/// until that branch merges. That is the right way round — a number from a
/// commit that was later rewritten is a number about a tree nobody has.
fn history_path() -> PathBuf {
    root().join("claims").join("history.jsonl")
}

/// The schema version of a history line.
///
/// Written into every record because this file is meant to be read years later
/// by change-point detection that does not exist yet, and the one thing such a
/// reader cannot recover is what an old line meant. Bumping this is how a
/// format change stays readable rather than silently reinterpreted.
const HISTORY_SCHEMA: u32 = 1;

fn history() -> Result<(), String> {
    let path = history_path();
    let Ok(text) = std::fs::read_to_string(&path) else {
        println!("history: nothing recorded yet ({})", relative(&path));
        println!(
            "\n`cargo xtask history append` writes one record for the current commit.\n\
             It is run by CI on main, not on a branch: see the note in xtask."
        );
        return Ok(());
    };

    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    println!("history: {} record(s) in {}", lines.len(), relative(&path));
    for line in lines.iter().rev().take(10).rev() {
        println!("  {line}");
    }
    if lines.len() > 10 {
        println!("  … {} earlier record(s)", lines.len() - 10);
    }
    println!(
        "\nAppend-only. Change-point detection at phase 02 reads this; until then\n\
         its job is to exist from the first measurement rather than from the\n\
         first time somebody wants a trend."
    );
    Ok(())
}

/// Append one record for the current commit.
fn history_append() -> Result<(), String> {
    let commit = capture("git", &["rev-parse", "HEAD"])?.trim().to_string();

    // What this run was actually allowed to record. A history that stored
    // refused measurements as zeroes or as absent-without-comment would be a
    // history whose gaps are unreadable later, and the gaps are the part a
    // change-point detector most needs to not trip over.
    let environment = std::env::var("F_ENVIRONMENT").unwrap_or_else(|_| "unset".into());

    let mut record = format!(
        "{{\"schema\":{HISTORY_SCHEMA},\"commit\":\"{commit}\",\"environment\":\"{environment}\""
    );

    // Coverage, when this run produced it. Not a timing measurement, so no
    // environment refuses it — a line count is the same on any machine, which
    // is exactly why it is the one thing a shared CI runner can contribute.
    let coverage = target_dir().join("coverage").join("summary.json");
    match std::fs::read_to_string(&coverage) {
        Ok(text) => {
            let percent = text
                .split("\"total\"")
                .nth(1)
                .and_then(|rest| rest.split("\"percent\":").nth(1))
                .and_then(|rest| rest.split('}').next())
                .map(|value| value.trim().to_string());
            match percent {
                Some(percent) => record.push_str(&format!(",\"coverage_percent\":{percent}")),
                None => record.push_str(",\"coverage_percent\":null"),
            }
        }
        Err(_) => record.push_str(",\"coverage_percent\":null"),
    }

    // Every distribution this run was permitted to write. In a refusing
    // environment there are none, and the record says so by carrying an empty
    // list rather than by being absent.
    let mut claims: Vec<String> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(root().join("claims")) {
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.to_string_lossy().ends_with(".local.jsonl"))
            .collect();
        files.sort();
        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else { continue };
            // The header line, which carries the summary. The bucket lines stay
            // in the local file: a history that inlined every distribution
            // would be megabytes of a file whose whole value is being readable.
            if let Some(header) = text.lines().next() {
                claims.push(header.to_string());
            }
        }
    }
    record.push_str(&format!(",\"claims\":[{}]}}\n", claims.join(",")));

    let path = history_path();
    let mut existing = std::fs::read_to_string(&path).unwrap_or_default();
    if !existing.is_empty() && !existing.ends_with('\n') {
        existing.push('\n');
    }
    existing.push_str(&record);
    std::fs::write(&path, existing).map_err(|e| format!("writing {}: {e}", relative(&path)))?;

    print!("appended to {}:\n  {record}", relative(&path));
    if claims.is_empty() {
        println!(
            "\nNo distributions in this record. `{environment}` is not a measurement\n\
             environment, so nothing timing-related was permitted to be written —\n\
             E0-P15. The record still exists, because a gap that is stated is\n\
             something a trend can reason about and a gap that is missing is not."
        );
    }
    Ok(())
}

/// Where the machine-readable snapshot of the registry is written.
///
/// In `claims/` beside the entries rather than in the build directory: it is a
/// statement about the registry, and `cargo clean` should not be able to delete
/// the answer to "what did this commit claim". It is generated, so it is
/// regenerated rather than edited, and `xtask lint` fails when it is stale.
fn snapshot_path() -> PathBuf {
    root().join("claims").join("snapshot.json")
}

/// Every claim file, in registry order.
fn claim_files() -> Result<Vec<PathBuf>, String> {
    let dir = root().join("claims");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("reading claims/: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    Ok(files)
}

/// A claim's `name`, which is the key everything else refers to it by.
fn claim_name(text: &str, path: &Path) -> String {
    toml_field(text, "name")
        .unwrap_or_else(|| path.file_stem().unwrap_or_default().to_string_lossy().to_string())
}

/// One value out of a claim file, addressed the way a document refers to it.
///
/// Two shapes, because the registry has two. A bare key is a top-level scalar —
/// `status`, `milestone`. A dotted key is a threshold bound:
/// `threshold.ns_per_op_p99.max` reads `max` out of
/// `ns_per_op_p99 = { max = 50 }` under `[threshold]`.
///
/// Not a TOML parser, and the same caveat as everywhere else in this file: it
/// reads the shape these files are written in. What makes that safe here is
/// that a reference which does not resolve is an error rather than an empty
/// string — a document rendering a blank where a number should be is the one
/// outcome this whole mechanism exists to prevent.
fn claim_value(text: &str, key: &str) -> Option<String> {
    let mut parts = key.split('.');
    let head = parts.next()?;
    let rest: Vec<&str> = parts.collect();

    if rest.is_empty() {
        return toml_field(text, head);
    }
    if rest.len() != 2 {
        return None;
    }

    // Find the `[head]` table, then the `rest[0] = { ... }` line inside it.
    let mut in_table = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_table = trimmed == format!("[{head}]");
            continue;
        }
        if !in_table {
            continue;
        }
        let Some((lhs, rhs)) = trimmed.split_once('=') else { continue };
        if lhs.trim() != rest[0] {
            continue;
        }
        // `{ max = 50 }` — one inline table, one bound per side.
        let inner = rhs.trim().trim_start_matches('{').trim_end_matches('}');
        for entry in inner.split(',') {
            let Some((bound, value)) = entry.split_once('=') else { continue };
            if bound.trim() == rest[1] {
                return Some(value.trim().trim_matches('"').replace('_', ""));
            }
        }
    }
    None
}

/// The registry, as one JSON object per claim.
///
/// Emitted whenever `xtask claims` runs, so the snapshot cannot be older than
/// the last time anybody looked at the registry.
fn write_snapshot() -> Result<PathBuf, String> {
    let path = snapshot_path();
    let out = snapshot_text()?;
    std::fs::write(&path, out).map_err(|e| format!("writing {}: {e}", relative(&path)))?;
    Ok(path)
}

/// The snapshot the registry currently implies, as bytes, without writing it.
///
/// Split out so that the question *is the committed file current* can be asked
/// without answering it by overwriting the evidence.
fn snapshot_text() -> Result<String, String> {
    let mut out = String::from("{\n  \"claims\": [\n");
    let files = claim_files()?;

    for (i, path) in files.iter().enumerate() {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let comma = if i + 1 == files.len() { "" } else { "," };
        let field = |key: &str| claim_value(&text, key).unwrap_or_else(|| "unknown".into());
        out.push_str(&format!(
            "    {{ \"name\": \"{}\", \"status\": \"{}\", \"milestone\": \"{}\", \
             \"file\": \"{}\" }}{comma}\n",
            claim_name(&text, path),
            field("status"),
            field("milestone"),
            relative(path),
        ));
    }
    out.push_str("  ]\n}\n");
    Ok(out)
}

/// Fail if `claims/snapshot.json` is not what the registry currently implies.
///
/// # Why this is a lint and not a line of CI shell
///
/// It was a line of CI shell — `git diff --quiet -- claims/snapshot.json` — and
/// it went red for a reason that had nothing to do with the snapshot. Inside a
/// container git refuses a working tree owned by another uid, and `git diff` is
/// one of the few commands that tolerates running outside a repository at all,
/// so it reports that refusal as *warning: Not a git repository* and exits
/// non-zero. The gate then said the snapshot was stale. It was byte-identical.
/// A check that reports the wrong failure is worse than no check, because the
/// reader spends their time on the file the message named.
///
/// Comparing the bytes needs no repository, no ownership and no second tool. It
/// also runs on a laptop, which the shell conditional never did: it was a rule
/// only CI could apply, and a rule you cannot run before pushing is one you
/// find out about from a red build.
fn lint_snapshot() -> Result<(), String> {
    let path = snapshot_path();
    let expected = snapshot_text()?;
    let actual =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;

    // Compared with CR stripped. `.gitattributes` commits this file `eol=lf`,
    // but a checkout that ignored that should fail on the claim that changed
    // rather than on every line at once.
    if expected.replace('\r', "") == actual.replace('\r', "") {
        println!("lint-snapshot: ok  ({} is current)", relative(&path));
        return Ok(());
    }

    let mut report = format!(
        "{} does not match claims/.\n\nA committed snapshot that disagrees with the \
         registry is a commit publishing numbers the tree does not hold.\n",
        relative(&path)
    );
    for (n, (want, have)) in expected.lines().zip(actual.lines()).enumerate() {
        if want != have {
            let line = n + 1;
            report.push_str(&format!(
                "\n  line {line}\n    registry: {want}\n    file:     {have}\n"
            ));
        }
    }
    let (want, have) = (expected.lines().count(), actual.lines().count());
    if want != have {
        report.push_str(&format!("\n  the registry implies {want} line(s), the file has {have}\n"));
    }
    report.push_str("\nRun `cargo xtask claims` and commit the result.");
    Err(report)
}

/// A reference to a claim value, found in a document.
struct Reference {
    /// Which document, for the message.
    file: String,
    /// `<claim>:<key>`, verbatim, for the message.
    key: String,
    /// What the document currently says.
    rendered: String,
    /// What the registry says.
    actual: String,
}

/// Every `data-claim` reference in `docs/`, resolved against the registry.
///
/// # Why a placeholder and not a build step that generates the whole page
///
/// Because the documents are written by hand and should stay that way. What is
/// wrong with a restated number is not that prose contains numbers, it is that
/// nothing connects the two — so a claim can go red, or move, and the sentence
/// arguing from it stays confidently in place. A marked span is the smallest
/// thing that creates the connection: the prose is still prose, and the number
/// in it has an owner.
fn claim_references() -> Result<Vec<Reference>, String> {
    // name -> file text, read once.
    let mut registry: BTreeMap<String, String> = BTreeMap::new();
    for path in claim_files()? {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        registry.insert(claim_name(&text, &path), text);
    }

    let mut found = Vec::new();
    for path in documents()? {
        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let file = relative(&path);

        for (key, rendered) in spans(&text) {
            let (claim, field) = key
                .split_once(':')
                .ok_or_else(|| format!("{file}: data-claim=\"{key}\" is not <claim>:<key>"))?;
            let source = registry
                .get(claim)
                .ok_or_else(|| format!("{file}: data-claim=\"{key}\" names no claim in claims/"))?;
            let actual = claim_value(source, field).ok_or_else(|| {
                format!("{file}: data-claim=\"{key}\" resolves to nothing in {claim}")
            })?;
            found.push(Reference { file: file.clone(), key: key.clone(), rendered, actual });
        }
    }
    Ok(found)
}

/// Every `<span data-claim="...">text</span>` in a document, as key and text.
fn spans(text: &str) -> Vec<(String, String)> {
    const OPEN: &str = "<span data-claim=\"";
    let mut out = Vec::new();
    for piece in text.split(OPEN).skip(1) {
        let Some((key, rest)) = piece.split_once('"') else { continue };
        let Some(rest) = rest.split_once('>').map(|(_, r)| r) else { continue };
        let Some((rendered, _)) = rest.split_once("</span>") else { continue };
        out.push((key.to_string(), rendered.to_string()));
    }
    out
}

/// The documents that may cite a claim.
fn documents() -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for dir in [root().join("docs"), root().join("docs").join("design")] {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.filter_map(Result::ok) {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "html") {
                out.push(path);
            }
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Rewrite every `data-claim` span to the value the registry holds.
fn render_claims() -> Result<(), String> {
    let stale: Vec<Reference> =
        claim_references()?.into_iter().filter(|r| r.rendered != r.actual).collect();

    if stale.is_empty() {
        println!("claims render: nothing to do — every citation already matches the registry");
        return Ok(());
    }

    let mut by_file: BTreeMap<String, Vec<&Reference>> = BTreeMap::new();
    for reference in &stale {
        by_file.entry(reference.file.clone()).or_default().push(reference);
    }

    for (file, references) in &by_file {
        let path = root().join(file);
        let mut text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        for reference in references {
            let from =
                format!("<span data-claim=\"{}\">{}</span>", reference.key, reference.rendered);
            let to = format!("<span data-claim=\"{}\">{}</span>", reference.key, reference.actual);
            text = text.replace(&from, &to);
            println!("  {file}  {}  {} -> {}", reference.key, reference.rendered, reference.actual);
        }
        std::fs::write(&path, text).map_err(|e| format!("writing {file}: {e}"))?;
    }

    println!("\nrendered {} citation(s) from claims/", stale.len());
    Ok(())
}

/// Fail if a document cites a claim value it no longer has.
///
/// This is the half that makes the mechanism worth having. Rendering on demand
/// would let a document sit stale until somebody remembered to re-render; a
/// check in `lint` means changing a threshold and not re-rendering is a red
/// build, which is the same discipline the determinism and licensing lints
/// apply to their own policies.
fn lint_claims() -> Result<(), String> {
    let references = claim_references()?;
    let stale: Vec<&Reference> = references.iter().filter(|r| r.rendered != r.actual).collect();

    if stale.is_empty() {
        println!("lint-claims: ok  ({} citation(s) match the registry)", references.len());
        return Ok(());
    }

    let mut report = String::new();
    for reference in &stale {
        report.push_str(&format!(
            "  {}  {}\n    document says {}, claims/ says {}\n",
            reference.file, reference.key, reference.rendered, reference.actual
        ));
    }
    Err(format!(
        "{} document citation(s) disagree with the registry:\n{report}\n\
         A number in a design document is not allowed to be a second copy of a\n\
         claim. Run `cargo xtask claims --render` to bring them back into line —\n\
         and if the document was right and the claim is wrong, change the claim,\n\
         because that is the file with the baseline and the reproduction in it.",
        stale.len()
    ))
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

    // Written every time the registry is listed, so the snapshot cannot be
    // older than the last time anybody looked. A snapshot regenerated only on
    // request is a snapshot that is stale exactly when it matters.
    let snapshot = write_snapshot()?;
    let citations = claim_references()?;
    println!("\nsnapshot  {}", relative(&snapshot));
    println!("cited     {} time(s) in docs/, checked by `cargo xtask lint`", citations.len());
    println!("\nEvery number published in docs/design must correspond to an entry here.");
    Ok(())
}

/// Run one claim's workload.
///
/// A `pending` claim runs its workload and reports, but does not gate — the
/// distinction matters, because a number produced before the machinery it
/// describes exists is a sanity check, not evidence.
/// How many boots claim `boot-to-m0` averages over.
///
/// Matches `repeat` in `claims/0003-boot-to-m0.toml`, and is stated in both
/// places because they are different kinds of statement: the claim says what
/// the number means, this says what the command does. A mismatch is a claim
/// describing a run nobody performed, so the command checks.
const BOOT_SAMPLES: u64 = 10;

/// Claim `boot-to-m0`: ten boots, one observation each.
///
/// The measurement is taken *inside* the kernel and printed only when asked
/// for, which is the same shape `timer=` already has and for the same reason.
/// The boot log is a fixture — two runs of a commit produce the same bytes, and
/// that is asserted — so a duration in it would destroy the one contract M0
/// makes. `boottime` is therefore a parameter, and a run carrying it is not a
/// fixture run.
fn claim_boot_to_m0() -> Result<(), String> {
    let mut sample = f_bench::Sample::new("boot-to-m0");

    for i in 0..BOOT_SAMPLES {
        // Not captured-and-printed for every boot: ten full boot logs is two
        // hundred lines of the same thing, and the line being looked for would
        // be lost in it. `machine_with`'s capture is what makes the parse
        // possible; the printing is what this suppresses.
        let (ending, log) = machine_quiet(Some("boottime"))?;
        match ending {
            Ending::Exited(33) => {}
            other => {
                print!("{log}");
                return Err(format!("boot {} of {BOOT_SAMPLES} {other}; expected 33", i + 1));
            }
        }

        let nanos = boot_nanos(&log).ok_or_else(|| {
            format!(
                "boot {} of {BOOT_SAMPLES} reached M0 and reported no boot time\n\n\
                 The kernel prints `boot time  <n> ns to M0` only when the command\n\
                 line carries `boottime`. If the line is missing entirely, the\n\
                 parameter is not reaching the kernel; if it says `unavailable`,\n\
                 the timestamp counter was never calibrated on this boot.",
                i + 1
            )
        })?;
        sample.latency.record(nanos);
        println!("  boot {:>2} of {BOOT_SAMPLES}   {nanos:>12} ns", i + 1);
    }

    println!();
    sample.report();

    match sample.persist(&root().join("claims")) {
        Ok(path) => println!("\nfull distribution written to {}", relative(&path)),
        Err(e) => println!("\nnot recorded: {e}"),
    }
    Ok(())
}

/// The nanosecond count from a boot log that was asked for one.
///
/// Reads the line the kernel writes and nothing else. A log that does not carry
/// it gives `None` rather than a zero, because a zero here would be a boot that
/// took no time — a number, and a wrong one, where the honest answer is that
/// there is no number.
fn boot_nanos(log: &str) -> Option<u64> {
    log.lines()
        .find_map(|line| line.trim().strip_prefix("boot time"))
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|value| value.parse().ok())
}

fn claim_run(name: Option<&str>) -> Result<(), String> {
    let Some(name) = name else {
        return Err("usage: cargo xtask claim <name>   (see `cargo xtask claims`)".into());
    };

    let dir = root().join("claims");
    let file = std::fs::read_dir(&dir)
        .map_err(|e| format!("reading claims/: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|p| p.file_stem().is_some_and(|s| s.to_string_lossy().ends_with(name)))
        .ok_or_else(|| format!("no claim named {name} in claims/"))?;

    let text = std::fs::read_to_string(&file).map_err(|e| e.to_string())?;
    let field = |key: &str| toml_field(&text, key);

    let status = field("status").unwrap_or_else(|| "unknown".into());
    let milestone = field("milestone").unwrap_or_else(|| "?".into());

    println!("claim     {name}");
    println!("status    {status}");
    println!("milestone {milestone}");
    println!("baseline  {}", field("system").unwrap_or_else(|| "unset".into()));
    println!("runner    {}", field("runner").unwrap_or_else(|| "unset".into()));
    println!();

    // Not every claim's workload is a benchmark binary. `boot-to-m0` measures
    // the kernel booting, which is a boot rather than a program — and the
    // measurement is taken inside the kernel because nothing outside it can see
    // where boot begins. Dispatching on the name keeps `cargo xtask claim
    // <name>` as the one reproduction command the registry publishes,
    // regardless of what the workload turns out to be.
    if name == "boot-to-m0" {
        claim_boot_to_m0()?;
    } else {
        // The workload binary is named after the claim, minus the registry prefix.
        let bin = name.replace('-', "_");
        let bin =
            bin.strip_prefix("ring_submit_latency").map_or(bin.clone(), |_| "ring_submit".into());
        sh("cargo", &["run", "--release", "-p", "f-bench", "--bin", &bin])?;
    }

    // The harness itself refuses in a non-measurement environment and says so
    // in its own output — `f_bench::Environment`, E0-P15. Repeating the
    // decision here would be a second copy of a rule that has to hold in one
    // place, so this only names where the decision was taken.
    match status.as_str() {
        "gating" => {
            println!(
                "\nthis claim gates the build; a regression here fails CI — but only\n\
                 where the run was permitted to record. A refusal above is not a pass."
            );
            Ok(())
        }
        "pending" => {
            println!(
                "\nstatus is `pending`: the workload ran, but the machinery this\n\
                 claim describes does not exist yet. Not evidence. Not gating."
            );
            Ok(())
        }
        "tracked" => {
            println!(
                "\nstatus is `tracked`: recorded and watched, and it does not gate.\n\
                 A tracked number exists so that a change nobody intended is visible;\n\
                 promoting it to `gating` is a decision, taken in a reviewable diff."
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
/// The crates whose host tests carry the coverage measurement.
///
/// Not the kernel: it has no host harness at all — `kernel/Cargo.toml` says
/// why — and not `xtask`, which is the tooling rather than the system.
const COVERED: [&str; 4] = ["f-abi", "f-env", "f-ring", "f-bench"];

/// One crate's share of the coverage report.
struct CrateCoverage {
    name: String,
    lines: u64,
    missed: u64,
}

impl CrateCoverage {
    fn percent(&self) -> f64 {
        if self.lines == 0 {
            100.0
        } else {
            (self.lines - self.missed) as f64 * 100.0 / self.lines as f64
        }
    }
}

/// Host tests under coverage instrumentation, reported per crate.
///
/// Every tool this uses comes from the pinned toolchain's own sysroot, the same
/// way [`llvm_tool`] takes the linker and `objcopy`. That is the whole reason
/// `cargo-llvm-cov` is not required here: a coverage number produced by a tool
/// installed separately is a number whose version nobody pinned, and this
/// repository has a container specifically to stop that. It also keeps this
/// command working in the `dev` image rather than only in `full`.
fn coverage() -> Result<(), String> {
    let profdata = llvm_tool("llvm-profdata")?;
    let llvm_cov = llvm_tool("llvm-cov")?;

    // Absolute, because this is read by each *test binary* and resolved against
    // that binary's own working directory rather than by cargo against the
    // workspace. A relative path here scatters profiles into whichever crate
    // directory the harness ran in — see `target_dir`.
    let profiles = target_dir().join("coverage");

    // A stale profile from an earlier build measures code that is no longer
    // there, and `llvm-profdata` merges it in without complaint. The directory
    // is rebuilt rather than added to.
    if profiles.exists() {
        std::fs::remove_dir_all(&profiles)
            .map_err(|e| format!("clearing {}: {e}", relative(&profiles)))?;
    }
    std::fs::create_dir_all(&profiles)
        .map_err(|e| format!("creating {}: {e}", relative(&profiles)))?;

    let mut args: Vec<&str> = vec!["test"];
    for name in COVERED {
        args.push("-p");
        args.push(name);
    }

    // Set on every cargo invocation below, not just the first. A second
    // invocation without it is a *different* build with a different fingerprint,
    // so cargo would rebuild everything uninstrumented and then report those
    // binaries — objects with no counters in them, against a profile that has
    // them, which llvm-cov reports as zero coverage rather than as an error.
    let instrument: [(&str, &str); 1] = [("RUSTFLAGS", "-Cinstrument-coverage")];

    println!("running host tests with coverage instrumentation");
    let status = Command::new("cargo")
        .args(&args)
        .envs(instrument)
        .env("LLVM_PROFILE_FILE", profiles.join("f-%p-%m.profraw"))
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    if !status.success() {
        return Err("instrumented tests failed".into());
    }

    // The same invocation with `--no-run`, which compiles nothing new and
    // reports where the harness put each binary. `llvm-cov` needs the objects
    // as well as the profile: a profile alone says which counters fired and not
    // which source line they belong to.
    let mut probe = args.clone();
    probe.push("--no-run");
    probe.push("--message-format=json");
    let manifest = capture_with("cargo", &probe, &instrument)?;
    let binaries = executables(&manifest);
    if binaries.is_empty() {
        return Err("cargo reported no test executables to measure".into());
    }

    let mut raw: Vec<PathBuf> = std::fs::read_dir(&profiles)
        .map_err(|e| format!("reading {}: {e}", relative(&profiles)))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "profraw"))
        .collect();
    raw.sort();
    if raw.is_empty() {
        return Err(format!(
            "no .profraw files in {}\n\n\
             The tests ran but wrote no profiles, so the instrumented build did\n\
             not take effect. Check that RUSTFLAGS is not already set in the\n\
             environment or in a cargo config, which replaces rather than adds.",
            relative(&profiles)
        ));
    }

    let merged = profiles.join("f.profdata");
    let mut merge = Command::new(&profdata);
    merge.arg("merge").arg("-sparse");
    for path in &raw {
        merge.arg(path);
    }
    merge.arg("-o").arg(&merged);
    let status = merge
        .current_dir(root())
        .status()
        .map_err(|e| format!("could not run llvm-profdata: {e}"))?;
    if !status.success() {
        return Err("llvm-profdata could not merge the raw profiles".into());
    }

    let mut report = Command::new(&llvm_cov);
    report.arg("report").arg(format!("--instr-profile={}", merged.display()));
    for path in &binaries {
        report.arg("-object").arg(path);
    }
    // The standard library and every dependency are not this project's code.
    // `/tests/` is excluded for a sharper reason: an integration test measures
    // its own execution and reports itself as covered, which raises the number
    // without covering anything. The question being asked is how much of the
    // library the tests reach.
    report.arg("--ignore-filename-regex=(/rustc/|/.cargo/registry/|/tests/)");
    let out =
        report.current_dir(root()).output().map_err(|e| format!("could not run llvm-cov: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "llvm-cov could not produce a report: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let text =
        String::from_utf8(out.stdout).map_err(|e| format!("llvm-cov printed non-UTF-8: {e}"))?;

    let crates = summarise(&text)?;
    if crates.is_empty() {
        return Err(format!(
            "llvm-cov reported no files belonging to this workspace\n\n\
             It measured {} object(s) against {} profile(s), so the run itself\n\
             happened. What failed is the mapping from a report row back to a\n\
             crate directory — see `summarise`.",
            binaries.len(),
            raw.len()
        ));
    }

    let lines: u64 = crates.iter().map(|c| c.lines).sum();
    let missed: u64 = crates.iter().map(|c| c.missed).sum();
    let total = CrateCoverage { name: "total".into(), lines, missed };

    println!("\ncoverage — host tests, lines reached\n");
    for c in &crates {
        println!(
            "  {:<10} {:>6.2}%   {:>5} of {:>5}",
            c.name,
            c.percent(),
            c.lines - c.missed,
            c.lines
        );
    }
    println!("  ----------");
    println!(
        "  {:<10} {:>6.2}%   {:>5} of {:>5}",
        total.name,
        total.percent(),
        lines - missed,
        lines
    );

    let summary = profiles.join("summary.json");
    std::fs::write(&summary, coverage_json(&crates, &total))
        .map_err(|e| format!("writing {}: {e}", relative(&summary)))?;

    println!(
        "\nstored in {}, which is what CI keeps with the run.\n\
         No threshold here on purpose: a coverage gate rewards tests written to\n\
         touch lines rather than to catch anything, so this number is reported\n\
         so a fall is visible. What gates is in claims/.",
        relative(&summary)
    );
    Ok(())
}

/// Every `"executable"` in a stream of cargo JSON messages.
///
/// Read the way everything else in this file reads a structured format: for the
/// one field it needs, in the shape the producer actually emits. `--no-run`
/// prints one compact JSON object per line, and a test target's `executable` is
/// a plain string containing no escape a path could need. A full parser here
/// would be a dependency bought to skip a `split`. The same caveat applies as
/// to `toml_field`: this is not a JSON parser, and the day something here needs
/// one it needs a parser rather than a longer version of this.
fn executables(stream: &str) -> Vec<PathBuf> {
    let key = "\"executable\":\"";
    let mut out: Vec<PathBuf> = Vec::new();
    for line in stream.lines() {
        let Some(rest) = line.split(key).nth(1) else { continue };
        let Some(path) = rest.split('"').next() else { continue };
        if path.is_empty() || path == "null" {
            continue;
        }
        let path = PathBuf::from(path);
        if !out.contains(&path) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Fold `llvm-cov report`'s per-file rows into one row per crate.
///
/// The crate is the first path component, which is true of every crate in this
/// workspace — and checked rather than assumed. A row whose first component is
/// not a directory with a `Cargo.toml` in it came from somewhere else and is
/// skipped, so a change to the layout shows up as a crate going missing from
/// the report rather than as a plausible wrong number.
fn summarise(report: &str) -> Result<Vec<CrateCoverage>, String> {
    let mut out: Vec<CrateCoverage> = Vec::new();

    for line in report.lines() {
        let row: Vec<&str> = line.split_whitespace().collect();
        // filename, regions, missed, cover, functions, missed, executed, lines,
        // missed lines, cover, and the branch columns after that. Ten is the
        // shortest row that still carries the two columns this reads.
        if row.len() < 10 {
            continue;
        }
        let Some(first) = row.first() else { continue };
        if *first == "Filename" || *first == "TOTAL" || first.starts_with('-') {
            continue;
        }
        let Some(krate) = first.split('/').next() else { continue };
        if !root().join(krate).join("Cargo.toml").exists() {
            continue;
        }

        let cell = |index: usize| -> Result<u64, String> {
            row.get(index)
                .ok_or_else(|| format!("llvm-cov row too short: {line}"))?
                .parse::<u64>()
                .map_err(|_| format!("llvm-cov row not understood: {line}"))
        };
        let lines = cell(7)?;
        let missed = cell(8)?;

        match out.iter_mut().find(|c| c.name == krate) {
            Some(existing) => {
                existing.lines += lines;
                existing.missed += missed;
            }
            None => out.push(CrateCoverage { name: krate.to_string(), lines, missed }),
        }
    }

    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// The summary, in the one shape a CI step or a later history can read.
///
/// Written by hand for the same reason it is read by hand: a name and three
/// numbers per crate do not justify a serialisation dependency in the tooling
/// crate, and the tooling crate is checked by the same lints it implements.
fn coverage_json(crates: &[CrateCoverage], total: &CrateCoverage) -> String {
    let mut s = String::from("{\n  \"crates\": [\n");
    for (i, c) in crates.iter().enumerate() {
        let comma = if i + 1 == crates.len() { "" } else { "," };
        s.push_str(&format!(
            "    {{ \"name\": \"{}\", \"lines\": {}, \"missed\": {}, \"percent\": {:.2} }}{comma}\n",
            c.name,
            c.lines,
            c.missed,
            c.percent()
        ));
    }
    s.push_str("  ],\n");
    s.push_str(&format!(
        "  \"total\": {{ \"lines\": {}, \"missed\": {}, \"percent\": {:.2} }}\n}}\n",
        total.lines,
        total.missed,
        total.percent()
    ));
    s
}

/// A `"""`-delimited value, which [`toml_scalar`] deliberately does not handle.
///
/// It exists for `evals/tasks/*.toml`, whose prompts are paragraphs and would be
/// illegible on one line. The same caveat applies as to everything else in this
/// file: it reads the shape these files are written in. It is not a TOML parser,
/// and an eval that needs one needs a parser rather than a longer version of
/// this.
fn toml_multiline(text: &str, key: &str) -> Option<String> {
    let mut body: Vec<&str> = Vec::new();
    let mut open = false;

    for line in text.lines() {
        if open {
            if line.trim_end().ends_with("\"\"\"") {
                return Some(body.join("\n").trim().to_string());
            }
            body.push(line.trim_end());
            continue;
        }
        // The `=` test is what stops `prompt` being answered by `prompt_notes`,
        // the same way `toml_field` stops `status` being answered by
        // `statement`.
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix(key)
            && let Some(rest) = rest.trim_start().strip_prefix('=')
            && rest.trim_start().starts_with("\"\"\"")
        {
            open = true;
        }
    }
    None
}

/// The eval suite, in file order, which is the order they were written in and
/// therefore roughly the order the mistakes happened in.
fn eval_files() -> Result<Vec<PathBuf>, String> {
    let dir = root().join("evals").join("tasks");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| format!("reading evals/tasks/: {e}"))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|e| e == "toml"))
        .collect();
    files.sort();
    Ok(files)
}

fn eval_name(path: &Path) -> String {
    path.file_stem().unwrap_or_default().to_string_lossy().to_string()
}

fn evals_list() -> Result<(), String> {
    let files = eval_files()?;
    if files.is_empty() {
        println!("evals: none registered yet. See evals/README.md.");
        return Ok(());
    }

    println!("evals:");
    for path in &files {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        let status = toml_field(&text, "status").unwrap_or_else(|| "unknown".into());
        let defends = toml_field(&text, "defends").unwrap_or_default();
        println!("  {:<30} {:<12} {}", eval_name(path), status, truncate(&defends, 44));
    }
    println!(
        "\n{} tasks, one policy each.\n\
         `cargo xtask eval` runs the suite; `cargo xtask eval <name>` runs one.",
        files.len()
    );
    Ok(())
}

/// What to say when the CLI the suite drives is absent. Worth stating in full,
/// because the alternative reading of a missing binary is that the suite passed.
const NO_CLAUDE_CLI: &str = "\
The eval suite drives the CLI in non-interactive mode, so it needs `claude` on
PATH and a credential in the environment. The CI job skips itself when the
secret is absent, rather than reporting green.";

/// Run the eval suite against the agent configuration in this repository.
///
/// # What this measures
///
/// Not the model. The model is not ours to change. What is ours is `CLAUDE.md`,
/// `.claude/skills/`, `.claude/hooks/` and `REVIEW.md` — and every one of those
/// is a change whose effect is invisible at the moment it is made. The suite is
/// how a change to them is observed at all, which is why the CI job that runs it
/// triggers on a diff to exactly those paths.
///
/// # Grading
///
/// Each task ends by demanding a verdict token, and grading is a substring test
/// for it. That is crude on purpose: a grader that judges free text is another
/// model, with its own failure modes, sitting between a change and the evidence
/// about it. A task that cannot be reduced to a verdict token is a task that has
/// not been made specific enough yet.
fn eval_run(filter: Option<&str>) -> Result<(), String> {
    let suite_path = root().join("evals").join("suite.toml");
    let suite = std::fs::read_to_string(&suite_path)
        .map_err(|e| format!("reading {}: {e}", suite_path.display()))?;
    let floor: f64 =
        toml_field(&suite, "min_pass_rate").and_then(|value| value.parse().ok()).unwrap_or(1.0);

    let mut ran = 0usize;
    let mut passed = 0usize;
    let mut failed: Vec<String> = Vec::new();

    for path in eval_files()? {
        let name = eval_name(&path);
        if filter.is_some_and(|f| !name.contains(f)) {
            continue;
        }

        let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let status = toml_field(&text, "status").unwrap_or_else(|| "active".into());
        if status != "active" {
            println!("  {name:<30} skipped ({status})");
            continue;
        }

        let prompt = toml_multiline(&text, "prompt")
            .ok_or_else(|| format!("{name}: no `prompt = \"\"\"..\"\"\"` block"))?;
        let expect = toml_field(&text, "expect")
            .ok_or_else(|| format!("{name}: no `expect` verdict token"))?;
        let forbid = toml_field(&text, "forbid").filter(|f| !f.is_empty());

        let out = Command::new("claude").args(["-p", &prompt]).current_dir(root()).output();
        let out = out.map_err(|e| format!("could not run `claude`: {e}\n\n{NO_CLAUDE_CLI}"))?;

        let answer = String::from_utf8_lossy(&out.stdout).to_lowercase();
        let found = answer.contains(&expect.to_lowercase());
        let tripped = forbid.as_ref().is_some_and(|f| answer.contains(&f.to_lowercase()));

        ran += 1;
        if found && !tripped {
            passed += 1;
            println!("  {name:<30} pass");
        } else {
            let why = if tripped {
                format!("said `{}`", forbid.unwrap_or_default())
            } else {
                format!("did not say `{expect}`")
            };
            println!("  {name:<30} FAIL   {why}");
            failed.push(format!("{name}: {why}"));
        }
    }

    if ran == 0 {
        println!("\nno active tasks matched. `cargo xtask evals` lists them.");
        return Ok(());
    }

    let rate = passed as f64 / ran as f64;
    println!("\n{passed}/{ran} passed ({:.0}%), floor is {:.0}%", rate * 100.0, floor * 100.0);

    if rate + f64::EPSILON < floor {
        let mut message = String::from(
            "eval pass rate is below the floor.\n\n\
             This gates changes to CLAUDE.md, .claude/ and REVIEW.md, because those\n\
             are changes whose effect is otherwise invisible. Either the change is\n\
             wrong, or the floor in evals/suite.toml moves — and moving it is a\n\
             reviewable diff, which is the point.\n",
        );
        for line in &failed {
            message.push_str("\n  ");
            message.push_str(line);
        }
        return Err(message);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{toml_field, toml_multiline};

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

    /// The shape an eval task is written in, which is the only shape
    /// `toml_multiline` promises to read.
    const TASK: &str = concat!(
        "status = \"active\"\r\n",
        "defends = \"the determinism policy\"\r\n",
        "prompt = \"\"\"\r\n",
        "Line one.\r\n",
        "\r\n",
        "Line two.\r\n",
        "\"\"\"\r\n",
        "expect = \"VERDICT: refuse\"\r\n",
    );

    #[test]
    fn a_multiline_prompt_keeps_its_paragraphs() {
        assert_eq!(toml_multiline(TASK, "prompt").as_deref(), Some("Line one.\n\nLine two."));
    }

    #[test]
    fn a_scalar_after_a_multiline_is_still_readable() {
        // The failure this guards against is a `"""` block swallowing the rest
        // of the file, which makes every later key look absent.
        assert_eq!(toml_field(TASK, "expect").as_deref(), Some("VERDICT: refuse"));
        assert_eq!(toml_multiline(TASK, "statement"), None);
    }
}

/// Each mechanised rule, against something that breaks it.
///
/// # Why the fixtures are strings and not files
///
/// A broken file on disk is checked by every other lint in this file too, so a
/// fixture that violates R03 also has to satisfy the SPDX header rule, the
/// unsafe rule and the determinism rule — or be excluded from all of them, at
/// which point it is excluded from the rule it exists to break as well. That is
/// the trap `lint-mutations` already had to design around.
///
/// A string handed to the same function the lint calls has neither problem, and
/// it makes the fixture and the assertion visible in one place. What it does
/// not cover is the file walk — whether the lint reaches the right files — and
/// that gap is real and stated rather than papered over.
#[cfg(test)]
mod mechanised_rules {
    use super::*;

    #[test]
    fn a_field_with_no_unit_is_caught() {
        let broken = "\
pub struct Sqe {
    /// Absolute deadline.
    pub deadline: u64,
}
";
        let findings = unit_findings("abi/src/lib.rs", broken);
        assert_eq!(findings.len(), 1, "expected one finding, got {findings:?}");
        assert!(findings[0].contains("deadline"), "the finding must name the field: {findings:?}");
    }

    #[test]
    fn a_field_that_states_its_unit_passes() {
        let sound = "\
pub struct Sqe {
    /// Absolute deadline. Unit: nanoseconds, monotonic, in this channel's
    /// epoch. Zero is NO_DEADLINE.
    pub deadline: u64,
    /// Operation selector. Unit: none — an opcode is an identifier.
    pub opcode: u8,
}
";
        assert!(unit_findings("abi/src/lib.rs", sound).is_empty());
    }

    #[test]
    fn the_unit_check_ignores_what_is_not_a_field() {
        // A lint that fired on every `pub fn` would be turned off within a week,
        // and a lint people turn off catches nothing.
        let sound = "\
pub fn submit(entry: Sqe) -> bool { true }
pub const ABI_VERSION: u32 = 1;
pub struct Cursor {
    /// Free-running index. Unit: entries since the channel opened.
    pub value: u32,
}
";
        assert!(
            unit_findings("abi/src/lib.rs", sound).is_empty(),
            "{:?}",
            unit_findings("abi/src/lib.rs", sound)
        );
    }

    #[test]
    fn a_public_callback_is_caught() {
        let broken = "\
pub fn on_completion(f: impl Fn(Cqe)) {}
";
        let findings = callback_findings("ring/src/lib.rs", broken);
        assert!(!findings.is_empty(), "a public closure parameter must be caught");
    }

    #[test]
    fn a_private_closure_is_not_a_callback() {
        // R05 is about what an interface *offers*. A closure inside an
        // implementation is an ordinary closure, and a rule that could not tell
        // the two apart would be a rule nobody could satisfy.
        let sound = "\
fn drain(each: impl Fn(Cqe)) {}
let f = |x: u32| x + 1;
";
        assert!(callback_findings("ring/src/lib.rs", sound).is_empty());
    }

    #[test]
    fn a_claim_with_no_owner_is_caught() {
        let broken = "name = \"ring-submit-latency\"\nstatus = \"pending\"\n";
        let findings = claim_owner_findings("claims/0001-x.toml", broken);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("no [owner]"));
    }

    #[test]
    fn a_claim_citing_a_document_that_does_not_exist_is_caught() {
        // The half that makes the rule worth having. A citation nobody can
        // follow is the failure wearing the fix's clothes, and it is the state
        // a claim drifts into when a document is renamed.
        let broken = "\
[owner]
document = \"docs/design/no-such-document.html\"
section  = \"06\"
";
        let findings = claim_owner_findings("claims/0001-x.toml", broken);
        assert!(
            findings.iter().any(|f| f.contains("does not exist")),
            "expected the missing document to be reported: {findings:?}"
        );
    }

    #[test]
    fn the_registry_as_it_stands_satisfies_all_three() {
        // Not a tautology, and the reason it is here: the three fixtures above
        // prove each lint *can* fail, and this proves the tree is on the other
        // side of that line. A lint that has never passed and a lint that has
        // never failed are equally uninformative.
        assert!(lint_units().is_ok(), "abi/ no longer satisfies R03");
        assert!(lint_callbacks().is_ok(), "an interface has acquired a callback");
        assert!(lint_claim_owners().is_ok(), "a claim has lost its owner");
    }
}
