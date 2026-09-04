// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One command: run a scenario at a seed, and print what it produced — or run
//! thousands of them and print what went wrong.
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
//!
//! # The sweep is the one thing that is deliberately *not* a process per run
//!
//! `--sweep` runs thousands of trials inside this one process, and the argument
//! above is why that needs saying rather than assuming. It does not weaken
//! anything, because a sweep is not a reproduction check: its job is to *find* a
//! failing `(seed, commit)` pair, and what it hands out is a command a stranger
//! runs as its own process. The two-process claim stays where it was — on
//! `cargo xtask sim` — and the sweep is measured against it, because every trial
//! it runs is one this binary would also run alone.
//!
//! A process per trial would cost tens of milliseconds of `cargo` and dynamic
//! linking against a run that takes under a millisecond, which is the difference
//! between a nightly sweep of a million trials and a nightly sweep of ten
//! thousand. `sweep.rs` is where the determinism of the in-process form is
//! argued and tested.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use f_sim::check::{Verdict, examine};
use f_sim::deploy::Deployment;
use f_sim::fault::Injection;
use f_sim::scenario::{self, SCENARIOS, Scenario};
use f_sim::sweep::{DEFAULT_SEEDS, Found, Report, Sweep, Trial};
use f_sim::{DEFAULT_SEED, Outcome, Trouble};

/// Where `cargo xtask component` writes the component files, relative to the
/// workspace root.
///
/// The root is this crate's manifest directory at compile time, so nothing here
/// reads the environment at run time and nothing depends on where the command
/// was invoked from. `--components` overrides it, and `xtask` always passes it
/// explicitly, because `xtask` is what knows about `CARGO_TARGET_DIR`.
const COMPONENTS: &str = "target/component";

/// Where the seed corpus lives, relative to the workspace root.
///
/// In the tree rather than under `target/`, because the corpus is the one
/// artefact of a sweep that outlives the sweep: it is the set of trials that
/// have ever found something, and `docs/design/proving-ground.html` names it as
/// what turns a failing seed into a permanent regression test. `RELEASING.md`
/// ships it.
const CORPUS: &str = "sim/corpus.txt";

/// The deliberate defect this binary was built with, if any.
///
/// Written into a corpus entry so that an entry which is green today says *why*
/// it is green. A corpus of seeds that were all found under a defect and are all
/// clean without it would otherwise read as a corpus of seeds that never found
/// anything. RFC 0040.
/// Both are listed, and both being on at once is named rather than silently
/// reported as one: a corpus entry recorded under two defects was found under
/// two defects.
const BUILT_WITH: Option<&str> =
    match (cfg!(feature = "mutate-crossed-completion"), cfg!(feature = "mutate-silent-reset")) {
        (true, true) => Some("mutate-crossed-completion, mutate-silent-reset"),
        (true, false) => Some("mutate-crossed-completion"),
        (false, true) => Some("mutate-silent-reset"),
        (false, false) => None,
    };

/// What the command line asked for.
struct Asked {
    scenario: Option<String>,
    seed: u64,
    components: Option<PathBuf>,
    corpus: Option<PathBuf>,
    what: What,
    /// The fields a replay may narrow. `None` is *whatever the scenario says*,
    /// which is what every command but a replay wants.
    clients: Option<u32>,
    window: Option<u32>,
    operations: Option<u32>,
    injects: Option<Vec<Injection>>,
    /// How many seeds a sweep runs. Unit: seeds.
    seeds: u32,
    /// Where this sweep's first seed sits in the whole derivation. Unit: seeds,
    /// zero-based.
    ///
    /// Zero unless a caller is sharding. A shard is a range of the *same* seed
    /// derivation, so `--from 11000 --seeds 11000` runs exactly the trials the
    /// unsharded sweep would have run at those indices; `sweep.rs` has the test.
    from: u32,
    /// How many scenarios a sweep runs, from the top of the table. Unit:
    /// scenarios; `None` is all of them.
    scenarios: Option<usize>,
    /// How many threads share a sweep's grid. Unit: threads. A cost knob and
    /// never a verdict — `sweep.rs` has the test that says so.
    jobs: usize,
    /// The other half of `(seed, commit)`.
    commit: Option<String>,
    /// Where `--scan` writes its marks.
    into: Option<PathBuf>,
    /// How far apart `--scan` places its marks. Unit: simulated minutes.
    every: u64,
    /// How many marks `--scan` keeps on disk; zero is all of them. Unit: marks.
    keep: u32,
    /// The first simulated minute `--scan` writes a mark for. Unit: minutes.
    after: u64,
    /// The snapshot `--resume` re-enters from.
    from_snapshot: Option<PathBuf>,
    /// Whether `--scan` writes marks that carry the artefact's running hash
    /// instead of the artefact.
    terse: bool,
}

/// Which of the things this command does.
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
    /// One trial, examined by the oracle. Exit status is the verdict.
    Check,
    /// N seeds across M scenarios, with every failure minimised.
    Sweep,
    /// The largest `--seeds` this process will accept, and nothing else.
    ///
    /// One number on one line, for the same reason `--hash` is one hash: the
    /// caller is a shell script deciding how many shards a night needs, and a
    /// caller that had to parse a report would be a second copy of the
    /// arithmetic. `cargo xtask sweep` is that caller.
    Ceiling,
    /// Replay every corpus entry and require each to be clean.
    Corpus,
    /// Sweep, then merge what it found into the corpus.
    Record,
    /// Every component in the deployment, killed under sustained load and again
    /// with nothing killed. Exit status is the verdict. `E1-P06`, RFC 0041.
    Chaos,
    /// The same, reduced to one digest so that two processes can be compared
    /// without parsing a report.
    ChaosHash,
    /// Run a scenario once, writing a snapshot every `--every` simulated
    /// minutes. `E1-P08`, RFC 0043.
    Scan,
    /// A reservation under adversarial load, and the two arms that make the
    /// result mean anything. Exit status is the verdict. `E1-B07`, RFC 0050.
    Admission,
    /// The same, reduced to one digest so that two processes can be compared
    /// without parsing a report.
    AdmissionHash,
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match parse(&args).and_then(run) {
        Ok(good) => {
            if good {
                ExitCode::SUCCESS
            } else {
                // A finding is not an error: the command did exactly what it was
                // asked to. It is still a non-zero exit, because a nightly job
                // and a shell pipeline read the status and not the prose.
                ExitCode::FAILURE
            }
        }
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
/// the comparison would fail for a reason nobody could see. The same rule covers
/// `--inject`: a misspelled class is refused, because a replay that armed
/// nothing would print a clean result for a trial nobody ran.
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut asked = Asked {
        scenario: None,
        seed: DEFAULT_SEED,
        components: None,
        corpus: None,
        what: What::Report,
        clients: None,
        window: None,
        operations: None,
        injects: None,
        seeds: DEFAULT_SEEDS,
        from: 0,
        scenarios: None,
        jobs: 1,
        commit: None,
        into: None,
        // One minute, which is the unit `E1-P08`'s exit is written in. A finer
        // default would fill a disk with marks nobody asked for; a coarser one
        // would make the phrase *re-entered at minute 39* untrue by a boundary.
        every: 1,
        // Zero: every mark is kept unless a caller says otherwise. `--keep` is
        // for a caller that knows it wants only the recent end of a long run,
        // and a default that silently discarded would be a default that loses a
        // mark somebody wanted.
        keep: 0,
        // Zero: mark from the beginning. `--after` is for a caller who already
        // knows roughly where the failure is and does not want the disk written
        // to for the thirty-eight minutes before it — a *write* avoided rather
        // than a file deleted, and on a whole scan of a long run those are
        // gigabytes apart.
        after: 0,
        from_snapshot: None,
        // Whole by default. A terse mark is much cheaper to re-enter and cannot
        // be judged by the oracle, and a default that quietly gave up the
        // oracle would be a default that answers a smaller question than the one
        // asked. `trace::Carried` is the argument.
        terse: false,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let mut value = |flag: &str| -> Result<String, String> {
            rest.next().cloned().ok_or_else(|| format!("{flag} needs a value"))
        };
        match arg.as_str() {
            "--list" => asked.what = What::List,
            "--hash" => asked.what = What::Hash,
            "--trace" => asked.what = What::Trace,
            "--deployment" => asked.what = What::Deployment,
            "--check" => asked.what = What::Check,
            "--sweep" => asked.what = What::Sweep,
            "--ceiling" => asked.what = What::Ceiling,
            "--corpus" => asked.what = What::Corpus,
            "--record" => asked.what = What::Record,
            "--chaos" => asked.what = What::Chaos,
            // Its own flag rather than `--chaos --hash`, because `what` is one
            // value and a pair of flags whose meaning depended on their order
            // would be a command line with a bug in it. R04, applied to this
            // tool's own argument parser.
            "--chaos-hash" => asked.what = What::ChaosHash,
            "--scan" => asked.what = What::Scan,
            "--admission" => asked.what = What::Admission,
            // Its own flag rather than `--admission --hash`, for the reason
            // `--chaos-hash` gives: `what` is one value, and a pair of flags
            // whose meaning depended on their order would be a command line
            // with a bug in it.
            "--admission-hash" => asked.what = What::AdmissionHash,
            // A *source*, not a `what`. `--resume file --trace` and `--trace
            // --resume file` mean the same thing, which they did not when this
            // set `what`: one order printed a usage banner and the other
            // silently dropped `--trace`, and a re-entry that could not be read
            // was a fast path nobody could bisect with. R04 applied to this
            // tool's own parser, the same sentence `--chaos-hash` above is
            // under: a pair of flags whose meaning depends on their order is a
            // command line with a bug in it.
            "--resume" => asked.from_snapshot = Some(PathBuf::from(value("--resume")?)),
            "--into" => asked.into = Some(PathBuf::from(value("--into")?)),
            "--every" => asked.every = u64::from(count("--every", &value("--every")?)?),
            "--keep" => asked.keep = count("--keep", &value("--keep")?)?,
            "--after" => asked.after = u64::from(offset("--after", &value("--after")?)?),
            "--terse" => asked.terse = true,
            "--components" => asked.components = Some(PathBuf::from(value("--components")?)),
            "--corpus-file" => asked.corpus = Some(PathBuf::from(value("--corpus-file")?)),
            "--commit" => asked.commit = Some(value("--commit")?),
            "--seed" => asked.seed = seed(&value("--seed")?)?,
            "--clients" => asked.clients = Some(count("--clients", &value("--clients")?)?),
            "--window" => asked.window = Some(count("--window", &value("--window")?)?),
            "--ops" => asked.operations = Some(count("--ops", &value("--ops")?)?),
            "--seeds" => asked.seeds = count("--seeds", &value("--seeds")?)?,
            "--from" => asked.from = offset("--from", &value("--from")?)?,
            "--jobs" => asked.jobs = count("--jobs", &value("--jobs")?)? as usize,
            "--scenarios" => {
                asked.scenarios = Some(count("--scenarios", &value("--scenarios")?)? as usize);
            }
            "--no-inject" => asked.injects = Some(Vec::new()),
            "--inject" => {
                let injection = inject(&value("--inject")?)?;
                asked.injects.get_or_insert_with(Vec::new).push(injection);
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

/// A count, refused rather than clamped when it is not one.
///
/// **Zero is refused, and that is R04 rather than fussiness.** Every flag that
/// reaches here names *how much of something a run has* — seeds, scenarios,
/// clients, operations, window, workers — and a zero in any of them produces a
/// run or a grid with nothing in it. `sweep::steps` already states what that
/// costs one level down: *a run with nothing in it produces a short trace and a
/// perfectly stable digest*, which is the one result a check must never report
/// as a pass. Before this refusal existed, `--clients 0` printed `check clean —
/// every property held` and exited zero, and `--seeds 0` printed `sweep: clean —
/// 0 trial(s)`; either would have been a permanently, vacuously green corpus
/// entry, in a file whose whole point is that its entries have been seen to
/// fail.
///
/// The flag is named in the refusal because a corpus line is an argv and the
/// person reading the refusal is looking at one.
fn count(flag: &str, text: &str) -> Result<u32, String> {
    let parsed = text.parse::<u32>().map_err(|_| format!("{flag} takes a number, not `{text}`"))?;
    if parsed == 0 {
        return Err(format!(
            "{flag} 0 asks for a run with nothing in it, which is not a smaller run — it\n\
             is a result that is green because it asserted nothing. Give at least 1."
        ));
    }
    Ok(parsed)
}

/// An index, where zero is a legitimate answer.
///
/// Separate from [`count`] on purpose: `--from 0` is the first shard and means
/// something, where `--seeds 0` means nothing. One function that accepted zero
/// for both would be the fail-open above with a different spelling.
fn offset(flag: &str, text: &str) -> Result<u32, String> {
    text.parse::<u32>().map_err(|_| format!("{flag} takes a number, not `{text}`"))
}

/// One armed class, as `<class>:<after>:<one_in>`.
///
/// The spelling a minimised trial prints, so a report's own output is a valid
/// command line. Three fields and no more, because [`Injection`] has two knobs
/// and a class, and a grammar with room for a fourth would be a grammar somebody
/// has to keep in step with the type.
fn inject(text: &str) -> Result<Injection, String> {
    let bad = || {
        let names: Vec<&str> = f_sim::fault::Class::ALL.iter().map(|class| class.label()).collect();
        format!(
            "`{text}` is not an injection: give <class>:<after>:<one_in>, where class is one of \
             {}",
            names.join(", ")
        )
    };
    let mut parts = text.split(':');
    let (Some(label), Some(after), Some(one_in), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Err(bad());
    };
    let class = f_sim::sweep::class(label).ok_or_else(bad)?;
    let after = after.parse::<u32>().map_err(|_| bad())?;
    let one_in = one_in.parse::<u32>().map_err(|_| bad())?;
    Ok(Injection { class, after, one_in })
}

fn usage() -> String {
    let mut out = String::from(
        "f-sim [--seed <n>] [--hash | --trace | --check] <scenario>\n\
         f-sim --sweep --commit <sha> [--seeds <n>] [--from <k>] [--scenarios <m>]\n\
        \x20        [--jobs <j>] [<scenario>]\n\
         f-sim --ceiling [--scenarios <m>]\n\
         f-sim --record --commit <sha> [--corpus-file <path>] ...\n\
         f-sim --corpus [--corpus-file <path>]\n\
         f-sim --scan --into <dir> --commit <sha> [--every <minutes>] [--keep <n>]\n\
        \x20        [--terse] [--after <minutes>] [--seed <n>] <scenario>\n\
         f-sim --resume <file.snap> [--commit <sha>] [--check|--trace|--hash]\n\
         f-sim --chaos | --chaos-hash [--seed <n>] [--components <dir>]\n\
         f-sim --deployment [--components <dir>]\n\
         f-sim --list\n\n\
         The deterministic simulator. One scenario, one seed, one artefact —\n\
         and the same pair produces the same bytes at this commit.\n\n\
         A replay may narrow the scenario it names, which is what a minimised\n\
         failure prints: --clients <n>, --window <n>, --ops <n>,\n\
         --no-inject, --inject <class>:<after>:<one_in>.\n\n\
         --resume is a source rather than a mode: it takes the same --check,\n\
         --trace and --hash a scenario does, in either order. A run re-entered\n\
         from a *terse* mark carries its prefix as a hash, so --hash answers\n\
         the whole run's digest and --trace prints the tail from the cut\n\
         onward, while --check refuses to judge a partial artefact and says\n\
         so. A *whole* mark answers all three over the whole run.\n\n\
         scenarios:\n",
    );
    for scenario in SCENARIOS {
        out.push_str(&format!("  {:<12} {}\n", scenario.name, scenario.what));
    }
    // The long table, listed here and deliberately **not** in `--list`. `--list`
    // is what `cargo xtask sim` reads to decide which scenarios to reproduce and
    // what `sim/corpus.txt`'s header is regenerated from, and a forty-minute
    // scenario in either of those is a cost nobody asked for. `scenario::LONG`
    // is where the split is argued.
    out.push_str("\nlong scenarios — runnable by name, not swept, not in --list:\n");
    for scenario in f_sim::scenario::LONG {
        out.push_str(&format!("  {:<12} {}\n", scenario.name, scenario.what));
    }
    out.push_str(&format!("\ndefault seed: {DEFAULT_SEED:#018x}\n"));
    out
}

/// Run a scenario once, writing a snapshot every `--every` simulated minutes.
///
/// **`E1-P08`.** The marks are taken during the pass rather than by replaying to
/// each of them, which is the whole of why re-entering is cheaper than replaying
/// — `f_sim::snap::scan` argues it. This is the driver: it makes the directory,
/// names each file after the minute it is the last state of, drops the oldest
/// when `--keep` says to, and prints what it wrote.
///
/// `Ok(false)` is *the run ended badly*, the same convention `--check` uses, so
/// that a scan of a failing scenario is a non-zero exit and a shell can read it.
fn scan(asked: &Asked) -> Result<bool, String> {
    // Fail closed, for `--sweep`'s reason exactly: a snapshot is a *point inside*
    // one (seed, commit) pair, and one that could not name its commit would be a
    // file that restores into a run nobody can place.
    let commit = asked.commit.clone().ok_or(
        "--scan needs --commit <sha>: a snapshot is a point inside one (seed, commit)\n\
         pair, and a file that cannot name its commit restores into a run nobody can\n\
         place. `cargo xtask snapshot` reads it from git and passes it.",
    )?;
    let into =
        asked.into.clone().ok_or("--scan needs --into <dir>: somewhere to write the marks")?;
    let trial = trial(asked)?;
    let deployment = deployment_for(trial.base(), asked.components.as_deref())?;
    std::fs::create_dir_all(&into).map_err(|e| format!("creating {}: {e}", into.display()))?;

    let every = asked.every.saturating_mul(f_sim::snap::MINUTE_NS);
    let mut written: Vec<(u64, PathBuf, usize)> = Vec::new();
    let mut failed: Option<String> = None;
    let outcome =
        f_sim::snap::scan(&trial, &deployment, every, &commit, asked.terse, &mut |mark| {
            if mark.minute() < asked.after {
                return Ok(());
            }
            let path = into.join(format!("minute-{}.snap", mark.minute()));
            if let Err(why) = std::fs::write(&path, mark.bytes) {
                // Carried out rather than panicked on: the run is still going, and
                // an I/O failure half way through a scan is a thing to report with
                // the marks that did land beside it.
                failed = Some(format!("writing {}: {why}", path.display()));
                return Err(f_sim::snap::Broken::Io(format!("writing {}: {why}", path.display())));
            }
            written.push((mark.minute(), path, mark.bytes.len()));
            if asked.keep > 0 && written.len() > asked.keep as usize {
                // The oldest goes. A sliding window is what bounds the disk a long
                // scan costs, and what it gives up is stated rather than implied: a
                // bisect that wants an *early* mark has to scan again from zero.
                // Nothing is lost that a second scan cannot produce, because the run
                // is a function of (seed, commit).
                let (_, old, _) = written.remove(0);
                let _ = std::fs::remove_file(old);
            }
            Ok(())
        });

    let outcome = match outcome {
        Ok(outcome) => outcome,
        Err(why) => return Err(failed.unwrap_or_else(|| why.message())),
    };

    println!("scenario   {}", trial.scenario);
    println!("seed       {:#018x}", trial.seed);
    println!("commit     {commit}");
    if !trial.is_whole() {
        println!("narrowed   {}", trial.argv().join(" "));
    }
    println!("steps      {}", outcome.steps);
    println!(
        "finished   {} ns ({} simulated minute(s))",
        outcome.finished_ns,
        outcome.finished_ns / f_sim::snap::MINUTE_NS
    );
    println!("digest     {:#018x}", outcome.digest());
    println!("marks      {} kept in {}", written.len(), into.display());
    for (minute, path, bytes) in &written {
        println!("  minute {minute:<6} {bytes:>12} bytes  {}", path.display());
    }
    Ok(examine_and_print(&trial, &Ok(outcome)))
}

/// Re-enter a run from a snapshot and finish it.
///
/// The verdict is `--check`'s, over the whole run and not over the tail: a
/// snapshot carries the artefact so far, so the oracle sees the run the file is
/// a point inside rather than the part that happened after it. `snap.rs`'s
/// `Trace::save` is where that choice is argued, and it is the reason this
/// command can be compared against a full replay at all.
fn resume(asked: &Asked) -> Result<bool, String> {
    let path = asked.from_snapshot.clone().ok_or("--resume needs a snapshot file")?;
    if asked.scenario.is_some() {
        // A snapshot names its own scenario and its own seed, so a second name
        // on the command line is either redundant or a contradiction and there
        // is no way to tell which. R04: refused rather than silently preferring
        // one of the two.
        return Err(format!(
            "--resume names the run: a snapshot carries its scenario, its seed and its\n\
             fault plan, so `{}` on the same line is either redundant or a contradiction.",
            asked.scenario.as_deref().unwrap_or("")
        ));
    }
    let bytes = std::fs::read(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    let commit = asked.commit.clone().unwrap_or_default();
    let (sim, head) = f_sim::snap::restore(&bytes, &commit).map_err(|why| why.message())?;
    let result = sim.run();

    // `--hash` and `--trace` answer with the thing and nothing else, exactly as
    // they do for a whole run: a caller comparing two digests should not have to
    // parse a report, and a person reading a failure should not have to.
    if asked.what == What::Hash {
        return Ok(hash_only(&result));
    }
    if asked.what == What::Trace {
        let outcome = result.map_err(Trouble::message)?;
        println!("{}", outcome.trace.text());
        return Ok(true);
    }

    println!("resumed    {}", path.display());
    println!("from       {}", head.line());
    println!("commit     {}", head.commit);
    let good = examine_and_print(&head.trial, &result);
    if let Ok(outcome) = &result {
        println!("steps      {}", outcome.steps);
        println!("finished   {} ns", outcome.finished_ns);
        println!("digest     {:#018x}", outcome.digest());
    }
    Ok(good)
}

/// The digest of a finished run and nothing else.
fn hash_only(result: &Result<Outcome, Trouble>) -> bool {
    match result {
        Ok(outcome) => {
            println!("{:#018x}", outcome.digest());
            true
        }
        Err(trouble) => {
            println!("{}", trouble.message());
            false
        }
    }
}

/// Do what was asked. `Ok(false)` is *the command worked and found something*.
fn run(asked: Asked) -> Result<bool, String> {
    // A snapshot is a *source* and not a verb: what to do with the run it
    // re-enters is `--check`, `--trace`, `--hash` or the default report, exactly
    // as it is for a run that started at step zero. Everything else names a
    // different kind of work — a grid, a scan, a table — and pairing one with a
    // file is refused rather than resolved by argument order (R04).
    if asked.from_snapshot.is_some() {
        return match asked.what {
            What::Report | What::Check | What::Trace | What::Hash => resume(&asked),
            _ => Err("--resume takes --check, --trace, --hash or nothing. The other \
                      modes start a run of their own, and a run re-entered from a file \
                      is not one of them."
                .to_string()),
        };
    }
    match asked.what {
        What::List => {
            for scenario in SCENARIOS {
                println!("{:<12} {}", scenario.name, scenario.what);
            }
            return Ok(true);
        }
        What::Deployment => {
            // The component set and nothing else, so that the join check
            // compares two lists rather than parsing a report. `cargo xtask sim
            // --join` reads the same hashes out of a real boot log and requires
            // them to be these.
            for component in read_components(asked.components.as_deref())?.components() {
                println!("{:<32} {:#018x}", component.name, component.id);
            }
            return Ok(true);
        }
        What::Sweep | What::Record => return sweep(&asked),
        What::Ceiling => {
            println!("{}", f_sim::sweep::max_seeds(asked.scenarios.unwrap_or(SCENARIOS.len())));
            return Ok(true);
        }
        What::Corpus => return corpus(&asked),
        What::Scan => return scan(&asked),
        What::Chaos => return chaos(&asked, false),
        What::ChaosHash => return chaos(&asked, true),
        What::Admission => return admission(&asked, false),
        What::AdmissionHash => return admission(&asked, true),
        What::Hash | What::Trace | What::Check | What::Report => {}
    }

    let trial = trial(&asked)?;
    let deployment = deployment_for(trial.base(), asked.components.as_deref())?;
    let result = trial.run(&deployment);

    if asked.what == What::Check {
        return Ok(examine_and_print(&trial, &result));
    }
    let outcome = result.map_err(Trouble::message)?;
    match asked.what {
        What::Hash => println!("{:#018x}", outcome.digest()),
        What::Trace => println!("{}", outcome.trace.text()),
        _ => report(&trial, &outcome),
    }
    Ok(true)
}

/// A reservation admitted, the same load without one, and one demand too many.
///
/// **`E1-B07`.** Three arms, and two of them are controls: the unreserved arm
/// must *miss*, or the granted arm's zero is a property of the workload rather
/// than of admission control, and the over-subscribed arm must be *refused*
/// rather than admitted and then late. `f_sim::reserve` is where the model, the
/// adversary and the verdict live; this prints what they produced and turns a
/// failure into an exit status.
///
/// `Ok(false)` is *the command worked and found something*, the same convention
/// `--check`, `--sweep` and `--chaos` use.
fn admission(asked: &Asked, hash_only: bool) -> Result<bool, String> {
    use f_sim::reserve;

    let runs = reserve::sweep(asked.seed);
    if hash_only {
        println!("{:#018x}", reserve::digest(&runs));
        return Ok(true);
    }

    println!("seed       {:#018x}", asked.seed);
    println!(
        "machine    {} physical core(s), no sibling, no cache or bandwidth partitioning",
        reserve::machine().physical_cores
    );
    println!(
        "demand     {} core(s), {} ns period, {} ns budget, hard class",
        reserve::demand().cores,
        reserve::demand().period_ns,
        reserve::demand().budget_ns,
    );
    println!("periods    {} per arm, {} ns per slot (virtual)", reserve::PERIODS, reserve::SLOT_NS);
    println!();
    // Every count the claim names is a column, because a claim whose
    // reproduction command does not print its own numbers is a claim nobody can
    // check. `stolen` and `missed` are the two the granted arm is about;
    // `refused` and `stretches` are what say the adversary existed; `idle` is
    // R12's cost written beside the number rather than under it.
    println!(
        "  {:<15} {:>9} {:>7} {:>7} {:>7} {:>8} {:>9} {:>8} {:>7} {:>8} {:>6} {:>7} {:>8}",
        "arm",
        "admitted",
        "periods",
        "met",
        "missed",
        "stolen",
        "refusals",
        "stretch",
        "burst",
        "clamped",
        "excl",
        "slack",
        "idle",
    );
    for run in &runs {
        let slack =
            if run.slack_min == u32::MAX { "-".to_string() } else { run.slack_min.to_string() };
        println!(
            "  {:<15} {:>9} {:>7} {:>7} {:>7} {:>8} {:>9} {:>8} {:>7} {:>8} {:>6} {:>7} {:>8}",
            run.arm.name(),
            run.admitted,
            run.periods,
            run.met,
            run.missed,
            run.stolen,
            run.refused_placements,
            run.stretches,
            run.bursts,
            run.clamped,
            run.excluded,
            slack,
            run.reserved_idle,
        );
    }
    println!();
    // And the same numbers again under the names the registry publishes them
    // under, because the table above is for a reader and this is for a tool:
    // `xtask::admission_reached` checks every one of these against
    // `claims/0010`'s `[threshold]`, and a row the command does not print is a
    // published minimum nothing enforces. `claims/0008` and `claims/0009` each
    // grew a lint of this shape; this is the third and it prints its inputs.
    println!("  claims/0010-admission-refusals, as this run measured them:");
    for (key, value) in reserve::metrics(&runs) {
        println!("    {key:<30} {value}");
    }
    println!();
    // The record RFC 0007 requires to travel with every number collected under
    // a reservation, printed rather than assumed: this part has no sibling, so
    // the mechanism was never exercised, so nothing here is a claim about four
    // delivered components.
    println!(
        "  all four exercised: {} — this part reports no thread-level sibling, so RFC 0005",
        runs[0].exercised
    );
    println!("  rule 2's record is `unexercised` rather than `satisfied`, and RFC 0007 says a");
    println!("  number collected under such a reservation is not a number about this system.");
    println!("  What is a number about it: every count above, taken on a virtual clock.");
    println!();
    println!("  digest   {:#018x}", reserve::digest(&runs));

    match reserve::verdict(&runs) {
        Ok(()) => {
            println!();
            println!(
                "admission  OK  an over-subscribed reservation was refused ADMISSION and ran \
nothing; a granted one met every one of {} deadlines under a load that made the same \
component miss {} of them without it",
                runs[0].periods, runs[1].missed,
            );
            Ok(true)
        }
        Err(why) => {
            println!();
            println!("admission  FAILED\n\n{why}");
            Ok(false)
        }
    }
}

/// Kill every component in the deployment under load, and judge each pair.
///
/// **`E1-P06`.** One pair per component — a run with kills in it and the same
/// run with none — because a survival with no control beside it establishes that
/// nothing went wrong rather than that anything was under test. `f_sim::chaos`
/// is where the mechanism and the verdict live; this prints what they produced
/// and turns a failure into an exit status, which is what a build gate reads.
///
/// `Ok(false)` is *the command worked and found something*, the same convention
/// `--check` and `--sweep` use.
fn chaos(asked: &Asked, hash_only: bool) -> Result<bool, String> {
    use f_sim::chaos;

    let deployment = read_components(asked.components.as_deref())?;
    let pairs = match chaos::sweep(&deployment, asked.seed, chaos::KILLS) {
        Ok(pairs) => pairs,
        Err(why) => {
            println!("chaos      FAILED\n\n{why}");
            return Ok(false);
        }
    };
    if hash_only {
        println!("{:#018x}", chaos::digest(&pairs));
        return Ok(true);
    }

    println!("seed       {:#018x}", asked.seed);
    println!("components {}", pairs.len());
    println!("kills      {} per component, under sustained load", chaos::KILLS);
    println!();
    // Every metric `claims/0005` names is a column here, because a claim whose
    // reproduction command does not print its own numbers is a claim nobody can
    // check. `twice` and `stale` are two columns rather than one — they are two
    // findings with two first debugging steps, and the verdict keeps them apart.
    // `flying` and `retired` are here for the same reason: `flying` is the
    // minimum in flight at any kill, which is what stops the zeros beside it
    // being about a quiescent system, and `retired` is the outage a restart is
    // not. `dropped` is the writes a kill caught between the medium and their
    // answer, which is what makes `wrong`'s zero a statement about durability.
    println!(
        "  {:<20} {:>7} {:>5} {:>6} {:>4} {:>5} {:>5} {:>4} {:>7} {:>6} {:>7} {:>8} {:>6}",
        "component",
        "settled",
        "kills",
        "flying",
        "lost",
        "twice",
        "stale",
        "torn",
        "wrong",
        "refused",
        "retired",
        "pend/res",
        "redone",
    );
    for pair in &pairs {
        println!(
            "  {:<20} {:>7} {:>5} {:>6} {:>4} {:>5} {:>5} {:>4} {:>7} {:>6} {:>7} {:>8} {:>6}",
            pair.chaos.name,
            pair.killed.settled,
            pair.killed.kills,
            pair.killed.flying_min,
            pair.killed.lost,
            pair.killed.twice,
            pair.killed.stale,
            pair.killed.torn,
            pair.killed.wrong,
            pair.killed.failed,
            pair.killed.retired,
            format!("{}/{}", pair.killed.pended, pair.killed.resumed),
            pair.killed.reclaimed,
        );
    }
    println!();
    // The writes a kill caught in the middle of the durability path, and the
    // policy each component declared. A component whose manifest says
    // `restart = never` is judged by a different question — its place is
    // *expected* to stay empty — and printing the policy is what stops a reader
    // wondering why one row's `retired` is one and another's is zero.
    for pair in &pairs {
        println!(
            "  {:<20} {} write(s) interrupted between the medium and their answer; policy {}",
            pair.chaos.name,
            pair.killed.dropped,
            if pair.chaos.refills { "refills the place" } else { "leaves the place empty" },
        );
    }
    println!();
    for pair in &pairs {
        println!(
            "  {:<20} control {} of {} answered, worst {} ns; killing cost {} ns more, against a \
             declared ladder of {} ns",
            pair.chaos.name,
            pair.calm.settled,
            pair.calm.owed,
            pair.calm.worst_ns,
            pair.added_ns(),
            pair.chaos.policy.ladder_ns(pair.chaos.kills),
        );
    }

    // `claims/0006`'s five metrics, produced by the command that claim's
    // `[reproduce]` names. They are printed and **not recorded**: the clock is
    // the model's own, so what a quantile here describes is the parameters of
    // this simulation rather than any machine, and `bench/src/lib.rs` refuses to
    // record a timing in a container for the reason `claims/runner-class-A.md`
    // states at length. VIRTUAL is on the heading rather than in a footnote
    // because a number quoted out of a log carries its heading and not its
    // context.
    println!(
        "\nclient wait, VIRTUAL nanoseconds — the model's own clock and not a machine's, which \
         is\n\x20             why claims/0006 is `pending` on runner-class-A and is reported \
         here rather\n\x20             than recorded\n"
    );
    println!(
        "  {:<20} {:>12} {:>12} {:>12} {:>12} {:>10}",
        "component", "p50 ns", "p99 ns", "p999 ns", "max ns", "restarts"
    );
    for pair in &pairs {
        println!(
            "  {:<20} {:>12} {:>12} {:>12} {:>12} {:>10}",
            pair.chaos.name,
            pair.killed.p50_ns,
            pair.killed.p99_ns,
            pair.killed.p999_ns,
            pair.killed.worst_ns,
            pair.killed.spawns.saturating_sub(1),
        );
    }

    // The number the claim records, and it is a count rather than a time — which
    // is the whole reason this claim may gate in a container while three others
    // wait for a machine. `bench/src/lib.rs` is where that rule is decided.
    let blast: u32 = pairs.iter().map(|pair| pair.killed.clients_failed).sum();
    println!("\nblast radius  {blast} client(s) observed anything except added latency");
    println!("digest        {:#018x}", chaos::digest(&pairs));
    Ok(true)
}

/// The trial the command line names.
fn trial(asked: &Asked) -> Result<Trial, String> {
    let name = asked.scenario.clone().ok_or_else(|| format!("no scenario named\n\n{}", usage()))?;
    let scenario =
        scenario::find(&name).ok_or_else(|| format!("no such scenario: {name}\n\n{}", usage()))?;
    let mut trial = Trial::of(scenario, asked.seed);
    if let Some(clients) = asked.clients {
        trial.clients = clients;
    }
    if let Some(window) = asked.window {
        trial.window = window;
    }
    if let Some(operations) = asked.operations {
        trial.operations = operations;
    }
    if let Some(injects) = &asked.injects {
        // Leaked, for `sweep::plan`'s reason: a world is armed with a `'static`
        // plan because a scenario is data in the binary, and a plan read off a
        // command line is granted for the life of the process that read it.
        trial.injects = Box::leak(injects.clone().into_boxed_slice());
    }
    Ok(trial)
}

/// The short report a person reads.
fn report(trial: &Trial, outcome: &Outcome) {
    println!("scenario   {}", trial.scenario);
    println!("seed       {:#018x}", trial.seed);
    if !trial.is_whole() {
        // A narrowed trial says so, because a report that looked like the
        // shipped scenario's while describing a smaller run is the one way this
        // output could mislead.
        println!("narrowed   {}", trial.argv().join(" "));
    }
    println!("steps      {}", outcome.steps);
    println!("decisions  {}", outcome.decisions);
    println!("records    {}", outcome.trace.len());
    // Faults, always, and zero for the scenarios that arm none. Printed rather
    // than omitted when it is zero, so that *this run injected nothing* is a
    // statement the report makes rather than an absence a reader has to
    // interpret.
    println!("injected   {}", outcome.injected);
    println!("finished   {} ns", outcome.finished_ns);
    println!("digest     {:#018x}", outcome.digest());
}

/// One trial, examined. Answers whether it was clean.
fn examine_and_print(trial: &Trial, result: &Result<Outcome, Trouble>) -> bool {
    println!("trial      {}", trial.argv().join(" "));
    match examine(result) {
        Verdict::Clean => {
            println!("check      clean — every property held");
            true
        }
        Verdict::Failed(finding) => {
            println!("check      {} — {}", finding.check, finding.what);
            println!("evidence   {}", finding.evidence);
            false
        }
    }
}

/// N seeds across M scenarios, and every failure minimised.
fn sweep(asked: &Asked) -> Result<bool, String> {
    // Fail closed, and the refusal is the point: a report that cannot name its
    // commit is an incomplete bug report, and a seed without a commit reproduces
    // nothing. `cargo xtask sweep` reads it from git and passes it, which is why
    // this is a refusal rather than a default.
    let commit = asked.commit.clone().ok_or(
        "--sweep needs --commit <sha>: a seed without a commit reproduces nothing.\n\
         `cargo xtask sweep` reads it from git and passes it; standing in a checkout,\n\
         `--commit $(git rev-parse HEAD)` is the same thing by hand.",
    )?;

    let sweep = match &asked.scenario {
        Some(name) => {
            let scenario = scenario::find(name)
                .ok_or_else(|| format!("no such scenario: {name}\n\n{}", usage()))?;
            Sweep::just(scenario, asked.seed, asked.seeds)
        }
        None => Sweep::span(
            asked.seed,
            asked.from,
            asked.seeds,
            asked.scenarios.unwrap_or(SCENARIOS.len()),
        ),
    };

    // Refused before a trial runs, on the same rule as everything else here. A
    // grid this process cannot hold does not fail by finding nothing: it is
    // killed for memory part way through, which produces a red status and a
    // truncated report — a nightly saying *nothing* in the shape of a nightly
    // saying *a bug*. `sweep.rs` is where the bound is computed and argued, and
    // `--ceiling` is how a caller shards on it without keeping a second copy of
    // the arithmetic.
    if sweep.over_budget() {
        let ceiling = f_sim::sweep::max_seeds(asked.scenarios.unwrap_or(SCENARIOS.len()));
        return Err(format!(
            "this grid would leave {} MiB at `'static` and one process holds {} MiB.\n\n\
             Every trial leaks its clients' buffer regions — sim/src/client.rs grants a\n\
             component's region for the life of the component, and a simulated component's\n\
             life is the run — so a million runs in one process is a million regions.\n\
             Refusing is R04: the alternative is being killed for memory half way through\n\
             a night and reporting a file that was truncated by the kill.\n\n\
             At most --seeds {ceiling} here. Sweep the rest as further shards, starting\n\
             `--from {} --seeds {ceiling}`. A shard is a range of the same seed derivation,\n\
             so the shards together try exactly what one process would have.",
            sweep.leak_bytes() / (1 << 20),
            f_sim::sweep::LEAK_BUDGET / (1 << 20),
            asked.from.saturating_add(ceiling),
        ));
    }

    // Read once, before a single trial: the deployment scenario's component set
    // is an artefact the build produced, and a sweep that read it per trial
    // would be a sweep whose result depended on what the filesystem did between
    // two of them.
    //
    // A missing component set is not fatal here, which is the one place in this
    // binary that a refusal is not: a sweep of the self-contained scenarios is a
    // legitimate thing to ask for with no build behind it. The empty deployment
    // is not swallowed either — the deployment scenario refuses on it,
    // `check::examine` gives that refusal a signature of its own, and the report
    // says so in the same shape it says everything else, which is R04 in the
    // direction that keeps the sweep honest about what it covered.
    let deployment =
        Deployment::read(&components_dir(asked.components.as_deref())).unwrap_or_default();

    let leak = sweep.leak_bytes();
    let report = sweep.run(asked.jobs, &deployment);
    // A grid that collapsed to nothing, caught after the fact as well as at the
    // command line: `--seeds 0` and `--scenarios 0` are refused by the parser,
    // and this is what catches a grid that emptied for a reason nobody has
    // thought of yet. A sweep of no trials is not a clean sweep.
    if report.vacuous() {
        return Err("the grid held no trials, so there is nothing this run could have found.\n\
                    A sweep that asserted nothing is not a pass — R04."
            .to_string());
    }
    print_sweep(&report, asked, &commit, leak);

    if asked.what == What::Record {
        let path = corpus_path(asked.corpus.as_deref());
        let added = record(&path, &report, &commit)?;
        println!("\ncorpus     {added} entr(y/ies) added to {}", path.display());
    }
    Ok(report.clean())
}

/// The sweep's report, as the bytes a nightly job keeps.
fn print_sweep(report: &Report, asked: &Asked, commit: &str, leak: u64) {
    println!("sweep — {} trial(s)", report.trials);
    println!("commit  {commit}");
    println!("base    {:#018x}", asked.seed);
    // What this grid left at `'static`, beside the bound it was checked against.
    // The refusal that bound produces is only as good as the model behind it, so
    // the model states itself in every artefact a nightly keeps: a sweep killed
    // for memory *inside* its budget leaves behind the number it thought it
    // needed, which is the one measurement that would say the arithmetic is
    // wrong. RFC 0042.
    println!(
        "memory  {} MiB left at `'static`, of a budget of {} MiB",
        leak / (1 << 20),
        f_sim::sweep::LEAK_BUDGET / (1 << 20)
    );
    // Printed only when it is not zero, because a line saying `from 0` on every
    // report would be a line readers learn to skip — and this one says the
    // report covers part of a grid, which is the one thing a reader must not
    // skip. A shard is a range of the same derivation, so the seed indices
    // below are indices in the whole sweep and not in this process.
    if asked.from > 0 {
        println!(
            "shard   seeds {} to {} of the derivation from that base",
            asked.from,
            u64::from(asked.from) + u64::from(asked.seeds) - 1
        );
    }
    if let Some(defect) = BUILT_WITH {
        println!("built   with the deliberate defect `{defect}` armed");
    }
    println!("\n  {:<12} {:>7} {:>7}", "scenario", "trials", "failed");
    for (name, trials, failed) in &report.tally {
        println!("  {name:<12} {trials:>7} {failed:>7}");
    }

    if report.clean() {
        println!(
            "\nsweep: clean — {} trial(s), nothing found.\n\
             \x20 A clean sweep is worth exactly what the oracle is worth, so the other half\n\
             \x20 of this command is `cargo xtask sweep --mutate`: it arms a deliberate defect\n\
             \x20 and requires this to go red. RFC 0040.",
            report.trials
        );
        return;
    }

    println!(
        "\n{} finding(s), {} distinct check(s), smallest reproduction first.\n\
         \x20 A finding is kept per (scenario, check) because each one reproduces on its\n\
         \x20 own; several scenarios tripping one check is usually one bug seen from\n\
         \x20 several angles, which is what the second number is for. Act on finding 1:\n\
         \x20 it is the tightest reproduction this sweep has.",
        report.found.len(),
        report.signatures()
    );
    for (nth, found) in report.found.iter().enumerate() {
        print_found(nth + 1, found, commit);
    }
    println!("\nsweep: {} finding(s) — each line above reproduces on its own.", report.found.len());
}

/// One finding, with the two lines that are the deliverable.
fn print_found(nth: usize, found: &Found, commit: &str) {
    let minimal = &found.minimal;
    println!("\nfinding {nth}  {} / {}", found.scenario, found.signature);
    println!("  property   {}", found.what);
    println!("  evidence   {}", found.evidence);
    println!("  seen       {} trial(s) of this scenario", found.occurrences);
    println!("  seed       {:#018x}  (seed {} of the sweep)", found.seed, found.at);
    println!("  repro      {}", line(&Trial::of(found_base(found), found.seed), commit));
    println!(
        "  minimised  {}{}",
        minimal.size.line(),
        if minimal.exhausted { ", and the shrink budget ran out first" } else { "" }
    );
    println!(
        "             {} candidate run(s); {}; {}",
        minimal.spent,
        if minimal.exhausted { "not a minimum" } else { "1-minimal against the move table" },
        if minimal.stable { "reproduced twice" } else { "DID NOT REPRODUCE TWICE" }
    );
    if !minimal.trial.is_whole() {
        println!("  smallest   {}", line(&minimal.trial, commit));
    }
    // The artefact, named rather than printed as a second command line. Both
    // lines above run `--check`, which exits non-zero and names the property, so
    // pasting one is a verdict; `--trace` is for the reader who then wants the
    // seventy lines behind it. Saying so once per finding costs a line and
    // removes the only question the two commands above leave open.
    println!("  artefact   the same line with `--check` replaced by `--trace`");
    if !minimal.stable {
        println!(
            "  warning    this failure did not reproduce, so the lines above are not a bug\n\
             \x20            report. Something in the model is reading what the seed does not\n\
             \x20            own — RFC 0004, and `cargo xtask sim` is the check that isolates it."
        );
    }
}

/// The shipped scenario a finding was found in.
fn found_base(found: &Found) -> &'static Scenario {
    scenario::find(found.scenario).expect("a finding names a shipped scenario")
}

/// The one line a stranger pastes.
///
/// It names the commit, because a seed without one reproduces nothing — and it
/// names it the way this tree already binds a run to a commit, by standing in
/// the checkout. The `git` half is a no-op for a reader who is already there,
/// which is deliberate: a reproduction that only works after a checkout is one
/// nobody runs while looking at the failure.
fn line(trial: &Trial, commit: &str) -> String {
    format!("git switch --detach {commit} && {}", trial.command())
}

/// Replay every corpus entry and require each to be clean.
fn corpus(asked: &Asked) -> Result<bool, String> {
    let path = corpus_path(asked.corpus.as_deref());
    let entries = read_corpus(&path)?;
    if entries.is_empty() {
        return Err(format!(
            "{} holds no entries.\n\n\
             An empty corpus is not a corpus: it is a file that passes because it asks\n\
             nothing. `cargo xtask sweep --mutate` is what puts entries in it.",
            path.display()
        ));
    }
    println!("corpus — {} entr(y/ies) from {}", entries.len(), path.display());
    if let Some(defect) = BUILT_WITH {
        println!("built   with the deliberate defect `{defect}` armed");
    }
    println!();

    let mut failed = 0usize;
    for argv in &entries {
        let asked = parse(argv)?;
        let trial = trial(&asked)?;
        let deployment = deployment_for(trial.base(), asked.components.as_deref())?;
        let verdict = examine(&trial.run(&deployment));
        match verdict.signature() {
            None => println!("  [ok]  {}", argv.join(" ")),
            Some(signature) => {
                failed += 1;
                println!("  [--]  {}  — {signature}", argv.join(" "));
            }
        }
    }

    if failed == 0 {
        println!(
            "\ncorpus: {} entr(y/ies), all clean.\n\
             \x20 Every one of these is a trial that found something once. They are green\n\
             \x20 now, and a change that made any of them red would be a regression with a\n\
             \x20 command already written for it.",
            entries.len()
        );
        return Ok(true);
    }
    println!("\ncorpus: {failed} of {} entr(y/ies) failed.", entries.len());
    Ok(false)
}

/// Every corpus entry, as the argument list it is.
///
/// A corpus line **is** an argv, which is the whole of the format: no schema, no
/// parser, no lint to keep the schema in step with the type — the same `parse`
/// above reads a corpus entry and a command line, so an entry that this binary
/// cannot run is an entry that fails to load. `#` opens a comment and a blank
/// line is nothing.
fn read_corpus(path: &Path) -> Result<Vec<Vec<String>>, String> {
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_whitespace().map(str::to_string).collect())
        .collect())
}

/// Merge a sweep's findings into the corpus, and answer how many were new.
///
/// Existing entries are kept exactly as they are — the corpus is append-only for
/// the same reason `docs/rfc/` is: an entry that was removed because somebody
/// believed the bug was gone is the entry that would have caught it coming back.
fn record(path: &Path, report: &Report, commit: &str) -> Result<usize, String> {
    let existing = read_corpus(path).unwrap_or_default();
    let held: Vec<String> = existing.iter().map(|argv| argv.join(" ")).collect();

    let mut text = corpus_header();
    if let Ok(previous) = std::fs::read_to_string(path) {
        // Everything after the generated header, verbatim. The header is
        // regenerated on every write so that the scenario set it states cannot
        // drift from the table; the entries below it are never rewritten.
        if let Some(body) = previous.split_once(HEADER_END) {
            text.push_str(body.1);
        } else {
            text.push_str(&previous);
        }
    }

    let mut added = 0usize;
    for found in &report.found {
        let argv = found.minimal.trial.argv().join(" ");
        if held.contains(&argv) {
            continue;
        }
        added += 1;
        text.push_str(&format!(
            "\n# {}\n# found     {} — {}\n# commit    {commit}\n# under     {}\n# evidence  {}\n{argv}\n",
            "-".repeat(70),
            found.signature,
            found.what,
            BUILT_WITH.unwrap_or("no deliberate defect; this tree, as it shipped"),
            found.evidence,
        ));
    }
    if added > 0 || !path.exists() {
        std::fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    }
    Ok(added)
}

/// Where the generated header stops and the entries begin.
const HEADER_END: &str = "# ---- entries ----\n";

/// The header, regenerated on every write.
///
/// It carries the **scenario set**, which is the other half of what
/// `RELEASING.md` calls *the seed corpus and scenario set*: one file holds both,
/// so a release row cannot be half true. Regenerated rather than maintained,
/// because a list of scenarios in a comment is a list that stops matching the
/// table.
fn corpus_header() -> String {
    let mut out = String::from(
        "# The seed corpus and the scenario set.\n\
         #\n\
         # Every line below that is not a comment is an argument list for `f-sim`, and\n\
         # every one of them is a trial that found something once. `cargo xtask sweep\n\
         # --corpus` replays all of them and requires each to be clean now, which is what\n\
         # makes this a regression suite rather than a list of numbers.\n\
         #\n\
         # There is no format here beyond *a line is an argv*: `f-sim`'s own command-line\n\
         # parser reads an entry, so an entry this binary cannot run is an entry that\n\
         # fails to load, and a stranger reproduces one by pasting it after\n\
         # `cargo run -q -p f-sim -- --trace`.\n\
         #\n\
         # Append-only. An entry removed because somebody believed the bug was gone is\n\
         # the entry that would have caught it coming back.\n\
         #\n\
         # The scenario set, as the table ships it:\n",
    );
    for scenario in SCENARIOS {
        out.push_str(&format!("#   {:<12} {}\n", scenario.name, scenario.what));
    }
    out.push_str("#\n");
    out.push_str(HEADER_END);
    out
}

/// Where the corpus lives.
fn corpus_path(given: Option<&Path>) -> PathBuf {
    given.map_or_else(|| workspace().join(CORPUS), Path::to_path_buf)
}

/// Where the component files live.
fn components_dir(given: Option<&Path>) -> PathBuf {
    given.map_or_else(|| workspace().join(COMPONENTS), Path::to_path_buf)
}

/// The workspace root, from this crate's manifest directory at compile time.
fn workspace() -> PathBuf {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.parent().map_or(root.clone(), Path::to_path_buf)
}

/// The component set, read from where the build left it.
///
/// Fail closed, and say what to run: the commonest way to arrive here is a fresh
/// checkout in which `cargo xtask component` has not run, and a tool that
/// answered that with a panic is a tool people work around.
fn read_components(dir: Option<&Path>) -> Result<Deployment, String> {
    Deployment::read(&components_dir(dir)).map_err(|why| why.message())
}

/// The component set a scenario needs, and nothing when it needs none.
///
/// The file reading is here rather than inside `f_sim::scenario`, and the split
/// is deliberate: a scenario is data a compiler checks, and a deployment is an
/// artefact a build produced. Keeping them apart is what lets every other
/// scenario stay a pure function of `(seed, commit)` with no filesystem under
/// it, and it is why this crate's own tests can cover the deployment scenario
/// without needing a build to have happened first.
fn deployment_for(scenario: &Scenario, dir: Option<&Path>) -> Result<Deployment, String> {
    if scenario.needs_components() { read_components(dir) } else { Ok(Deployment::default()) }
}
