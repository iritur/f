// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One command: run a scenario at a seed, and print what it produced.
//!
//! # Why this is a binary and not a subcommand of `xtask`
//!
//! Because the claim is about two *processes*. `cargo xtask trace` boots QEMU
//! twice and compares two hashes; the thing that makes that evidence rather than
//! an assertion is that the two runs share nothing but the commit. A simulator
//! called twice inside one process shares an address space, an allocator and
//! whatever a library happened to leave behind — so it can agree with itself for
//! reasons that have nothing to do with the seed.
//!
//! So `xtask` runs this binary twice and compares two lines, which is the same
//! shape as the boot's check and is comparable evidence. The in-process pair is
//! also asserted, in `scenario.rs`, because it is much cheaper and catches the
//! same thing most of the time.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use f_sim::deploy::Deployment;
use f_sim::scenario::{self, SCENARIOS, Scenario};
use f_sim::{DEFAULT_SEED, Outcome, Trouble};

/// Where `cargo xtask component` writes the component files, relative to the
/// workspace root.
///
/// The root is this crate's manifest directory at compile time, so nothing here
/// reads the environment at run time and nothing depends on where the command
/// was invoked from. `--components` overrides it, and `xtask` always passes it
/// explicitly, because `xtask` is what knows about `CARGO_TARGET_DIR`.
const COMPONENTS: &str = "target/component";

/// What the command line asked for.
struct Asked {
    scenario: Option<String>,
    seed: u64,
    components: Option<PathBuf>,
    what: What,
}

/// Which of the three things this command does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum What {
    /// The scenario set, one per line.
    List,
    /// One hash and nothing else, so a comparison does not have to parse a
    /// report. `cargo xtask trace --hash` prints its line the same way.
    Hash,
    /// The whole trace, for a person reading a failure.
    Trace,
    /// The component set, one per line: the name and the content hash a spawn
    /// names it by.
    ///
    /// For the join check. `cargo xtask sim --join` boots the real kernel, reads
    /// the hashes out of its log, and requires this set to hold them — which is
    /// what turns `boot-to-workload` from two commands into one claim about one
    /// component set. RFC 0035.
    Deployment,
    /// A short report: what ran, at what seed, and what it produced.
    Report,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).and_then(run) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("f-sim: {message}");
            ExitCode::FAILURE
        }
    }
}

/// Read the command line, refusing anything it does not understand.
///
/// R04: an unknown flag is refused rather than ignored. A tool that quietly
/// dropped `--hash` would print a report where a comparison expected a hash, and
/// the comparison would fail for a reason nobody could see.
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut asked =
        Asked { scenario: None, seed: DEFAULT_SEED, components: None, what: What::Report };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--list" => asked.what = What::List,
            "--hash" => asked.what = What::Hash,
            "--trace" => asked.what = What::Trace,
            "--deployment" => asked.what = What::Deployment,
            "--components" => {
                let value = rest.next().ok_or("--components needs a directory")?;
                asked.components = Some(PathBuf::from(value));
            }
            "--seed" => {
                let value = rest.next().ok_or("--seed needs a value")?;
                asked.seed = seed(value)?;
            }
            "--help" | "-h" => return Err(usage()),
            other if other.starts_with('-') => {
                return Err(format!("unknown option: {other}\n\n{}", usage()));
            }
            other if asked.scenario.is_some() => {
                let first = asked.scenario.as_deref().unwrap_or("");
                return Err(format!("two scenarios named: {first} and {other}\n\n{}", usage()));
            }
            other => asked.scenario = Some(other.to_string()),
        }
    }
    Ok(asked)
}

/// A seed, in hexadecimal with `0x` or in decimal without.
fn seed(text: &str) -> Result<u64, String> {
    let cleaned: String = text.chars().filter(|c| *c != '_').collect();
    let parsed = match cleaned.strip_prefix("0x").or_else(|| cleaned.strip_prefix("0X")) {
        Some(hex) => u64::from_str_radix(hex, 16),
        None => cleaned.parse::<u64>(),
    };
    parsed.map_err(|_| format!("`{text}` is not a seed: give 0x-prefixed hex or decimal"))
}

fn usage() -> String {
    let mut out = String::from(
        "f-sim [--seed <n>] [--hash | --trace] <scenario>\n\
         f-sim --deployment [--components <dir>]\n\
         f-sim --list\n\n\
         The deterministic simulator. One scenario, one seed, one artefact —\n\
         and the same pair produces the same bytes at this commit.\n\n\
         scenarios:\n",
    );
    for scenario in SCENARIOS {
        out.push_str(&format!("  {:<12} {}\n", scenario.name, scenario.what));
    }
    out.push_str(&format!("\ndefault seed: {DEFAULT_SEED:#018x}\n"));
    out
}

fn run(asked: Asked) -> Result<(), String> {
    if asked.what == What::List {
        for scenario in SCENARIOS {
            println!("{:<12} {}", scenario.name, scenario.what);
        }
        return Ok(());
    }
    if asked.what == What::Deployment {
        // The component set and nothing else, so that the join check compares
        // two lists rather than parsing a report. `cargo xtask sim --join` reads
        // the same hashes out of a real boot log and requires them to be these.
        for component in read_components(asked.components.as_deref())?.components() {
            println!("{:<32} {:#018x}", component.name, component.id);
        }
        return Ok(());
    }

    let name = asked.scenario.ok_or_else(|| format!("no scenario named\n\n{}", usage()))?;
    let scenario =
        scenario::find(&name).ok_or_else(|| format!("no such scenario: {name}\n\n{}", usage()))?;

    let outcome = execute(scenario, asked.seed, asked.components.as_deref())?;

    match asked.what {
        What::Hash => println!("{:#018x}", outcome.digest()),
        What::Trace => println!("{}", outcome.trace.text()),
        What::List | What::Deployment | What::Report => {
            println!("scenario   {}", scenario.name);
            println!("seed       {:#018x}", asked.seed);
            println!("steps      {}", outcome.steps);
            println!("decisions  {}", outcome.decisions);
            println!("records    {}", outcome.trace.len());
            // Faults, always, and zero for the scenarios that arm none. Printed
            // rather than omitted when it is zero, so that *this run injected
            // nothing* is a statement the report makes rather than an absence a
            // reader has to interpret.
            println!("injected   {}", outcome.injected);
            println!("finished   {} ns", outcome.finished_ns);
            println!("digest     {:#018x}", outcome.digest());
        }
    }
    Ok(())
}

/// The component set, read from where the build left it.
///
/// Fail closed, and say what to run: the commonest way to arrive here is a fresh
/// checkout in which `cargo xtask component` has not run, and a tool that
/// answered that with a panic is a tool people work around.
fn read_components(dir: Option<&Path>) -> Result<Deployment, String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let default = root.parent().unwrap_or(&root).join(COMPONENTS);
    Deployment::read(dir.unwrap_or(&default)).map_err(|why| why.message())
}

/// Run one scenario, reading a component set first if it needs one.
///
/// The file reading is here rather than inside `f_sim::scenario`, and the split
/// is deliberate: a scenario is data a compiler checks, and a deployment is an
/// artefact a build produced. Keeping them apart is what lets every other
/// scenario stay a pure function of `(seed, commit)` with no filesystem under
/// it, and it is why this crate's own tests can cover the deployment scenario
/// without needing a build to have happened first.
fn execute(scenario: &Scenario, seed: u64, dir: Option<&Path>) -> Result<Outcome, String> {
    if !scenario.needs_components() {
        return scenario.run(seed).map_err(Trouble::message);
    }
    scenario.run_on(seed, &read_components(dir)?).map_err(Trouble::message)
}
