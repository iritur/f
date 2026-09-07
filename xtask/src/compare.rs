// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `cargo xtask compare`: two whole-system states, two hashes, and the subtree
//! they differ in.
//!
//! `E2-P05`. The exit is *an injected divergence is localised to a named
//! subtree automatically, with no human reading a log*, and this file is the
//! `automatically`: one command, an exit status, and a name printed by a
//! machine that computed it.
//!
//! # What is here and what is in `f-sim`
//!
//! The fold and the descent are `sim/src/whole.rs` — beside the states they
//! run over, because that is where a whole-system state exists. What is here is
//! the same thing `trace_check` is to a boot: the pair of runs, the negative
//! control, and the refusal to believe a green comparison that could not have
//! gone red.
//!
//! # Why the first phase is two processes and not two calls
//!
//! `sim/src/main.rs` states the argument and it is the reason this file exists
//! at all: two runs inside one process share an address space, an allocator and
//! whatever a library left behind, so they can agree for reasons that have
//! nothing to do with the seed. `--compare`'s own first phase makes the
//! in-process claim, which is cheaper and weaker; this one runs `--compare-hash`
//! as two subprocesses and compares two lines, which is the shape
//! `cargo xtask trace` uses for a boot and is comparable evidence.
//!
//! # What this command does *not* cover, and where that is written down
//!
//! The frame's own tree. `kernel/src/state.rs` publishes one and mounts a
//! region per component under it, but a spawned component's words are written by
//! nobody — the frame lays out the header and the schema and writes no word
//! after that (RFC 0065, `E1-B15`). So a boot's component subtrees are constants,
//! and a comparison over constants is a comparison that cannot diverge. The
//! simulator's components are where the counters actually live —
//! `sim/src/state.rs` argues why they live *in* the published region — which is
//! why the two-hash comparison is asked of a run and not yet of a boot. That is
//! a gap in the system and not in this command, and [`WHOLE_SYSTEM_GAP`] is
//! where a reader meets it rather than having to infer it.

use crate::{capture, capture_echoing, component_dir};

/// The scenario `compare` runs.
///
/// `alloc`, and the choice is the whole of what makes phase 3 mean anything: it
/// is the one shipped scenario that installs **more than one client** *and* arms
/// a fault class. A single-client scenario would make *localised to a named
/// subtree* a question with one possible answer, and a multi-client scenario
/// that armed nothing would have no divergence to localise.
///
/// That is not a property this file went looking for; the scenario's own entry
/// in `sim/src/scenario.rs` states it about its `clients: 2` — *an allocation
/// failure is one component's, so the run has to contain a component it did not
/// happen to; one client would make "contained" unfalsifiable*. The same
/// sentence is why it is the right scenario here: *contained* and *localised*
/// are the same claim asked of a trace and of a tree.
///
/// It reads no component file — `Peer::Blk`, so `deployment_for` gives it an
/// empty deployment — which is why this command has no build ahead of it and
/// can sit early in the loop.
const SCENARIO: &str = "alloc";

/// The seed both halves are driven by.
///
/// The same constant `trace` and `sim` use, and stated here rather than
/// defaulted inside the simulator for the reason that one gives: the contract is
/// about a *pair*, `(seed, commit)`, and a pair with an implicit half is a pair
/// nobody can quote. Unit: none — half of that pair.
const SEED: &str = "0xf00dbeefcafe1234";

/// The negative control.
///
/// A second seed rather than a second build, for `SIM_OTHER_SEED`'s reason: two
/// runs at one seed agreeing proves nothing on its own, because a root over
/// something that does not vary agrees with itself forever. A boot needs a
/// broken kernel to demonstrate that and a simulated run does not.
/// Unit: none — a seed.
const OTHER_SEED: &str = "0x5eed0000000005ee";

/// What a whole-system comparison does not yet reach, printed by the command
/// rather than left in a document.
///
/// The frame publishes a tree and mounts one region per component under it, and
/// **the component regions are all zeros**: `E1-B15` has the frame write the
/// header and the schema out of the manifest and write no word after that. A
/// component that has run has published nothing into its own mapping, so a
/// boot's whole-system state is the frame's own nodes beside four constant
/// subtrees — and two of those compared is a comparison that cannot diverge.
///
/// It is printed on every green run for `CHAOS_GAP`'s reason: a limitation
/// stated in a commit message is one nobody re-reads, and the failure that
/// matters is not that it is never closed but that it is closed and the
/// command goes on describing it. Whoever teaches a component to write into its
/// own region deletes this constant in the same diff.
pub const WHOLE_SYSTEM_GAP: &str = "\
The frame's half is not in this. `kernel/src/state.rs` mounts one region per
component under the frame's root, and nothing writes a word into those regions:
the frame lays out the header and the schema from the manifest and stops there
(RFC 0065). So a boot's component subtrees are constants, and a comparison over
constants cannot diverge. What is compared above is the simulator's components,
whose counters live *in* the published region — `sim/src/state.rs` argues why —
which is the only place in this tree where a whole-system state has numbers in
it that two runs can disagree about.";

/// One whole-system root, as a subprocess.
fn root(scenario: &str, seed: &str) -> Result<String, String> {
    let dir = component_dir()?;
    let out = capture(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "f-sim",
            "--",
            "--compare-hash",
            "--seed",
            seed,
            "--components",
            &dir,
            scenario,
        ],
    )?;
    let line = out.trim().to_string();
    // A root is 64 lower-case hexadecimal digits and nothing else. Checked
    // rather than assumed, because the failure this would otherwise hide is the
    // worst one available: two runs that both printed a usage banner would
    // compare equal, and the comparison would report agreement about a pair of
    // runs that never happened. R04.
    if line.len() != 64 || !line.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(format!("f-sim printed `{line}`, which is not a whole-system root"));
    }
    Ok(line)
}

/// The whole check.
///
/// # Errors
///
/// A sentence naming which phase failed and what it means. A comparison that
/// went red is not an error — the command did what it was asked — but it is a
/// non-zero exit, because a job and a shell pipeline read the status and not the
/// prose.
pub fn compare() -> Result<(), String> {
    println!("whole-system state comparison — scenario {SCENARIO}, seed {SEED}\n");

    println!("[1/3] two processes, one seed — the roots must agree");
    let first = root(SCENARIO, SEED)?;
    let second = root(SCENARIO, SEED)?;
    println!("  run 1  {first}");
    println!("  run 2  {second}");
    if first != second {
        return Err("two processes at one (seed, commit) produced two whole-system states.\n\n\
             That is RFC 0004's contract failing where every other layer of this\n\
             apparatus rests on it, and it makes the comparison below meaningless:\n\
             a descent that names a subtree is naming one that moved for a reason\n\
             nobody asked for."
            .to_string());
    }
    println!("  agreed: {first}\n");

    println!("[2/3] a second seed — the roots must differ");
    let other = root(SCENARIO, OTHER_SEED)?;
    println!("  seed {OTHER_SEED}  {other}");
    if other == first {
        return Err("two different seeds produced one whole-system root.\n\n\
             That makes the agreement above worth nothing: a root folded over\n\
             something that does not vary agrees with itself forever, which is\n\
             indistinguishable from a root that works. Either the components have\n\
             stopped publishing what they do, or `whole::Whole::root` is hashing\n\
             something the run cannot move — `sim/src/whole.rs`."
            .to_string());
    }
    println!("  differed, as required — the root is a reading and not a constant\n");

    println!("[3/3] an injected divergence — the descent must name the subtree");
    let dir = component_dir()?;
    let (report, ok) = capture_echoing(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "f-sim",
            "--",
            "--compare",
            "--seed",
            SEED,
            "--components",
            &dir,
            SCENARIO,
        ],
    )?;
    if !ok {
        return Err("the injected divergence was not localised. The report above says which\n\
             phase went red: a pair of runs that would not agree, an injection that\n\
             changed no published state, a descent that named a component and no\n\
             subtree inside it, or a seeded injection the descent could not name by\n\
             its exact node."
            .to_string());
    }
    // The status alone is not the exit. *Localised to a named subtree* is a
    // claim about the output, and a `--compare` that exited zero having printed
    // nothing would satisfy an exit code while naming nothing at all — which is
    // the argument `MUTATIONS` makes for every boot, applied to a report.
    let named = report
        .lines()
        .find_map(|line| line.trim().strip_prefix("subtree").map(str::trim))
        .filter(|name| !name.is_empty())
        .ok_or(
            "`f-sim --compare` exited clean and printed no `subtree` line, so nothing\n\
             was named. The exit E2-P05 is written against is *localised to a named\n\
             subtree*, and a green status with no name in it meets the status and not\n\
             the exit.",
        )?
        .to_string();

    println!("\ncompare: ok — the divergence was localised to `{named}`\n");
    println!("{WHOLE_SYSTEM_GAP}");
    Ok(())
}
