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

/// Content addressing and archiving, with no dependency outside the tree.
/// Split out because it is algorithm rather than policy, and because the rest
/// of this file is a list of decisions while that one is a list of constants.
mod pack;

/// The component manifest schema, as a check. Split out because it is a parser
/// with a schema behind it rather than a grep with a policy behind it, and
/// because `docs/manifest.md` names it as the place the schema is code.
mod manifest;

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

/// The component crates that publish a *zero copies on the data path* counter,
/// and the one function in each that is permitted to move bytes.
///
/// # Why this is a lint and not a comment
///
/// `E1-B02`'s exit is *zero copies on the data path, verified by counter*, and
/// the counter it names is structurally zero: a request resolves to a `Reach`,
/// which is an address and a length and deliberately not a slice, so the
/// address goes into a descriptor and the bytes never reach the component. That
/// is a statement about what the crate's *source* contains, not about what a
/// boot observed — a zero counter published by a crate that had grown a second
/// way to move bytes would say exactly the same thing as this one, which is the
/// defect `state::node::MEMORY_FORCED` exists to keep out of the allocator's
/// number one subsystem over.
///
/// So the structure is checked rather than asserted. For each row: the mover
/// must be defined exactly once, called exactly once, and called from the named
/// function and from nowhere else. A fourth clause used to sit here — no
/// shipped line may mint a granted window out of a bare address — and [`MINTS`]
/// is where it went and why.
///
/// Each row is `(crate prefix, the function that moves bytes, the one function
/// allowed to call it)`. `E1-B03` and `E1-B04` join by adding a row; a driver
/// that makes no zero-copy claim adds none, and says so in its manifest.
const DATAPATH: &[(&str, &str, &str)] = &[
    ("user/virtio-blk/", "stage", "provoke_copy"),
    // `E1-B03`, joining the way the paragraph above said a second driver
    // would: one row, no change to the check. What is worth stating is that
    // the claim behind the row is *harder* on this crate than on the one
    // above it. A block driver could be zero-copy by accident, because a
    // client's bytes go past it in both directions and it never needs to
    // look at them; a network driver is the only thing between a device and
    // a client's buffer on the receive path, and the obvious implementation
    // reads the frame to find out how long it is. This one takes the length
    // off the used ring instead, and that is what the row holds it to.
    ("user/virtio-net/", "stage", "provoke_copy"),
    // `E1-B04`, and the row is here to be *weaker* than the two above it rather
    // than stronger. A display driver is not between the device and the pixels
    // in either direction — the device reads a client's buffer and writes to a
    // screen — so a zero here is easier to hold than the network driver's and
    // saying otherwise would be dishonest. What the row still buys is the shape:
    // one function that moves bytes, called once, from the boot's own
    // self-check, so the published zero remains a measurement rather than an
    // absence.
    ("user/virtio-gpu/", "stage", "provoke_copy"),
];

/// The constructors that turn a bare address into a granted window.
///
/// **Empty, and the emptiness is the point — read the paragraph before
/// widening it back.**
///
/// It held `Region::at(` and `Window::at(` for as long as the driver's code ran
/// *in the frame*. The reason was exact and is worth keeping: those are safe
/// `const fn`s, the frame's direct map covers all of physical memory, and a
/// driver crate forbidding `unsafe` could therefore have named a client's page
/// through one, read the bytes, published a copy count of zero, and been
/// telling the truth about `stage` while lying about the datapath. The lint was
/// the only thing standing there.
///
/// It is not the only thing standing there now. RFC 0047 schedules the driver
/// at ring 3, where the pages mapped for it are its text, its stack, its two
/// rings, its board, its device's registers and its own queue memory — and
/// nothing else in the machine. An address it invents is a page fault, taken by
/// the component, reported by the frame as an ordinary ring-3 fault, and
/// `cargo xtask blk` fails. That is an address space refusing what a source
/// scan could only look for, and it is the enforcement `kernel/src/blk.rs`
/// named as arriving *with the scheduler* rather than the one it had.
///
/// So the row stays, the mover check stays — one function that moves bytes,
/// called once, from `provoke_copy` — and this list is empty. **What would put
/// something back in it is a component whose code the frame runs.** If a driver
/// is ever linked into the frame again for any reason, the direct map is under
/// it again and this is the check that was holding.
const MINTS: &[&str] = &[];

/// Where a component's code may not be called from, what a call looks like,
/// and where that name has to exist for the rule to mean anything.
///
/// One row, and it is RFC 0033's own reversal condition made executable:
/// *grep for `Driver::execute` and see which crate calls it.* The answer was
/// `kernel/` for the whole of E1-B02 and E1-B08, it is `user/virtio-blk` since
/// RFC 0047, and this is what notices the day it goes back.
///
/// The needle is `Driver::` rather than `Driver::execute` on purpose. A frame
/// that had gone back to calling the driver would not necessarily call
/// *execute* first — it would call `Driver::start`, because that is the one
/// that brings a device up — so a check spelled after the symptom would miss
/// the cause. What it does not match is `DriverPlan` and `Trouble::Driver`,
/// which are the frame's own types and name nothing of the component's.
///
/// # Why there is a third field
///
/// Because a needle is a *name*, and this check looks for its absence. Rename
/// `Driver` in `user/virtio-blk`, or add a second driver crate whose type is
/// called anything else, and the absence under `kernel/` is satisfied by a
/// string that no longer refers to anything — green while the frame runs a
/// component's code, with the direct map back under a crate whose `copies = 0`
/// this tree publishes as a property. That is the same defect [`DATAPATH`]'s
/// `defined != 1` clause exists to refuse one field over, and it is refused the
/// same way: the needle must be *present* under the crate that owns it. A check
/// whose needle nothing defines is indistinguishable from a rule that holds.
///
/// Each row is `(the prefix that must not name it, the needle, the prefix that
/// must)`. A second driver therefore **does** add a row, because the needle is
/// spelled after a type and each crate spells its own — `E1-B03` and `E1-B04`
/// each add one. The rule stays about the frame; what the third field pins is
/// that the rule still has a subject.
const NOT_THE_FRAME: &[(&str, &str, &str)] = &[
    ("kernel/", "Driver::", "user/virtio-blk/"),
    // The second row the doc comment above predicted, and it is not
    // redundant with the first even though the needle is the same string.
    // The first field is a rule about `kernel/`; the third is what keeps
    // that rule from being satisfied by a name nothing defines, and each
    // crate spells its own. Delete `user/virtio-net`'s `Driver` and this
    // row goes red, which is exactly what the third field is for.
    ("kernel/", "Driver::", "user/virtio-net/"),
    // The third, and the doc comment above predicted it exactly. The needle is
    // the same string for the third time and the third field is what keeps the
    // rule from being satisfied by a name nothing defines.
    ("kernel/", "Driver::", "user/virtio-gpu/"),
];

/// The reversal conditions that have fallen due and are **not paid**, declared
/// as a set rather than left as a paragraph in three documents.
///
/// # Why this is data and not a sentence in an RFC
///
/// RFC 0036 is the precedent, `CHAOS_GAP` is the precedent's second use, and the
/// argument does not change: a deviation that lives in prose is a deviation
/// nobody re-checks, and the failure mode is not that it is never fixed — it is
/// that it *is* fixed and three documents go on describing it. So each entry
/// names a file and the exact text whose **presence** keeps the deviation open,
/// and this check requires every one of them to still be there. The day one
/// goes, the build goes red and tells whoever closed it which documents now
/// describe a tree that does not exist.
///
/// Three entries, and each of them is an RFC's own words:
///
/// - **RFC 0008.** *Restart is the supervisor's act and the frame provides only
///   the mechanism.* The policy runs in the frame. `component::policy::decide`
///   was written to take a record and a tally and no kernel state precisely so
///   that moving it would be a move rather than a rewrite, and what it is
///   waiting for is not a place to move to but a supervisor to move into: a
///   component that can be told its occupant died and can say *spawn it again*.
///   RFC 0047 built the half of that a driver needed — a component asks the
///   frame for something on its control ring and the frame answers — and did
///   not build `op::SPAWN` or `op::STOP` behind it.
/// - **RFC 0014.** `ANNOUNCE` and `PROGRESS` retire when a component is started
///   with a channel and told on it. The channel exists now and carries
///   operations in both directions; what a component still cannot do is *be
///   started* by anything but the frame writing a job into a per-core slot, so
///   `ANNOUNCE` has nothing to announce itself onto that the frame did not
///   already know.
/// - **RFC 0015.** The four capability calls retire onto
///   `control::op::INSPECT`, `DERIVE`, `REVOKE` and `MAP`. All four opcodes are
///   named in `abi/src/control.rs` and nothing implements them; the two that
///   *are* implemented are RFC 0047's, and they are the two a driver could not
///   do without.
///
/// **Read this before deleting a row.** Emptying one because the work *could*
/// be done is the failure the constant exists to prevent. A row goes when the
/// text it names goes, and what replaces it is a boot that shows the new thing
/// happening.
const OWED_REVERSALS: &[Gap] = &[
    (
        "kernel/src/component.rs",
        "policy::decide(",
        "RFC 0008: the restart policy runs in the frame, where that RFC says it does not belong",
        "TODO.md E1-B05; docs/rfc/0008; kernel/src/component.rs's module comment; \
         claims/0006-driver-restart-latency.toml's [workload] notes",
    ),
    (
        "abi/src/door.rs",
        "pub const ANNOUNCE",
        "RFC 0014: `ANNOUNCE` and `PROGRESS` are still on the door, because nothing starts a \
         component with a channel",
        "TODO.md E1-B05; docs/rfc/0014; abi/src/door.rs's module comment",
    ),
    (
        "abi/src/door.rs",
        "pub const CAP_INSPECT",
        "RFC 0015: the four capability calls are still on the door, because the four control-ring \
         opcodes that retire them are named and unimplemented",
        "TODO.md E1-B05; docs/rfc/0015; abi/src/door.rs's module comment; the four \
         unimplemented opcodes in abi/src/control.rs",
    ),
    // `E1-B03`'s, and the first entry here that was found by *running out* of
    // something rather than by reading a document. The frame's driver shape maps
    // one page of stack; a component has no allocator, so everything a driver
    // holds lives in it; and the second driver overran it by fifty-six bytes at
    // eight receive slots — a page fault at the guard, observed. The number in
    // that crate is therefore a bound on the frame and not on the protocol, and
    // it is declared here so that the day the frame gives a driver a stack, the
    // build says which documents describe a wall that is gone.
    (
        "user/virtio-net/src/driver.rs",
        "RECEIVE_SLOTS_STACK_BOUND",
        "RFC 0051: a scheduled driver gets one page of stack, so the network driver posts four \
         receive buffers rather than as many as its clients would give it",
        "docs/rfc/0051; user/virtio-net/src/driver.rs's RECEIVE_SLOTS_STACK_BOUND; \
         kernel/src/net.rs's module comment; kernel/src/process.rs's SPAWN_STACK",
    ),
    // `E1-B04`'s, and it is a promise rather than a wall: RFC 0051 said *what
    // would merge them is a third driver, at which point the shared half moves
    // out of both and neither is closed evidence any more*. There are three
    // drivers now and the half has not moved. RFC 0054 argues why — the move
    // rewrites `kernel/src/blk.rs`, which is the evidence a closed task's exit
    // rests on, in a task whose own evidence is a picture on a screen — and this
    // row is what stops that argument from quietly becoming permanent. The
    // needle is the type the three supervisors duplicate; the day it leaves
    // `blk.rs` the build names every document that says it is still there.
    (
        "kernel/src/blk.rs",
        "struct Supervising",
        "RFC 0051: three driver supervisors hold one `Registers`, `Supervising`, `Reported`, \
         `declared` and `order_for` between them, and the third driver was to have merged them",
        "docs/rfc/0051; docs/rfc/0054; the module comments of kernel/src/blk.rs, \
         kernel/src/net.rs and kernel/src/gpu.rs; kernel/src/gpu.rs's Registers",
    ),
];

/// Every entry of a declared set is still true.
///
/// Shared by [`OWED_REVERSALS`] and by `chaos`'s own gap, because the two are
/// one discipline used twice and a second copy of the reading would be a second
/// place for it to rot.
///
/// # Errors
///
/// A needle that is gone, which is good news and a red build on purpose, or a
/// file that is not there — which is a declaration nobody can check.
fn gap_holds(what: &str, gap: &[Gap]) -> Result<(), String> {
    gap_holds_under(&root(), what, gap)
}

/// One declared deviation: where it lives, what keeps it open, why it is still
/// there, and **which documents say so**.
///
/// # Why there is a fourth field
///
/// Because the failure this whole discipline exists to prevent happened anyway,
/// one row over. RFC 0047 closed half of `CHAOS_GAP` and paid RFC 0033's
/// reversal; two constants and one module comment were updated, and five other
/// live documents went on describing the tree that had gone. The refusal below
/// said *every document that describes the same deviation ... update them* and
/// named none of them, which is an instruction that assumes the reader already
/// knows the answer. So the answer is data, and the day a gap closes the build
/// prints the list.
///
/// It is a list of documents rather than a set of paths a lint reads, and that
/// is deliberate: half of these entries are `TODO.md`, which this tree's agents
/// may not edit, and a check that refused a stale sentence in a file nobody may
/// touch would be a check that has to be switched off.
type Gap = (&'static str, &'static str, &'static str, &'static str);

/// [`gap_holds`], against a directory the caller names.
///
/// Split out so the mechanism has a fixture. Four declared quantities rest on
/// this function — `OWED_REVERSALS`'s three rows and `CHAOS_GAP`'s one — and
/// every one of them is worth exactly what this is: a check that has never
/// failed is indistinguishable from a check that cannot.
///
/// # Errors
///
/// As [`gap_holds`].
fn gap_holds_under(base: &Path, what: &str, gap: &[Gap]) -> Result<(), String> {
    for (file, needle, _, describes) in gap {
        let path = base.join(file);
        let text = std::fs::read_to_string(&path).map_err(|e| {
            format!(
                "reading {file}: {e}\n\nThe declared {what} gap names a file that is not there, \
                 which is a declaration nobody can check."
            )
        })?;
        if !text.contains(needle) {
            return Err(format!(
                "`{needle}` is gone from {file}.\n\n\
                 That is a declared gap closing, which is good news and a red build on\n\
                 purpose: `{what}` in xtask, and every document that describes the same\n\
                 deviation, now describe a tree that no longer exists.\n\n\
                 These are those documents. Update them in the diff that closes it, not\n\
                 in the one after:\n\
                 \x20  {describes}\n\n\
                 Narrowing is what to do here rather than emptying: shrink the constant to\n\
                 exactly what is still true, or delete the row and say in an RFC why nothing\n\
                 is."
            ));
        }
    }
    Ok(())
}

/// The reversal conditions this tree owes, still owed.
///
/// # Errors
///
/// One of them having been paid. See [`OWED_REVERSALS`].
fn lint_owed() -> Result<(), String> {
    gap_holds("OWED_REVERSALS", OWED_REVERSALS)?;
    println!(
        "lint-owed: ok  ({} reversal condition(s) fallen due and unpaid, each still unpaid)",
        OWED_REVERSALS.len()
    );
    for (file, _, why, _) in OWED_REVERSALS {
        println!("  {file:<28} {why}");
    }
    Ok(())
}

/// One file, against one row of [`NOT_THE_FRAME`].
///
/// Comments are stripped first, for the reason they are stripped everywhere
/// else in this file: this tree explains its reversals in prose, so the line
/// that says *`Driver::execute` used to be called here* must not be the line
/// that fails the build.
/// How many shipped lines under one file name `needle`.
///
/// The presence half of [`NOT_THE_FRAME`]. Comments are stripped for the reason
/// they are stripped in [`frame_findings`] — the sentence recording that the
/// frame used to call this must not be what keeps the rule alive — and the scan
/// stops at `#[cfg(test)]`, so a fixture naming the type is not a definition of
/// it either.
fn code_mentions(text: &str, needle: &str) -> usize {
    let mut seen = 0;
    let mut carry = Carry::default();
    for raw in text.lines() {
        let code = strip_to_code(raw, &mut carry);
        if code.trim().starts_with("#[cfg(test)]") {
            break;
        }
        if code.contains(needle) {
            seen += 1;
        }
    }
    seen
}

fn frame_findings(rel: &str, text: &str, needle: &str) -> Vec<String> {
    let mut findings = Vec::new();
    let mut carry = Carry::default();
    for (n, raw) in text.lines().enumerate() {
        let code = strip_to_code(raw, &mut carry);
        if code.trim().starts_with("#[cfg(test)]") {
            break;
        }
        if code.contains(needle) {
            findings.push(format!(
                "  {rel}:{}  the frame names `{needle}` — a component's code, called from \
                 inside the frame",
                n + 1
            ));
        }
    }
    findings
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(String::as_str).unwrap_or("help");

    let result = match cmd {
        "build" => build(),
        "run" => run(),
        "orders" => orders(),
        "fault" => fault(args.get(1).map(String::as_str)),
        "user" => user(args.get(1).map(String::as_str)),
        "cap" => cap(args.get(1).map(String::as_str)),
        "iommu" => iommu(args.get(1).map(String::as_str)),
        "blk" => blk(args.get(1).map(String::as_str)),
        "net" => net(args.get(1).map(String::as_str)),
        "gpu" => gpu(args.get(1).map(String::as_str)),
        "deadline" => deadline(args.get(1).map(String::as_str)),
        "runtime" => runtime(args.get(1).map(String::as_str)),
        "init" => init_image().map(|path| println!("{}", relative(&path))),
        "component" => components().map(|_| ()),
        // E1-P06. Every component the build produced, killed under sustained
        // load and again with nothing killed. The verdict is `f-sim`'s and this
        // is the driver: the component directory, the two processes the
        // reproduction check needs, and the declared gap between what the
        // simulator kills and what a boot can. RFC 0041.
        "chaos" => chaos(),
        "mutate" => mutate(),
        "prove" => prove(args.get(1).map(String::as_str)),
        // E1-B14. What an unmap costs under churn, counted both ways in one
        // boot, and the host workload beside the E1-P10 claims that asks the
        // same question of a clock which refuses to answer. RFC 0052.
        "churn" => churn(),
        // E1-P03. The verb `xtask` owns is the driver: the commit, the
        // component directory and the wall clock. Everything that decides a
        // verdict is in `f-sim`, where no clock can reach it. RFC 0040.
        "sweep" => sweep_verb(args.get(1..).unwrap_or_default()),
        // E1-P04. A peer that writes arbitrary values to the shared header and
        // cursors, restarts mid-operation and lies about its epoch, generated
        // from a seed. Three properties, three counts, three defects — one per
        // property, because a harness with one defect in it exercises one
        // property and decorates the others. RFC 0046.
        "hostile" => hostile_verb(args.get(1..).unwrap_or_default()),
        // E1-P05. A submission entry, generated by its structure and kept by
        // its coverage. RFC 0048.
        "entries" => entries_verb(args.get(1..).unwrap_or_default()),
        // E1-B07. A reservation refused when it does not fit, and one that does
        // put under adversarial load with two controls beside it. The boot half
        // asks this machine what it can reserve; the model half is `f-sim`'s,
        // where a virtual clock makes *met its deadline* a count rather than a
        // property of the emulator. RFC 0050.
        "admission" => admission_gate(),
        // E1-P08. A long run, marked as it goes and re-entered one minute
        // before it fails, with both wall-clock numbers printed. RFC 0043.
        "snapshot" => snapshot(),
        "panic" => panic_path(),
        "trace" => match args.get(1).map(String::as_str) {
            Some("--hash") => trace_hash_only(),
            Some(other) => Err(format!("unknown option for trace: {other}")),
            None => trace_check(),
        },
        // The simulator's half of the same question `trace` asks about a boot.
        // RFC 0032 is where the seam between the two is argued; `sim.rs` in
        // `f-sim` is where it is implemented.
        "sim" => match args.get(1).map(String::as_str) {
            Some("--list") => sim_list(),
            // The seam, and the only command in the tree that runs both halves
            // of `boot-to-workload` and requires them to be about one component
            // set. RFC 0035.
            Some("--join") => sim_join(),
            Some("--hash") => {
                sim_hash_only(args.get(2).map(String::as_str), args.get(3).map(String::as_str))
            }
            Some(other) => Err(format!("unknown option for sim: {other}")),
            None => sim_check(),
        },
        // `reproduce` used to mean the determinism check above, and it now
        // means what `RELEASING.md`, the long plan and `proving-ground` all use
        // the word for: re-running a published number. The old spelling gets a
        // message naming where it went rather than an alias, because an alias
        // rots and a signpost is read exactly when it is needed.
        "reproduce" => match args.get(1).map(String::as_str) {
            Some("--trace") => Err("`reproduce --trace` moved. The determinism check is now
                 `cargo xtask trace`, and one hash is `cargo xtask trace --hash`.
                 `reproduce` takes a claim name — see `cargo xtask reproduce`."
                .into()),
            other => reproduce(other),
        },
        "timer" => timer(args.get(1).map(String::as_str)),
        "test" => test(),
        // The two halves of `test`, separately, because CI runs them on
        // different machines: `test-host` on both runners, `cross` wherever the
        // policy job lands. E1-P11.
        "test-host" => test_host(),
        "cross" => cross_check(),
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
        "lint-manifests" => lint_manifests(),
        "lint-components" => lint_components(),
        "lint-datapath" => lint_datapath(),
        "lint-owed" => lint_owed(),
        "lint-arch-tests" => lint_arch_tests(),
        "lint-snapshot" => lint_snapshot(),
        "lint-reproduce" => lint_reproduce(),
        "lint-proofs" => lint_proofs(),
        "unsafe" => unsafe_report(args.get(1).map(String::as_str) == Some("--by-file")),
        "release" => release(args.get(1).map(String::as_str)),
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
  orders             Boot on a machine with a gibibyte in it and require the
                     allocator to serve order 18 — the largest order it admits,
                     and one the 128 MiB fixture has no memory for
  fault [kind]       Boot it into a deliberate fault and check the report:
                     pf, ud, df, nx, wx or stack
  user [kind]        Boot into a process that violates one isolation property
                     on purpose and check the kernel survives it: kernel, null,
                     text, stack, priv, call or exit. All seven with no argument
  cap [kind]         Boot into a process that tries to escape its capabilities
                     and check the frame refuses it: grant, unowned, forge,
                     stale, rights, type, flood or unmap. All eight with no
                     argument
  iommu [half]       Boot into a real device transfer and check the remapping
                     unit: inside, which must land, or outside, which must
                     fault and land nothing. Both with no argument
  blk [half]         Boot the block datapath: a driver component moves a sector
                     through a ring with nothing copied — inside; the same run
                     with the client's grant withdrawn must fault — outside; and
                     the driver pointing the device past what it was answered
                     must be faulted at the address it invented — escape. All
                     three with no argument
  net [half]         Boot the network datapath: a second driver component
                     puts a frame on a link and the answer lands in a registered
                     buffer with nothing copied — inside; the identical client
                     with nothing sent, where nothing may arrive — silent; and
                     the driver pointing the device past what it was answered on
                     the descriptor the *device writes* — escape. All three with
                     no argument
  gpu [half]         Boot the display datapath: a third driver component puts a
                     client's pixels on a scanout through a ring, and this
                     harness captures the emulator's framebuffer and requires it
                     to hold them — inside; the identical client with nothing
                     submitted, where nothing may reach the screen — blank; and
                     the driver pointing the device past what it was answered at
                     the memory a display *reads* — escape. All three with no
                     argument. The only check here that observes something from
                     outside the machine, because a scanout cannot be read back
                     from inside one
  deadline [half]    Boot the block datapath with batch work queued and a
                     hard-class read submitted behind it: ordered, where the
                     read must be handed to the device first; arrival, the
                     control, where the same burst must put it last; and
                     unadmitted, where a client that does not hold the hard
                     class writes it and must be refused. All three with no
                     argument
  runtime [half]     Boot a component that holds a core and schedules its own
                     work inside it: load, which must cross the boundary only on
                     its way out; provoke, which crosses once on purpose so the
                     counter is shown to move; reclaim, where the timer posts a
                     notice under load and the runtime must park cleanly; and
                     hostile, where its control ring header is scribbled and
                     adoption must refuse. All four with no argument
  init               Build user/init into the flat image the loader hands over,
                     and check it is one
  component          Build every component file: a manifest compiled to its
                     record, its image linked, and one content hash over both
  admission          Refuse an over-subscribed reservation and put a granted
                     one under adversarial load, with two controls beside it:
                     the same load without a reservation, which must miss, and
                     an over-subscription, which must be refused ADMISSION and
                     run nothing. Then ask this machine what it can reserve
  chaos              Kill every component under sustained load at seeded
                     moments, and again with nothing killed. No client may
                     observe anything except added latency
  mutate             Build the kernel with a deliberate defect, boot it, and
                     require the boot to go red — then require the same boot to
                     go green without it
  prove [harness]    Bounded model checking, in two crates: the five capability
                     properties over arbitrary handles, and the ring's
                     validation paths over arbitrary peer bytes. Every harness
                     must pass, at both bounds where the bound does not bind it,
                     and then each deliberate defect must fail the harness that
                     states the property it breaks. Needs Kani, `full` image
  timer [seconds]    Run the 1 kHz timer and print a jitter histogram. Sixty
                     seconds by default. A measurement, not an assertion
  test               test-host, then cross. Both halves, on this machine
  test-host          The host suite on whatever architecture this is, derived
                     from the workspace rather than from a list beside it. CI
                     runs it on x86-64 and on the arm runner
  cross              Compile every crate that reaches the machine for
                     aarch64-unknown-none, and print what is excluded and why
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
  lint-manifests     Every component manifest fits docs/manifest.md; RFC 0005
                     rule 4 and RFC 0008's shape, checked before a spawn does
  lint-components    The components this tree declares are the components it
                     builds — a manifest with nothing building it is a component
                     no downstream count ever looks for
  lint-datapath      The mechanism behind `blk/copies`: each crate that claims
                     zero copies moves bytes in exactly one function, calls it
                     from exactly one place, and that place is not the data path,
                     and no part of it is called by the frame
  lint-owed          The reversal conditions RFC 0008, RFC 0014 and RFC 0015
                     name and this tree has not paid, declared as a set — red
                     the day one of them is paid and the documents go stale
  lint-snapshot      claims/snapshot.json holds what the registry holds
  lint-arch-tests    No test is compiled on one architecture and not the other
                     without a reason and a reversal recorded beside it
  lint-proofs        kernel/proofs and ring/proofs still compile against the
                     code they prove, under the pinned toolchain and in every
                     feature configuration `prove` builds. Both are outside the
                     workspace, so nothing else in the gate builds them

  unsafe             The number A-05 reports: lines inside `unsafe` as a share
                     of the frame crates and of the whole tree, against
                     RFC 0001's under-5% target and 10% reversal trigger

  trace              Two runs of this commit must produce one trace hash,
                     and one unseeded read of time must break that
  trace --hash       Print this run's trace hash and nothing else

  sim                Run every simulator scenario twice at one seed and once at
                     another: the pair must agree and the odd one must not
  sim --hash [name] [seed]
                     Print one scenario's trace hash and nothing else. The pair
                     is (seed, commit): the seed is this argument or the default
                     below, and the commit is the checkout you are standing in
  sim --join         Boot the real kernel, read the component hashes out of its
                     log, and require the set the simulator runs to be the set
                     the boot spawned. The two halves of boot-to-workload,
                     joined at an artefact rather than at a sentence
  sim --list         The scenario set

  sweep [n] [m]      N seeds across M scenarios, every failure minimised to a
                     reproduction command that judges itself. 64 seeds and every
                     scenario by default; the wall-clock cost is printed beside
                     the report and is in no verdict in it. A grid too large for
                     one process is run as consecutive shards of the same seed
                     derivation, which is a fact about memory and not coverage
  sweep --help       What a seed is, what the scenario set is, what a finding
                     looks like and what to do with one. The published entry
                     point for somebody who has just cloned this
  sweep --base <s>   The seed the sweep derives from, for any of the forms
                     below. The default is the tree's own; a nightly varies it
                     so that successive nights cover new seed space, and every
                     report names the base it used
  sweep --mutate     Arm a deliberate defect in the simulator, require the sweep
                     and the corpus to find it, disarm it, and require both to
                     go quiet
  sweep --corpus     Replay every trial in sim/corpus.txt and require each to be
                     clean. The permanent regression half of a seed sweep
  sweep --record [n] Sweep, and merge what it finds into sim/corpus.txt
  sweep --record --mutate
                     The same, with the deliberate defect armed. This is how
                     the entries in sim/corpus.txt were produced

  hostile [n]        A peer that writes arbitrary values to the shared header
                     and cursors, restarts mid-operation and lies about its
                     epoch, drawn from a seed. No panic and no hang, and the
                     hang is a *count* rather than a timeout. A hundred million
                     operations by default; the exit's billion takes 44-60 s here
  hostile --exit     E1-P04's own number — one billion operations — by name
                     rather than as a literal a workflow file would keep a
                     second copy of
  hostile --miri     The third property, memory unsafety, under the only tool
                     that can see it — at four thousand operations rather than
                     a billion, because Miri costs six orders of magnitude and
                     saying so is the point
  hostile --mutate   Arm the two defects this half can see, require each to be
                     found by the property it breaks, and show that the third
                     is invisible here
  hostile --miri --mutate
                     Arm the third and require Miri to report it
  hostile --corpus   Replay every run in ring/corpus.txt and require each to be
                     clean. Add --miri to replay them under Miri
  hostile --record   Arm the defects and merge what they find into
                     ring/corpus.txt. This is how its entries were produced
  hostile --base <s> The seed the episodes derive from, for any of the forms
                     above. The default is the tree's own; a nightly varies it
                     so successive nights cover new seed space, and every
                     report names the seed it used

  entries [n]        Generate submission entries by their structure — a real
                     opcode with a wrong flag, a live set id with an index one
                     past the set, a length one byte past the arena — and
                     require three oracles to hold: the envelope is refused
                     with the code R04 names, an id is never reissued, and a
                     resolved buffer is inside its own set
  entries --coverage What share of the entry-validation path the committed
                     corpus covers, function by function, out of the same
                     instrumentation `coverage` reports from. The number
                     claims/0009 publishes
  entries --record   Draw with the per-case coverage signal on, keep what
                     reaches something new, minimise, and write
                     ring/entries-corpus.txt. The one build with feedback in it
  entries --corpus   Replay every case in ring/entries-corpus.txt and require
                     each to be clean
  entries --mutate   Arm the three defects, one per oracle, and require each to
                     be found by the oracle it breaks and by no other
  entries --base <s> The seed the episodes derive from, for any of the forms
                     above

  snapshot           A long run that goes wrong in simulated minute 40,
                     re-entered at minute 39 from a snapshot written while it
                     passed — with both wall-clock numbers, because *bisects in
                     seconds rather than hours* is a claim about time

  reproduce          Every claim, its published reproduction command, the
                     machine class it needs, and whether this one may record
  reproduce <claim>  Re-run one claim's own reproduction, from this checkout
  lint-reproduce     Every claim's reproduction command resolves in this tree

  panic              Three endings CI must tell apart: a clean boot, a
                     deliberate panic, and a boot that never finishes

  release            Build the package: the contract's contents, a MANIFEST,
                     and one content address over the whole of it
  release --dry-run  The manifest it would produce, without building
  release --twice    Package the same tree twice and require one address
  release --address  The address alone, one line, for two runners to compare

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

/// [`capture`], for output that is not text.
///
/// `git archive` writes a tar to standard output, and a tar is not UTF-8.
fn capture_bytes(program: &str, args: &[&str]) -> Result<Vec<u8>, String> {
    let out = Command::new(program)
        .args(args)
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if !out.status.success() {
        return Err(format!("{program} {} failed", args.join(" ")));
    }
    Ok(out.stdout)
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

/// Where a flat image is built, and where the boot loader is told to find it.
///
/// One directory per image, because each is compiled with different flags from
/// everything else in the workspace — see [`flat_image`] — and two sets of flags
/// sharing a target directory is two full rebuilds every time the build
/// alternates between them.
fn image_dir(name: &str) -> PathBuf {
    target_dir().join(name)
}

/// The components this tree builds, in the order the loader carries them.
///
/// Module one is `user/init`'s flat image and is not in this list: it has no
/// manifest, every existing boot depends on it being first, and RFC 0030 says
/// why that position is the contract. Everything here follows it, each as one
/// module holding a record and an image.
const COMPONENTS: &[&str] = &["store", "virtio-blk", "virtio-net", "virtio-gpu"];

/// Every component the *source tree* declares, by the name in its manifest.
///
/// # Why this exists beside [`COMPONENTS`]
///
/// Because [`COMPONENTS`] is a hand-written list and every check that read the
/// build output was reading it back. `cargo xtask chaos` compared the components
/// its sweep killed with the components the deployment directory held — two
/// reads of one directory, so the comparison could not fail, and a component
/// dropped from [`COMPONENTS`] would have vanished from both sides at once and
/// printed `coverage 1 of 1` over half the tree. That is the third time in this
/// epoch a join has been found comparing a set with itself, which is why the
/// answer here is a second, independently derived set rather than a better
/// message.
///
/// The independent derivation is the one thing in the tree that cannot be
/// hand-maintained: a `manifest.toml` is what makes a directory a component, and
/// `manifest::files` finds them by walking. `user/init` has none — RFC 0030 says
/// why module one is not a component — so what comes back is exactly the set a
/// deployment should hold.
///
/// # Errors
///
/// A manifest that does not parse, which [`lint_manifests`] reports properly.
/// Here it is a hard error, because a set derived from a file nobody could read
/// is not a set to compare anything against.
fn declared_components() -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for path in manifest::files(&root(), &target_dir())? {
        let rel = relative(&path);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        let checked = manifest::check(&rel, &text).map_err(|findings| {
            format!(
                "{rel} does not fit the schema, so the set of components this tree declares \
                 cannot be read:\n{}",
                findings.join("\n")
            )
        })?;
        names.push(checked.name);
    }
    names.sort();
    Ok(names)
}

/// The build list and the source tree name the same components.
///
/// [`COMPONENTS`] decides what is built and therefore what every downstream
/// check sees — the deployment scenario, `sim --join`, and `cargo xtask chaos`.
/// Nothing tied it to the manifests the tree declares, so a component crate
/// added with a manifest and left out of the list was a component no check ever
/// looked for: it was absent from the build output and absent from every count
/// taken over the build output, which is a gap that reads as a pass.
///
/// # Errors
///
/// The names in one set and not the other, in both directions.
fn lint_components() -> Result<(), String> {
    let declared = declared_components()?;
    let mut built: Vec<String> = COMPONENTS.iter().map(|name| (*name).to_string()).collect();
    built.sort();
    if declared == built {
        println!(
            "lint-components: ok  ({} component(s); the build list is the manifest set)",
            built.len()
        );
        return Ok(());
    }
    let missing: Vec<&String> = declared.iter().filter(|name| !built.contains(name)).collect();
    let extra: Vec<&String> = built.iter().filter(|name| !declared.contains(name)).collect();
    Err(format!(
        "the components this tree declares and the components it builds are not the same set.\n\n\
         declared and not built: {}\n\
         built and not declared: {}\n\n\
         `COMPONENTS` in xtask/src/main.rs is what decides what is built, and everything\n\
         downstream — the deployment scenario, `sim --join`, `cargo xtask chaos` — counts what\n\
         was built. A component missing from that list is therefore missing from both sides of\n\
         every one of those comparisons at once, which is a gap that reads as a pass. Add it\n\
         there, or delete its manifest.",
        if missing.is_empty() { "none".to_string() } else { join_names(&missing) },
        if extra.is_empty() { "none".to_string() } else { join_names(&extra) },
    ))
}

/// A list of names for a refusal, comma-separated.
fn join_names(names: &[&String]) -> String {
    names.iter().map(|name| name.as_str()).collect::<Vec<_>>().join(", ")
}

/// Where a component file is written: a manifest record and an image, in one
/// blob the loader hands over as one module. RFC 0030.
fn component_path(name: &str) -> PathBuf {
    target_dir().join("component").join(format!("{name}.fc"))
}

/// Build one component file: check its manifest, compile it to a record, link
/// its image, and put the two in one blob.
///
/// The order is the point. `manifest::compile` runs the same checker
/// `cargo xtask lint-manifests` runs — one parser, not two — and refuses before
/// anything is built, so a manifest that stopped fitting the schema is a build
/// failure naming the field rather than a component the frame refuses much
/// later. RFC 0030.
fn component_image(name: &str) -> Result<(PathBuf, u64, usize), String> {
    let image = flat_image(&format!("f-{name}"), name)?;
    let bytes = std::fs::read(&image).map_err(|e| format!("reading {}: {e}", relative(&image)))?;

    let rel = format!("user/{name}/{}", manifest::FILE_NAME);
    let source = root().join(&rel);
    let text = std::fs::read_to_string(&source).map_err(|e| format!("reading {rel}: {e}"))?;
    let file = manifest::compile(&rel, &text, &bytes)
        .map_err(|findings| format!("{rel} does not fit the schema:\n\n{}", findings.join("\n")))?;

    let out = component_path(name);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", relative(parent)))?;
    }
    std::fs::write(&out, &file).map_err(|e| format!("writing {}: {e}", relative(&out)))?;
    Ok((out, manifest::content_id(&file), bytes.len()))
}

/// Build every component file, and say what each one is.
///
/// The identity it prints is what a spawn names — one hash over the record and
/// the image together — so a component whose *code* changed has a different
/// identity from one whose declaration did, and both are visible here rather
/// than at the boot that refuses to refill a place.
fn components() -> Result<Vec<PathBuf>, String> {
    let mut built = Vec::new();
    for name in COMPONENTS {
        let (path, id, image) = component_image(name)?;
        println!("  {name:<12} {id:#018x}  record + {image} byte image  {}", relative(&path));
        built.push(path);
    }
    Ok(built)
}

/// Where a component's text is mapped. `kernel::process::TEXT`.
///
/// Stated here as well as there and in `user/init/link.ld` because the three
/// are linked separately and there is nothing to share a constant through. The
/// check in [`init_image`] is what makes the duplication safe: it reads the
/// address the linker actually used.
const INIT_TEXT: u64 = 0x0040_0000;

/// How large a component's image may be, by the shape the frame builds it in.
///
/// **Two numbers, and the pair is the honest statement.** One page is what
/// `kernel::process::prepare` and `prepare_runtime` map, and every component
/// they build is one page of text; a *driver* is built by `prepare_driver`,
/// which reserves `kernel::process::TEXT_PAGES` — sixteen — and RFC 0047
/// argues the size. A single constant would have to be the larger of the two,
/// and would then stop refusing a `store` that had quietly grown past what the
/// frame maps for it.
///
/// It is still not a loader. Neither shape reads a component's headers; both
/// copy a flat image to a fixed address and jump to its first byte, and the
/// difference is only how many pages the frame reserved. E5 is where the
/// headers are read and where both of these stop existing.
/// Unit: bytes.
const IMAGE_MAX: &[(&str, u64)] =
    &[("virtio-blk", 16 * 4096), ("virtio-net", 16 * 4096), ("virtio-gpu", 16 * 4096)];

/// What a component whose shape [`IMAGE_MAX`] does not name may be.
/// Unit: bytes.
const INIT_MAX: u64 = 4096;

/// The bound for one component, by directory name.
///
/// The name and not the package, because the name is what the manifest, the
/// directory and the frame's own `Record::label` all agree on.
/// Unit: bytes.
fn image_max(dir: &str) -> u64 {
    IMAGE_MAX.iter().find_map(|(name, bytes)| (*name == dir).then_some(*bytes)).unwrap_or(INIT_MAX)
}

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
    flat_image("f-init", "init")
}

/// Build one crate into a flat image and check that it is one.
///
/// `package` is the crate, `dir` the target directory and the image's base
/// name. Every component in this tree is linked by `user/init/link.ld`, and
/// that is a claim rather than a convenience: the script places the entry by
/// matching the section `component::start` is compiled into, so every crate
/// whose entry has that path is placed the same way and checked the same way.
/// A component with a differently named entry would need its own script and
/// would be a second answer to a question this tree has one answer to.
/// Every archive cargo says it built, except the component's own.
///
/// Read out of `--message-format=json` rather than found by walking a
/// directory, because cargo's layout for intermediate artefacts is not
/// something this file is entitled to know: it has already moved once, and a
/// glob that stopped matching would produce an image with a member missing
/// rather than an error. The `filenames` field is cargo's stated interface for
/// exactly this question.
///
/// The scan is textual and does not parse JSON, which is a real limitation and
/// a deliberate one: `xtask` has no serialisation dependency, and the thing
/// being looked for is a quoted absolute path ending in `.rlib`. A path
/// containing a quote or a backslash escape would be missed, and the linker
/// would then fail with an undefined symbol — loudly, naming the symbol, which
/// is the failure mode this whole step already relies on.
fn artefacts(emitted: &str, own: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for piece in emitted.split('"') {
        if !piece.ends_with(".rlib") {
            continue;
        }
        let path = PathBuf::from(piece);
        if !path.is_absolute() || !path.exists() || path == own || found.contains(&path) {
            continue;
        }
        found.push(path);
    }
    found
}

fn flat_image(package: &str, dir: &str) -> Result<PathBuf, String> {
    let lld = llvm_tool("rust-lld")?;
    let objcopy = llvm_tool("llvm-objcopy")?;
    let nm = llvm_tool("llvm-nm")?;

    // The base name is kept before the directory shadows it, because the image
    // is named after the crate rather than called `image.bin` in a directory
    // that happens to say which crate it was. `target/init/init.bin` is a path
    // `tools/f-on-metal.sh` and `docs/booting-on-hardware.md` both write down,
    // and an artefact whose name is only meaningful relative to its directory
    // is one a script names wrongly the first time somebody moves it.
    let name = dir;
    let dir = image_dir(dir);
    let target =
        dir.to_str().ok_or("a flat image target directory is not valid UTF-8")?.to_string();

    // `relocation-model=static` for the same reason the kernel uses it: this is
    // a fixed-address image, and without it the crate compiles as position
    // independent and wants a relocation table nothing will process.
    //
    // `--message-format=json` on stdout, with stderr left where it was, because
    // the linker below needs to be told which archives cargo produced. Cargo's
    // own layout for intermediate artefacts is not a path this file gets to
    // know — it has already moved once — and asking cargo is the difference
    // between a build step and a build step plus a guess. Progress lines are on
    // stderr, so a caller still sees the same output it saw before.
    let mut child = Command::new("cargo")
        .args([
            "build",
            "-p",
            package,
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
            "--message-format=json",
        ])
        // `relocation-model=static` for the reason above.
        //
        // `panic=immediate-abort` because a component has no way to report a
        // panic and never will have one at this size: no serial port, no
        // unwinder, and a `#[panic_handler]` whose whole body is a halt. The
        // several kilobytes of formatting machinery a panic *message* needs are
        // therefore bought for a string nobody can read, in an image the frame
        // maps one page for — which is a real bound and not a preference, and
        // which `f_ring::adopt` arriving in a component pushed past.
        //
        // What it buys is also the more honest failure: a panic becomes `ud2`,
        // an invalid-opcode fault at ring 3 that the frame reports with a
        // vector, an address and an instruction pointer, and which RFC 0008
        // already names as one of the three ways a component ends. A component
        // parking silently tells nobody anything.
        //
        // Here rather than in `[profile.init]`, which is where it belongs,
        // because the profile key needs `cargo-features` at the top of the
        // workspace manifest — an opt-in that would apply to every build in the
        // tree to change one step. *Reversal:* the key stabilising.
        .env("RUSTFLAGS", "-C relocation-model=static -Zunstable-options -Cpanic=immediate-abort")
        .current_dir(root())
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    let mut emitted = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_string(&mut emitted).map_err(|e| format!("reading cargo's output: {e}"))?;
    }
    let status = child.wait().map_err(|e| format!("waiting for cargo: {e}"))?;
    if !status.success() {
        return Err(format!("building {package} failed"));
    }

    let rlib = format!("lib{}.rlib", package.replace('-', "_"));
    let archive = dir.join(KERNEL_TARGET).join("init").join(&rlib);
    if !archive.exists() {
        return Err(format!(
            "{package} did not produce {}\n\n\
             That is the library `link.ld` links. If it is missing, the crate has\n\
             stopped being a library — see the note in user/init/Cargo.toml.",
            relative(&archive)
        ));
    }

    let elf = dir.join(format!("{name}.elf"));

    // The component's own archive whole, and everything cargo built for it
    // beside — as archives, so the linker takes only what is reached.
    //
    // This used to be one library and nothing else, on the grounds that
    // everything a component called across a crate boundary was `#[inline]` in
    // `f_abi::door` and therefore already compiled in. That stopped being true
    // at E1-B08: a component that drives a ring calls `f_ring::adopt`, whose
    // bodies are in `f_ring`, and the linker said so — an undefined symbol is
    // an error here rather than a warning, which is what made the old comment
    // safe to state and what made this change a build failure rather than a
    // silently smaller image.
    //
    // `--whole-archive` on the component's own rlib because nothing refers to
    // anything: the entry is called by the kernel, so without it the linker
    // would pull in no members at all and produce an empty image.
    // `--no-whole-archive` before the rest, so a dependency contributes only
    // what the component actually reaches — the difference between linking a
    // ring and linking every ring. `--gc-sections` then takes back what the
    // entry does not reach, and the `KEEP()` in the script is what stops it
    // taking the entry too.
    let mut link = Command::new(&lld);
    link.args(["-flavor", "gnu", "-T", "user/init/link.ld", "--gc-sections", "--whole-archive"])
        .arg("-o")
        .arg(&elf)
        .arg(&archive)
        .arg("--no-whole-archive");
    for path in artefacts(&emitted, &archive) {
        link.arg(path);
    }
    let status =
        link.current_dir(root()).status().map_err(|e| format!("could not run rust-lld: {e}"))?;
    if !status.success() {
        return Err(format!("linking {package} against user/init/link.ld failed"));
    }

    // The symbol at the first byte. `link.ld` places the entry there by naming
    // the section pattern its function is compiled into; this is what says the
    // pattern still matches. A toolchain that changes how it names sections
    // makes this fail with a sentence, rather than making a boot jump into the
    // middle of some other function.
    let nm = nm.to_str().ok_or("llvm-nm's path is not valid UTF-8")?.to_string();
    let elf_path = elf.to_str().ok_or("the image elf path is not valid UTF-8")?.to_string();
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
            "the first byte of the {package} image is not `component::start`.\n\n\
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
    //
    // The two exceptions are the linker's own boundary markers, and excluding
    // them is a statement rather than a concession: `__image_start` and
    // `__image_end` are *addresses*, not objects, and `llvm-nm` classifies an
    // address by whichever output section it happens to land in. From E1-B08 a
    // component links `f_ring`, which brings an eight-byte global offset table
    // — addresses the linker resolved, written by nothing at run time — and
    // `lld` gives its output section the writable flag whatever
    // `user/init/link.ld` says. The end marker then lands in it and the check
    // reported the image's own extent as writable data.
    //
    // Nothing about the property is weakened: a real mutable global still has a
    // name of its own and is still caught, and `.data` and `.bss` being empty is
    // still what makes this image safe to map read-only.
    let writable: Vec<&str> = symbols
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let class = fields.nth(1)?;
            let name = fields.next()?;
            if name == "__image_start" || name == "__image_end" {
                return None;
            }
            matches!(class, "d" | "D" | "b" | "B" | "g" | "G" | "s" | "S").then_some(name)
        })
        .collect();
    if !writable.is_empty() {
        return Err(format!(
            "the {package} image has writable data: {}\n\n\
             Its text page is mapped read-only, so the first write to any of these\n\
             is a page fault in a component with no way to report one. A component\n\
             that genuinely needs writable state has to be given a frame for it —\n\
             which is a capability, and which E1's quota is about.",
            writable.join(", ")
        ));
    }
    let objcopy = objcopy.to_str().ok_or("llvm-objcopy's path is not valid UTF-8")?.to_string();
    let bin = dir.join(format!("{name}.bin"));
    let bin_path = bin.to_str().ok_or("the flat image path is not valid UTF-8")?.to_string();
    capture(&objcopy, &["-O", "binary", &elf_path, &bin_path])?;

    let bytes = std::fs::metadata(&bin)
        .map_err(|e| format!("could not measure the {package} image: {e}"))?
        .len();
    if bytes == 0 {
        return Err(format!("the {package} image is empty: the linker discarded everything"));
    }
    let most = image_max(name);
    if bytes > most {
        return Err(format!(
            "the {package} image is {bytes} bytes and the frame maps {most} for it.\n\n\
             A component that outgrows what its shape reserves needs a loader that reads\n\
             its headers, which is E5. Until then this is a real bound, and widening it\n\
             means widening `kernel::process`'s own reservation in the same diff: the two\n\
             numbers are one number, and `IMAGE_MAX` in xtask says which shape it belongs\n\
             to."
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

/// The machine every boot in this file is run on, unless it says otherwise.
///
/// Pinned rather than defaulted, for the reason `machine_with` states at the
/// `-m` argument: the kernel prints the loader's memory map, so the machine's
/// size is part of the boot log, and the boot log is the artefact
/// `cargo xtask trace` hashes.
const BOOT_MEMORY: &str = "128M";

/// A machine with a gibibyte in it, for the one check that needs one.
///
/// Four rather than one, because the kernel's largest order is a gibibyte and
/// a machine of exactly that size has none to spare: the loader's holes, the
/// kernel image and the first megabyte all come out of the one region, so the
/// gibibyte-aligned gibibyte never exists. Four leaves three whole ones.
///
/// Nothing hashes this boot's log — see `orders` — so the size may move when
/// the reason to move it appears.
const LARGE_MEMORY: &str = "4G";

/// The machine model, and the one piece of hardware it is pinned for.
///
/// Until E1-B01 there was no `-machine` here at all: the boot ran on whatever
/// QEMU defaults to, which is the 1996 desktop chipset, and the pin was
/// implicit. RFC 0031 makes it explicit and moves it, because the older
/// chipset has no PCI Express configuration space and no place to put a
/// remapping unit — so on it the kernel's IOMMU stage has nothing to find, and
/// a protection nothing exercises is a protection nobody has checked.
///
/// The interrupt controller is split because the remapping unit's interrupt
/// half requires it. This build does not enable interrupt remapping and does
/// not need it; the option is here because the device refuses to be created
/// without it, and pinning the machine means pinning that too.
///
/// `-net none` is not decoration. From the moment the kernel enables
/// translation, a device with no domain cannot address memory — so a network
/// card the emulator adds by default, that nothing in this kernel drives, would
/// be a bus master that faults the first time a packet arrives. Removing it is
/// the honest version of *this kernel drives no devices that do DMA*, and the
/// day one of them does is the day this option comes out and a domain goes in.
const MACHINE: &[&str] =
    &["-machine", "q35,kernel-irqchip=split", "-device", "intel-iommu,intremap=on", "-net", "none"];

/// The one device this tree drives that performs DMA, and the disk behind it.
///
/// Added only by `iommu`, because it exists to be provoked. Three of its
/// options are load-bearing and each would silently change what is being
/// measured:
///
/// - `disable-legacy=on` forces the modern register layout. A legacy virtio
///   device cannot negotiate the feature bit that routes its transfers through
///   the remapping unit, so it would bypass translation and the provocation
///   would pass while proving nothing — `kernel/src/arch/x86_64/dma.rs` records
///   what that cost to find out.
/// - `iommu_platform=on` is the device half of the same bit.
/// - `read-zeroes=on` makes the null block driver actually write to the
///   destination buffer. Without it the device completes a read and touches
///   nothing, which is indistinguishable from a transfer the unit refused —
///   which is precisely the distinction the whole check is about.
const DMA_DEVICE: &[&str] = &[
    "-drive",
    "if=none,id=blk0,driver=null-co,size=1048576,read-zeroes=on",
    "-device",
    "virtio-blk-pci,drive=blk0,disable-legacy=on,iommu_platform=on",
];

/// The network device `cargo xtask net` adds, and the backend behind it.
///
/// Added only by `net`, because the machine this tree boots has no network at
/// all — `MACHINE` passes `-net none`, and that option's comment says why: from
/// the moment the kernel enables translation, a device with no domain cannot
/// address memory, so a card the emulator adds by default would be a bus master
/// that faults the first time a packet arrives. *The day one of them does is the
/// day this option comes out and a domain goes in*, said that file, and this is
/// that day for one command: `net` adds a card and `kernel/src/net.rs` puts it
/// in a domain of its own before it is allowed to master the bus.
///
/// Two of the options are `DMA_DEVICE`'s and the reasoning is not repeated:
/// `disable-legacy=on` forces the modern register layout, without which the
/// device cannot negotiate the feature bit that routes its transfers through the
/// remapping unit, and `iommu_platform=on` is the device half of the same bit.
/// On a *network* device the consequence of getting that wrong is worse than a
/// green test: a legacy virtio-net device addresses physical memory by
/// specification and writes into it whenever a packet arrives, with no request
/// outstanding and nothing timing it.
///
/// The third is the backend. `user` is QEMU's own user-mode network stack: it
/// needs no privilege, no tap device and no host configuration, and it answers
/// address resolution for its gateway. That last property is the whole of what
/// this demonstration uses it for — it is a peer that replies, and nothing more.
/// **It is not a network**, and `kernel/src/net.rs` says at length what the
/// demonstration therefore does and does not show.
///
/// `restrict=on` is not decoration either. It confines the backend so that
/// nothing this boot sends can leave the host, which is what makes a check in
/// this suite a check rather than traffic: a test that could reach the outside
/// world is a test whose result depends on where it is run.
const NET_DEVICE: &[&str] = &[
    "-netdev",
    "user,id=net0,restrict=on",
    "-device",
    "virtio-net-pci,netdev=net0,disable-legacy=on,iommu_platform=on",
];

/// How large the disk `cargo xtask blk` gives the device is.
///
/// One mebibyte, which is two thousand and forty-eight sectors — far more than
/// the one the datapath writes, and small enough that creating it is not a
/// noticeable part of the command. It exists at all because the null block
/// driver `cargo xtask iommu` uses cannot hold anything: `read-zeroes=on` makes
/// it *write* to a destination buffer, which is what that check needs, and it
/// discards what is written to it, which is exactly what a write-then-read-back
/// check cannot use.
/// Unit: bytes.
const BLK_DISK_BYTES: usize = 1024 * 1024;

/// Make the disk the block datapath works on, fresh.
///
/// Rewritten on every run rather than created once, and that is what keeps the
/// boot a fixture: the kernel writes a sector and reads it back in the same
/// boot, so a file left over from a previous run would make the read succeed
/// for a reason this run did not establish — which is precisely the shape of
/// pass this whole family of commands exists to refuse.
fn blk_disk() -> Result<PathBuf, String> {
    let dir = target_dir().join("blk");
    std::fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", relative(&dir)))?;
    let path = dir.join("disk.img");
    std::fs::write(&path, vec![0u8; BLK_DISK_BYTES])
        .map_err(|e| format!("writing {}: {e}", relative(&path)))?;
    Ok(path)
}

/// The device and the disk behind it, for `cargo xtask blk`.
///
/// The two load-bearing options are `DMA_DEVICE`'s and the reasoning is not
/// repeated: `disable-legacy=on` and `iommu_platform=on` together are what put
/// the device's transfers through the remapping unit at all, and without them
/// every isolation result here would be a pass for the wrong reason. What
/// differs is the drive — a real file rather than a null device, because this
/// check writes a sector and reads it back, and a device that discards writes
/// would answer zeroes to both halves.
fn blk_device(disk: &Path) -> Result<Vec<String>, String> {
    let path = disk.to_str().ok_or("the disk image path is not valid UTF-8")?;
    Ok(vec![
        "-drive".to_string(),
        format!("if=none,id=blk0,file={path},format=raw,cache=unsafe"),
        "-device".to_string(),
        "virtio-blk-pci,drive=blk0,disable-legacy=on,iommu_platform=on".to_string(),
    ])
}

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
    machine_with(append, &[], Capture::Printed, seconds, BOOT_MEMORY)
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
    machine_with(append, features, capture, BOOT_TIMEOUT, BOOT_MEMORY)
}

/// [`machine`], capturing the log and printing none of it.
fn machine_quiet(append: Option<&str>) -> Result<(Ending, String), String> {
    machine_with(append, &[], Capture::Quiet, BOOT_TIMEOUT, BOOT_MEMORY)
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
    memory: &str,
) -> Result<(Ending, String), String> {
    machine_devices(append, features, capture, timeout, memory, &[])
}

/// The emulator, described once.
///
/// Extracted from [`machine_devices`] rather than copied into the one caller
/// that could not use it, and the reason is that function's own comment: *there
/// is one place the machine is described and a second one would drift from it
/// exactly as slowly as nobody notices.* `cargo xtask gpu` has to interleave
/// with a running boot — it reads the serial log as it arrives, captures the
/// emulator's framebuffer over a monitor socket when a line appears, and writes
/// a byte back — so it cannot use a function that spawns and waits. What it can
/// use is this.
///
/// Everything below this line is verbatim from where it used to live, including
/// the comments, so that the diff that moved it is a move.
///
/// # Errors
///
/// A build that failed, or an image or module path that is not valid UTF-8.
fn emulator(
    append: Option<&str>,
    features: &[&str],
    memory: &str,
    devices: &[&str],
) -> Result<Command, String> {
    build_with(features)?;
    let kernel = kernel_elf32();
    if !kernel.exists() {
        return Err(format!("kernel image not found at {}", kernel.display()));
    }
    let init = init_image()?;
    let components = components()?;

    let mut qemu = Command::new("qemu-system-x86_64");
    qemu.args(["-kernel", kernel.to_str().ok_or("kernel path is not valid UTF-8")?]);

    // The boot modules. Multiboot 1 calls them modules and QEMU's own loader
    // spells them `-initrd` with a comma-separated list; the kernel sees
    // validated extents and nothing about how they arrived.
    //
    // Module one is `user/init`'s flat image and its position is the contract:
    // it has no manifest, and `main::component` names it by index. Everything
    // after it is a component file — a record and an image — which the frame
    // finds by magic rather than by position, so a loader that reordered them
    // would produce a smaller topology rather than a component built out of the
    // wrong bytes. RFC 0030.
    //
    // Passed on every boot, including the ones that provoke something. The
    // provocations run the kernel's own adversary, which is a different program
    // — see `kernel::process::Plan` — and the component runs first regardless,
    // because "a second process cannot use the first one's handles" is a
    // property every boot should be checking rather than a special run.
    let mut modules = vec![init.to_str().ok_or("the init image path is not valid UTF-8")?];
    for path in &components {
        modules.push(path.to_str().ok_or("a component file path is not valid UTF-8")?);
    }
    qemu.args(["-initrd", &modules.join(",")]);

    if let Some(append) = append {
        qemu.args(["-append", append]);
    }

    // Named by the caller, not defaulted. The kernel prints the loader's memory
    // map, so the machine's size is part of its output — and an emulator
    // default that moves between versions would move the boot log with it,
    // quietly breaking the one M0 contract that matters: the same commit
    // produces the same run, byte for byte. Every caller but one passes
    // `BOOT_MEMORY`, and the one that does not is `orders`, whose log nothing
    // hashes and which exists because the fixture is too small to hold the
    // largest thing the allocator can hand out.
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
        memory,
        "-serial",
        "stdio",
        "-display",
        "none",
        "-device",
        "isa-debug-exit,iobase=0xf4,iosize=0x04",
        "-no-reboot",
    ]);

    // The chipset and the remapping unit, pinned for the same reason the memory
    // size is and argued at [`MACHINE`]. RFC 0031.
    qemu.args(MACHINE);
    // Whatever this one command needs and no other does.
    qemu.args(devices);

    qemu.current_dir(root());
    Ok(qemu)
}

/// [`machine_with`], plus devices only one command wants.
///
/// A parameter rather than a second description of the emulator, because there
/// is one place the machine is described and a second one would drift from it
/// exactly as slowly as nobody notices. Every caller but `iommu` passes an
/// empty slice, and that slice is the whole of the difference between the boot
/// that is a fixture and the boot that has something to provoke.
fn machine_devices(
    append: Option<&str>,
    features: &[&str],
    capture: Capture,
    timeout: u64,
    memory: &str,
    devices: &[&str],
) -> Result<(Ending, String), String> {
    let mut qemu = emulator(append, features, memory, devices)?;

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

/// The largest block the allocator can hand out, on a machine that has one.
///
/// # Why this is a second boot rather than a bigger `run`
///
/// The fixture is 128 MiB and stays 128 MiB: the kernel prints the loader's
/// memory map, `cargo xtask trace` hashes the boot log, and a machine size
/// that moved would move that hash for a reason that is not the kernel. But
/// the largest block a 128 MiB machine can give away is order 13 — order 14 is
/// 64 MiB and the one usable region ends before a 64 MiB-aligned one fits —
/// while the largest order the allocator's own type admits, and the one
/// E1-B12's exit criterion names, is 18: a gibibyte.
///
/// So the fixture cannot reach the top of the structure it is testing, and the
/// paths only a large machine takes — `Order::up`'s bound, the top of the
/// coalescing sweep, `refill`'s branch above the default grain — could
/// regress with every other check in this file still green. A number
/// reproduced by hand in a report and by nothing in the tree is the shape
/// `claims/README.md` calls an anecdote.
///
/// This boots the same image on a machine that has the memory, and reads back
/// the number the kernel reports. Nothing hashes *this* log, which is exactly
/// why it is a separate command: the boot that has to be reproducible is
/// small, and the boot that has to be large is not asked to be reproducible.
///
/// It asserts the top where `mem::self_test` reports it. The kernel reports
/// because it does not know what machine it is on; this asserts because it
/// chose the machine.
fn orders() -> Result<(), String> {
    /// `kernel::mem::Order::MAX`. Written down here rather than read out of
    /// the log, because a check that took the kernel's own answer for what the
    /// kernel should answer would check nothing.
    const LARGEST: u8 = 18;
    /// The prefix `mem::self_test`'s line puts that number after.
    const MARK: &str = "orders 0..=";

    let (ending, log) = machine_with(None, &[], Capture::Printed, BOOT_TIMEOUT, LARGE_MEMORY)?;
    match ending {
        Ending::TimedOut(seconds) => {
            return Err(format!(
                "the boot was still running after {seconds}s and was killed\n\n\
                 The log up to that point is above."
            ));
        }
        Ending::Exited(33) => {}
        Ending::Exited(other) => {
            return Err(format!("qemu exited {other}; expected 33 — see the log above"));
        }
        Ending::Signalled => return Err("qemu terminated by signal".into()),
    }

    let after = log.split(MARK).nth(1).ok_or_else(|| {
        format!(
            "no line in the boot log said `{MARK}N`.\n\n\
             That line is how `kernel::mem::self_test` reports the largest order it\n\
             could serve. If it has been renamed, this check has been reading nothing\n\
             — re-point it rather than delete it."
        )
    })?;
    let digits: String = after.chars().take_while(char::is_ascii_digit).collect();
    let reported: u8 = digits
        .parse()
        .map_err(|_| format!("`{MARK}{digits}` is not a number this check can read"))?;

    if reported != LARGEST {
        return Err(format!(
            "the allocator served order {reported} on a {LARGE_MEMORY} machine; \
             expected {LARGEST}\n\n\
             Order {LARGEST} is `Order::MAX` and this machine has the memory for one, so\n\
             a smaller answer is the allocator refusing an order it admits: the split\n\
             path above the default grain, `Order::up`'s bound, or the top of the\n\
             coalescing sweep. `cargo xtask run` cannot see any of it — 128 MiB stops\n\
             at order 13."
        ));
    }

    println!("\norders: order {LARGEST} served on a {LARGE_MEMORY} machine, and given back");
    Ok(())
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
const DEFECTS: &[&str] = &[
    TRACE_DEFECT,
    "mutate-unchecked-index",
    "mutate-relaxed-submission",
    "mutate-relaxed-completion",
    "mutate-no-doorbell-fence",
    // The simulator's, and the first defect in this list that is not the
    // kernel's. `cargo xtask sweep --mutate` is its harness and RFC 0040 is
    // where the extension of RFC 0017's argument to this layer is written down.
    "mutate-crossed-completion",
    // The second, and it is here because five oracle properties with one defect
    // between them is one property under test and four decorations. This one
    // trips a different check — RFC 0042.
    "mutate-silent-reset",
    // The ring's three, one per property `E1-P04`'s exit names, and they are
    // three for RFC 0042's arithmetic rather than for thoroughness: a
    // hostile-peer fuzzer with one defect behind it demonstrates that one of
    // *no panic, no memory unsafety, no hang* can fail and says nothing about
    // the other two. RFC 0046.
    "mutate-believed-header",
    "mutate-trusted-slot",
    "mutate-unbounded-drain",
    // And E1-P05's three, one per oracle the entry fuzzer has, by the same
    // arithmetic one task later: the envelope, the ledger and the reach. RFC
    // 0048, and `cargo xtask entries --mutate` requires each to be found by its
    // own oracle and by no other.
    "mutate-ignored-flag",
    "mutate-reusable-slot",
    "mutate-lenient-index",
];

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
    let (ending, log) = machine_with(None, features, Capture::Quiet, BOOT_TIMEOUT, BOOT_MEMORY)?;
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
fn trace_check() -> Result<(), String> {
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
        "\ntrace: ok — {first:#018x}\n\
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
fn trace_hash_only() -> Result<(), String> {
    println!("{:#018x}", trace(&[])?);
    Ok(())
}

/// The scenario `sim --hash` runs when none is named.
///
/// `contention` rather than whichever is first in the table, because it is the
/// one whose refusal path runs. A default that exercised the quiet scenario
/// would give CI a hash that could stop moving without anything saying so.
const SIM_SCENARIO: &str = "contention";

/// The second seed the simulator check compares against.
///
/// It is the negative control, and the simulator's version of the argument
/// `trace_check` makes with a deliberate defect: two runs at one seed agreeing
/// proves nothing on its own, because a digest over something that does not vary
/// agrees with itself forever. A boot needs a broken build to demonstrate that,
/// because a boot has no seed on its command line yet. A simulated run does have
/// one, so its negative control is a second seed rather than a second build —
/// cheaper, and it tests the same property.
const SIM_OTHER_SEED: &str = "0xa5a5a5a5a5a5a5a5";

/// One simulated run, as a subprocess, answering its trace hash.
///
/// A subprocess and not a library call, for the reason `sim/src/main.rs` states:
/// the claim is that two runs sharing nothing but the commit produce one
/// artefact, and two calls inside one process share an address space, an
/// allocator and whatever a library left behind. `f-sim`'s own tests make the
/// in-process claim, which is cheaper and weaker; this is the one that matches
/// what `trace` does with two boots.
fn sim(scenario: &str, seed: &str) -> Result<u64, String> {
    // The component directory is passed on every run rather than only for the
    // scenario that reads it, because this file is what knows about
    // `CARGO_TARGET_DIR` and the simulator should not have to guess where a
    // build put anything.
    let dir = component_dir()?;
    let out = capture(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "f-sim",
            "--",
            "--hash",
            "--seed",
            seed,
            "--components",
            &dir,
            scenario,
        ],
    )?;
    let line = out.trim();
    let unhash = || format!("f-sim printed `{line}`, which is not a hash");
    let digits = line.strip_prefix("0x").ok_or_else(unhash)?;
    u64::from_str_radix(digits, 16).map_err(|_| unhash())
}

/// One simulated run, as a subprocess, answering its whole artefact.
///
/// The same run [`sim`] takes a digest of, printed rather than hashed. It exists
/// because the exit criterion's word is *byte-identically* and a digest is not
/// bytes: `trace::digest` says of itself that it "does not have to be
/// collision-resistant, because nothing adversarial produces these traces",
/// which is the right argument for the boot's log hash and does not make a
/// 64-bit digest into a byte comparison. The boot has no alternative; the
/// simulator does, and this is it.
fn sim_trace(scenario: &str, seed: &str) -> Result<String, String> {
    let dir = component_dir()?;
    capture(
        "cargo",
        &[
            "run",
            "-q",
            "-p",
            "f-sim",
            "--",
            "--trace",
            "--seed",
            seed,
            "--components",
            &dir,
            scenario,
        ],
    )
}

/// Where `cargo xtask component` leaves the component files.
///
/// One directory, named here rather than in the simulator, because this file is
/// what knows about `CARGO_TARGET_DIR` and a second answer to *where is the
/// build output* is a second answer that stops matching.
fn component_dir() -> Result<String, String> {
    let dir = target_dir().join("component");
    dir.to_str()
        .map(str::to_string)
        .ok_or_else(|| format!("the component directory {} is not valid UTF-8", dir.display()))
}

/// Build every component file, saying nothing about it.
///
/// [`components`] prints a line per component, which is right for
/// `cargo xtask component` and wrong immediately before a hash a CI job pipes
/// into a file. Same builder, same refusals, no report.
fn components_quietly() -> Result<(), String> {
    for name in COMPONENTS {
        component_image(name)?;
    }
    Ok(())
}

/// The scenario set, read from the simulator rather than restated here.
///
/// A second copy of the list in this file is a second copy that stops matching
/// the day somebody adds a scenario, and it would stop matching silently —
/// which is exactly how `f-bench` and `f-init` came to have tests nothing ran.
fn sim_scenarios() -> Result<Vec<String>, String> {
    let out = capture("cargo", &["run", "-q", "-p", "f-sim", "--", "--list"])?;
    let names: Vec<String> =
        out.lines().filter_map(|line| line.split_whitespace().next()).map(str::to_string).collect();
    if names.is_empty() {
        return Err("f-sim listed no scenarios, so there is nothing to reproduce".into());
    }
    Ok(names)
}

/// What `cargo xtask chaos` kills that a boot cannot, declared as a set rather
/// than left as a silence.
///
/// # Why a declaration and not a paragraph
///
/// RFC 0036 is the precedent and the argument is the same one: the join between
/// two halves of a claim has a difference, and a difference that is prose is a
/// difference nobody re-checks. So the gap is data, each entry naming the file
/// and the exact text whose *presence* is what keeps the gap open, and this verb
/// requires every one of them to still be there. The day one goes, this check
/// goes red and tells whoever closed it to update the RFC — which is the
/// opposite of the usual failure, where a gap quietly stops being true and the
/// document keeps describing it.
///
/// One entry, and it is **narrower than it was**, which is the mechanism
/// working rather than the gap closing on its own.
///
/// It used to be RFC 0033's reversal condition — *grep for `Driver::execute`
/// and see which crate calls it* — and while the frame called it, the code the
/// datapath ran on was not a scheduled component at all. RFC 0047 ended that:
/// the driver serves its client from ring 3, on a core of its own, out of its
/// own polling loop, and `cargo xtask lint-datapath` now refuses a frame that
/// names the type. What that closed is *the datapath is served by a scheduled
/// component*.
///
/// What it did not close is the sentence beside it, and the two are easy to
/// read as one. `kernel/src/component.rs` builds a **place** for this manifest
/// on every boot — an account, needs checked handle by handle, an endpoint
/// clients hold, a restart policy — and never hands its occupant a core;
/// `kernel/src/blk.rs` hands a core to an instance that is in no place. So the
/// occupant a boot can kill is still not the occupant that serves a client's
/// load, and *under sustained load* is still a sentence only the simulator
/// makes true. The needle is the call that stands a driver up outside a place,
/// and it goes when a supervisor spawns and schedules in one act — which is
/// E1-B05's remaining half and RFC 0008's *restart is the supervisor's*.
/// RFC 0041 states the shape of the gap; RFC 0047 states what is left of it.
const CHAOS_GAP: &[Gap] = &[(
    "kernel/src/blk.rs",
    "prepare_driver(",
    "the driver is scheduled outside the place its manifest is spawned into, so \
     the occupant a boot can kill is not the occupant serving the datapath",
    "TODO.md E1-B02, E1-B08 and E1-P06; docs/rfc/0041's gap section and its closing \
     condition; docs/rfc/0047's *Foreclosed*; sim/src/chaos.rs's module comment; \
     claims/0006-driver-restart-latency.toml's [workload] and [hardware] notes",
)];

/// Why release 0.2 carries none of the four datapath numbers, as data.
///
/// # Why this is a declared quantity and not the paragraph it replaces
///
/// `RELEASING.md` names four numbers `E1-P10` would register — ring submit under
/// load, doorbells per operation, copies per operation, kernel entries per
/// operation — and says the release does not contain them. An absence stated in
/// prose has the failure mode this tree has already been bitten by twice: the
/// reason stops being true, and the paragraph goes on being read. Here the
/// paragraph is load-bearing in the worst way, because it is the one a reader
/// checks the honesty of a release against.
///
/// So the two reasons are needles, and the build goes red the day either one is
/// gone. Two rows and either alone is sufficient, which is why they are separate
/// rows rather than one:
///
/// - **There is no workload.** `Bell::new` refuses [`Path::UserInterrupt`] on a
///   build whose hardware does not report the feature, and that is every machine
///   this project can reach — TCG implements no part of UINTR and no `-cpu`
///   model advertises the bit. `E1-B09` needs that path to execute; `E1-P10`
///   needs `E1-B09`.
/// - **There is no machine.** All four are times, and `f_bench::Environment`
///   refuses to record one where `F_ENVIRONMENT=container`. `E0-D10` owns
///   obtaining a machine that may.
///
/// Neither substitutes for the other, and the row that goes first says which
/// half of the section in `RELEASING.md` has stopped being true. The day both
/// go, that section is deleted rather than amended — RFC 0056 says so, and this
/// is what makes the build say it out loud instead of waiting for somebody to
/// re-read a document. `E1-R02`.
const DATAPATH_GAP: &[Gap] = &[
    (
        "ring/src/doorbell.rs",
        "!hardware.user_interrupts",
        "the user-interrupt doorbell refuses to construct on every machine this project can \
         reach, so E1-B09 has no path that executes and E1-P10 has no workload",
        "TODO.md E0-B15, E1-B09 and E1-P10; RELEASING.md's *Release 0.2, and the four numbers \
         that are not in it*; docs/rfc/0056; docs/TESTING-STATUS.md's user-interrupt bullet",
    ),
    (
        "bench/src/lib.rs",
        "WHY_CONTAINER",
        "no machine this project has may record a timing, so all four datapath numbers are \
         pending on E0-D10 rather than on four different things",
        "TODO.md E0-D10; RELEASING.md's *Release 0.2, and the four numbers that are not in it*; \
         docs/rfc/0056; claims/runner-class-A.md; every claim whose status is `pending`",
    ),
];

/// Kill every component under load, twice over, and judge the pair.
///
/// # What this command is, in one paragraph
///
/// Every component the build produced is put in a place, driven by a client that
/// keeps work in flight, and killed at seeded moments; then the same workload is
/// run against the same component with nothing killed. The simulator holds the
/// verdict — `sim/src/chaos.rs`, and the exit status is it — and this holds the
/// three things a verdict cannot: that the run reproduces across two processes,
/// that it moves when the seed moves, and that the gap between what it kills and
/// what a boot kills is still the gap that was declared.
///
/// # Errors
///
/// The verdict, the reproduction check, or the declared gap having closed.
/// What `E1-B14` measured and did not fix.
///
/// The task's subject was the unmap, and the unmap is batched: one global
/// invalidation per request instead of one per page, which is 87.5% of the
/// remapping unit's round trips gone at the eight-page set the workload cycles.
/// The same measurement found the same cost on the other side of the same
/// cycle and left it standing. A registration maps its pages one at a time and
/// `vtd::Unit::map` invalidates after each, so registering a set costs exactly
/// what retiring one used to.
///
/// It is declared rather than mentioned because a measurement that reported
/// only the half it improved is R12's concession hidden in a metric. The number
/// is in `claims/0014` — `map_invalidations_per_page`, bounded at one in both
/// directions — and `kernel/src/main.rs`'s churn verdict fails the boot if it
/// moves, so this constant and that threshold go red together.
///
/// **Why it was not simply done here.** A map that fails part way must undo
/// what it made, or a device is left with a translation for the first half of a
/// buffer its driver was told it does not have at all — `iommu::Grant::map`
/// says so and has the undo loop. Batching the invalidation across that undo is
/// a different argument from batching it across an unmap, and it is a change to
/// the mapping path of two live datapaths. Doing it on the strength of a
/// measurement taken for something else is how a task acquires a second one.
/// RFC 0052's *what would reverse this* names the owner.
const CHURN_GAP: &[Gap] = &[(
    "kernel/src/iommu.rs",
    "self.unit.map(self.frames, self.domain, at, at, writable)",
    "a registration invalidates the remapping unit once per page, which is what an \
     unmap did until E1-B14 batched it",
    "docs/rfc/0052's Consequences and its reversal section; claims/0014-unmap-churn.toml's \
     map_invalidations_per_page threshold and the note beside it; kernel/src/churn.rs's \
     module comment; kernel/src/arch/x86_64/vtd.rs's Unit::pages_mapped",
)];

/// What the churn observes about a *device*, which is nothing.
///
/// # Why this is declared rather than argued
///
/// `E1-B14` batched an unmap: N entries cleared, one global invalidation
/// published for the lot. `kernel/src/churn.rs` now reads the unit's own
/// second-level tables back after every retirement and requires the set's pages
/// to be gone — so *the entries are cleared* is observed rather than counted,
/// and the batch's distinguishing behaviour is watched by something that can
/// fail. What no boot in this tree watches is the other half of a revocation: a
/// **device** attempting the transfer afterwards and faulting. `cargo xtask
/// blk`'s `outside` half does exactly that and does it over a **one-page**
/// registration, where `Invalidation::PerRequest` and `PerPage` are the same
/// run and the batch is not exercised at all.
///
/// So the residual is precise: nothing observes a device failing to reach a
/// buffer set that a *multi-page batched* unmap took away. It is small — the
/// invalidation is global, so one at the end throws away every entry the loop
/// cleared exactly as well as one after each, which is `Unit::unmap_range`'s
/// argument — and *small* is why it is a declared quantity rather than a task
/// nobody schedules. What closes it is either a multi-page registration in the
/// boot that already watches a fault, which is `kernel/src/blk.rs`'s geometry
/// and E1-B06's file, or a device attached to the churn's own domain, which is
/// what the needle below names: the day that SAFETY comment stops being true,
/// this goes red and the documents that describe the gap are printed.
const REVOKE_GAP: &[Gap] = &[(
    "kernel/src/churn.rs",
    "nothing was ever attached to this domain",
    "no boot observes a device faulting after a *batched multi-page* unmap; the churn reads \
     the unit's tables, and the boot that watches a device fault registers one page",
    "docs/test-taxonomy.md's `Mapping left after revoke, under churn` row and \
     docs/test-taxonomy.toml's row of the same name; docs/rfc/0052's *What would reverse \
     this*; claims/0014-unmap-churn.toml's standing_after_unmap threshold and the note \
     beside it; kernel/src/churn.rs's module comment",
)];

/// E1-B14: what an unmap costs under churn, and what a batch buys.
///
/// # Why one command runs two things
///
/// The exit names three quantities — shootdowns, interrupts and a p99 unmap
/// cost — and this machine may *publish* two of them and not the third. A count
/// is the same number in a container and on bare metal, so the boot takes those
/// and `claims/0014` gates on them. A p99 in nanoseconds is not, and
/// `bench/src/lib.rs` refuses to record one here.
///
/// Refusing to publish is not refusing to measure, and the two are separated so
/// that the instrument is checkable where it runs. The boot times a thousand
/// and twenty-four unmap requests through the shipped path on this machine's
/// real remapping unit on every run, and fails on a short sample, a maximum of
/// zero ticks, or a histogram whose own arithmetic does not hold — all counts.
/// What it does with the percentiles is decided here, by
/// `f_bench::Environment::detect`, and passed to the kernel as a word on the
/// command line: one rule about what may be quoted, in the place that already
/// owns it. `claims/0015` is that number and it is `pending` on `E0-D10`'s
/// machine, for the reason its `[hardware]` note gives — this emulator answers
/// an invalidation instantly and in software.
///
/// The host workload runs too and is the smaller half by construction: there is
/// no unit on the host, so what it times is the registry arithmetic above the
/// hardware. Both are required to report the same counts.
fn churn() -> Result<(), String> {
    println!("--- the frame: what the remapping unit was made to do, counted both ways");
    // Whether the boot may *publish* the time it takes is `f_bench`'s decision
    // and not a second one made in the kernel. The boot records the
    // distribution either way — an instrument that only runs on a machine
    // nobody has yet is an instrument nobody has checked — and prints
    // percentiles only when this parameter says the machine is one a number may
    // be quoted from. `E0-P15`'s rule, carried across the privilege boundary as
    // a word on a command line.
    let environment = f_bench::Environment::detect();
    let append = if environment.records() { "churn=unmap measure" } else { "churn=unmap" };
    let (ending, log) = machine(Some(append), &[], Capture::Printed)?;
    match ending {
        Ending::Exited(33) => {}
        Ending::Exited(35) => {
            return Err("the kernel refused to finish the unmap churn. Either the two \
                        halves did not do the same work, or an invalidation policy \
                        stopped meaning what it says, or the churn reached a shootdown. \
                        The serial log above says which, and every one of those is a \
                        finding rather than a flake."
                .into());
        }
        other => return Err(format!("the boot {other}; expected exit 33")),
    }

    // The verdict is the kernel's, as it is for `cap`, `iommu`, `blk` and
    // `runtime`. This checks the stage ran at all, which an exit code cannot
    // say: a build where the parameter had been renamed would boot cleanly and
    // measure nothing.
    if !log.contains("churn verdict") {
        return Err("`churn=unmap` finished without reaching a verdict, so the stage did \
                    not run: no remapping unit was found, or the parameter is no longer \
                    the one `churn_measurement` reads."
            .into());
    }

    // The three lines that are an observation rather than a count, each
    // required by name. A boot that stopped taking one of them would still exit
    // 33 and still print a saving, which is the shape of green-while-false this
    // epoch keeps finding: the stage would be measuring the frame's bookkeeping
    // and nothing would say the tables were ever read back.
    for (needle, what) in [
        ("churn revoke", "the walk that reads the unit's tables back after the unmap"),
        ("churn frames", "the free count either side of the churn"),
        ("churn hole", "the batched unmap of a set with a page taken out from under it"),
        ("churn cost", "the timed pass, which is `claims/0015`'s workload"),
    ] {
        if !log.contains(needle) {
            return Err(format!(
                "`churn=unmap` printed no `{needle}` line, so {what} did not run. It is \
                 not optional: a churn that counts what it did and never looks at what it \
                 left is the measurement this task shipped a first draft of."
            ));
        }
    }

    // And the refusal itself, where the refusal is what should have happened.
    // `f_bench` declining to publish is a rule that is worth exactly as much as
    // the check that it fired — a kernel that printed a percentile in a
    // container would otherwise be caught by nobody, because the number would
    // look perfectly reasonable.
    if !environment.records() && !log.contains("latency refused") {
        return Err(format!(
            "this is `{}`, which is not a measurement environment, and the boot published a \
             latency anyway. `churn_cost` in kernel/src/main.rs gates on the `measure` \
             parameter and this run did not pass it — so either the gate is gone or the \
             parameter arrived from somewhere else.",
            environment.name()
        ));
    }

    // The half this task measured and did not fix, checked rather than
    // remembered. It goes red the day the mapping path stops being one
    // invalidation per page, which is the day `claims/0014`'s threshold and
    // RFC 0052's Consequences stop being true together.
    gap_holds("CHURN_GAP", CHURN_GAP)?;
    // And the half it observes and does not: the tables are read back, a device
    // is not. See `REVOKE_GAP`.
    gap_holds("REVOKE_GAP", REVOKE_GAP)?;

    println!();
    println!("--- the clock: the same churn on the host, where a timing may not be recorded");
    let host = capture("cargo", &["run", "--release", "-p", "f-bench", "--bin", "unmap_churn"])?;
    print!("{host}");

    // The two sides, required to agree on the half they can both see.
    //
    // This is the check this epoch keeps finding it needed. A boot that
    // measured one geometry and a workload that measured another would be two
    // experiments sharing one claim, and every number in `claims/0014` would be
    // about whichever of them a reader happened to run. So the counts are read
    // out of both logs and compared — which also catches the quieter failure,
    // a workload whose constants drifted from `kernel/src/churn.rs`'s while
    // both halves stayed internally consistent.
    let frame = churn_counts(&log, "churn perpage")
        .ok_or("the boot printed no `churn perpage` line to read a count from")?;
    let modelled =
        churn_counts(&host, "counts  ").ok_or("the host workload printed no `counts` line")?;
    if frame != modelled {
        return Err(format!(
            "the frame and the host workload measured different churn: the boot made \
             {} unmap request(s) over {} page(s) and the workload {} over {}.\n\n\
             They are one experiment and have to be the same one. The geometry is \
             declared in both kernel/src/churn.rs and bench/src/bin/unmap_churn.rs, \
             and the two lists have drifted.",
            frame.0, frame.1, modelled.0, modelled.1
        ));
    }

    // The observation count out of the boot's own line rather than a constant
    // here, for the same reason the counts are compared at all: a number this
    // command states about a boot has to have come from that boot.
    let timed = log
        .lines()
        .find(|line| line.contains("churn cost") && line.contains("timed unmap request"))
        .and_then(|line| line.split_whitespace().find_map(|word| word.parse::<u64>().ok()))
        .ok_or("the boot printed no timed observation count to read")?;

    println!();
    println!(
        "churn: both sides agree — {} unmap request(s) over {} page(s) per round. The frame\n\
         counted what it was made to do, read the unit's tables back to check it, and timed\n\
         {} unmap request(s) through the shipped path. A percentile of those is published\n\
         only on a machine a number may be quoted from, and this one is `{}`. The counts\n\
         are `claims/0014`, which gates; the time is `claims/0015`, `pending` on E0-D10's.",
        frame.0,
        frame.1,
        timed,
        environment.name()
    );
    Ok(())
}

/// The `(requests, pages)` pair out of a line that names both.
///
/// Textual, because the two producers are a kernel writing to a serial port and
/// a host binary writing to a pipe, and the only thing they share is the
/// English they print. Narrow on purpose: it takes the first two numbers on the
/// first line containing `marker`, so a line that stopped printing one of them
/// yields `None` and the caller fails rather than comparing against a default.
fn churn_counts(log: &str, marker: &str) -> Option<(u64, u64)> {
    let line = log.lines().find(|line| line.contains(marker))?;
    let mut numbers = line
        .split(|c: char| !c.is_ascii_digit())
        .filter(|word| !word.is_empty())
        .filter_map(|word| word.parse::<u64>().ok());
    Some((numbers.next()?, numbers.next()?))
}

fn chaos() -> Result<(), String> {
    components_quietly()?;
    let dir = component_dir()?;

    // The reproduction check first, and in two processes, for the reason
    // `sim/src/main.rs` opens with: a harness called twice inside one process
    // shares an address space and an allocator and can agree with itself for
    // reasons that have nothing to do with the seed. A chaos test that cannot be
    // replayed reports a symptom rather than a bug, which is the exact thing
    // gate G1 exists to stop.
    println!("chaos reproduction check — seed {TRACE_SEED}\n");
    let first = chaos_hash(TRACE_SEED, &dir)?;
    let second = chaos_hash(TRACE_SEED, &dir)?;
    let other = chaos_hash(SIM_OTHER_SEED, &dir)?;
    println!("  {:<12} {first}  {second}  {other}", "sweep");
    if first != second {
        return Err("two runs of the chaos sweep at one seed produced different results.\n\n\
             A kill at a seeded moment has to be at *the* seeded moment. Something in the\n\
             harness is reading a clock, an address or an iteration order the seed does\n\
             not own, and a failure it finds is a symptom rather than a bug report.\n\
             RFC 0004, RFC 0041."
            .into());
    }
    if first == other {
        return Err("the chaos sweep produced the same result at two different seeds.\n\n\
             That makes the check above worth nothing: a digest over something that does\n\
             not vary agrees with itself forever. Either the kills are landing at the same\n\
             moment whatever the seed says, or the digest is taken over less than the run."
            .into());
    }

    // Then the run itself, printed. Its exit status is the verdict, and a
    // failure prints the report above the reason rather than instead of it —
    // which is the difference between a gate somebody can act on and one they
    // have to reproduce first.
    println!();
    let (ok, report) = chaos_report(TRACE_SEED, &dir)?;
    print!("{report}");
    if !ok {
        return Err("a client observed something other than added latency.\n\n\
             Gate G1: *a driver is killed under sustained load and the system does not\n\
             notice*. The report above says which of the three halves of that sentence\n\
             failed — an operation lost, an operation answered twice, or an answer that\n\
             disagreed with what was written — and at which component."
            .into());
    }

    // And the coverage, against a set this command did not produce.
    //
    // The first version of this check compared the number the sweep ran with the
    // number the deployment directory held — two reads of one directory, so it
    // could not fail, and a component dropped from `COMPONENTS` would have taken
    // both sides down together and printed a green `coverage 1 of 1` over half
    // the tree. So the number on the other side of the comparison is now the set
    // of `manifest.toml` files the *source tree* carries, which is the one thing
    // about a component that cannot be hand-maintained: a directory with a
    // manifest is a component, and `lint-components` separately requires the
    // build list to be that same set.
    let ran = report
        .lines()
        .find_map(|line| line.trim().strip_prefix("components "))
        .and_then(|rest| rest.trim().parse::<usize>().ok())
        .ok_or("the chaos report did not say how many components it ran")?;
    let declared = declared_components()?;
    let built: Vec<String> = capture(
        "cargo",
        &["run", "-q", "-p", "f-sim", "--", "--deployment", "--components", &dir],
    )?
    .lines()
    .filter_map(|line| line.split_whitespace().next().map(str::to_string))
    .collect();
    if ran != declared.len() || built.len() != declared.len() {
        let missing: Vec<&str> = declared
            .iter()
            .map(String::as_str)
            .filter(|name| !built.iter().any(|had| had == name))
            .collect();
        return Err(format!(
            "the sweep killed {ran} component(s), the build produced {}, and this tree\n\
             declares {} in its manifests{}.\n\n\
             *Each driver component in turn* is the exit criterion's own words, so a\n\
             component the sweep did not reach is a component nobody has killed — and a\n\
             green result over a smaller set is the failure this check exists to refuse.\n\
             `cargo xtask lint-components` says which list is short.",
            built.len(),
            declared.len(),
            if missing.is_empty() {
                String::new()
            } else {
                format!(" — not built: {}", missing.join(", "))
            }
        ));
    }
    println!(
        "\ncoverage      {ran} component(s) killed, of {} this tree's manifests declare",
        declared.len()
    );

    println!("\ndeclared gap  what this kills that a boot cannot, and why it is still true:");
    // The same reading `lint-owed` performs, from the same helper, with
    // this verb's own guidance appended: the two gaps are one discipline used
    // twice and a second copy of the loop would be a second place for it to rot,
    // but what a reader should *do* about each of them is different.
    gap_holds("CHAOS_GAP", CHAOS_GAP).map_err(|why| {
        format!(
            "{why}\n\n\
             The reason `cargo xtask chaos` is the only half of E1-P06 that can kill a\n\
             component under load has stopped being true, so RFC 0041's gap section and\n\
             RFC 0047's now describe a tree that no longer exists. Move the kill into the\n\
             boot."
        )
    })?;
    for (file, _, why, _) in CHAOS_GAP {
        println!("  {file:<24} {why}");
    }

    println!(
        "\nchaos: ok — every component the build produced was killed under load and refilled\n\
         \x20      under its own declared policy, and no client observed anything except a wait.\n\
         \x20      The control run beside each of them completed with nothing killed, which is\n\
         \x20      what makes the survival evidence rather than an absence of trouble."
    );
    Ok(())
}

/// One chaos sweep, as a subprocess, reduced to its digest.
fn chaos_hash(seed: &str, dir: &str) -> Result<String, String> {
    let out = capture(
        "cargo",
        &["run", "-q", "-p", "f-sim", "--", "--chaos-hash", "--seed", seed, "--components", dir],
    )?;
    Ok(out.trim().to_string())
}

/// A reservation refused, a reservation granted, and a boot that asks this
/// machine which it can do.
///
/// **`E1-B07`.** Three things happen and none of them means much alone, which is
/// why they are one command:
///
/// 1. **The reproduction check**, in two processes, for `sim/src/main.rs`'s
///    reason: a model called twice inside one process shares an allocator and
///    can agree with itself for reasons the seed does not own. Two seeds, so a
///    digest over something that does not vary cannot pass it.
/// 2. **The model**, printed with its three arms. The exit status is
///    `f_sim::reserve::verdict`'s, and two of the three arms are controls: the
///    unreserved arm must miss and the over-subscribed one must be refused
///    without running, or the granted arm's zero is about the workload.
/// 3. **The boot**, which asks *this* machine what it can reserve and requires
///    the arithmetic to be able to say both yes and no. On QEMU the answer for
///    this machine is no — no thread level, no cache topology, no RDT — and the
///    described half beside it is what says that is a refusal rather than a
///    function that only refuses.
///
/// # Errors
///
/// A sentence naming which of the three did not hold.
fn admission_gate() -> Result<(), String> {
    println!("admission reproduction check — seed {TRACE_SEED}\n");
    let first = admission_hash(TRACE_SEED)?;
    let second = admission_hash(TRACE_SEED)?;
    let other = admission_hash(SIM_OTHER_SEED)?;
    println!("  {:<12} {first}  {second}  {other}", "model");
    if first != second {
        return Err("two runs of the reservation model at one seed produced different \n\
             results. The adversary's stretches are drawn from `f_env::Env` and nothing \n\
             else, so something in the model is reading a clock, an address or an \n\
             iteration order the seed does not own — and a miss it finds would be a \n\
             symptom rather than a bug report. RFC 0004."
            .into());
    }
    if first == other {
        return Err("the reservation model produced the same result at two different \n\
             seeds, which makes the check above worth nothing: a digest over something \n\
             that does not vary agrees with itself forever. Either the adversary is not \n\
             drawing from the seed, or the digest is taken over less than the run."
            .into());
    }

    println!();
    let (ok, report) = admission_report(TRACE_SEED)?;
    print!("{report}");
    if !ok {
        return Err("the reservation model's verdict went red.\n\n\
             E1-B07: *an over-subscribed reservation is refused with ADMISSION; a granted\n\
             one meets its deadline under adversarial load.* The report above says which\n\
             arm failed. Read the two controls first: an unreserved arm that stopped\n\
             missing means the load has gone soft, and a granted arm that misses means\n\
             admission control granted something the machine could not keep."
            .into());
    }

    // And the frame's own half, on the machine this tree can actually boot.
    println!();
    let (ending, log) =
        machine_with(Some("admission"), &[], Capture::Printed, BOOT_TIMEOUT, BOOT_MEMORY)?;
    match ending {
        Ending::Exited(33) => {}
        Ending::Exited(35) => {
            return Err("the kernel refused to finish the admission stage. Either the \n\
                 arithmetic granted an over-subscribed reservation, or it refused one the \n\
                 described part can hold — which would mean it refuses everything and the \n\
                 refusal on this machine says nothing. The serial log above says which."
                .into());
        }
        other => return Err(format!("the boot {other}; expected exit 33")),
    }
    if !log.contains("admission verdict") {
        return Err("the boot finished without reaching an admission verdict, so the stage \n\
             did not run."
            .into());
    }

    // And the registry, read out of the two halves that just ran rather than
    // restated beside them.
    println!();
    admission_reached(&report, &log)?;

    println!("\nadmission: the model held all three arms and the frame answered for this machine");
    Ok(())
}

/// `claims/0010`'s file.
const ADMISSION_CLAIM: &str = "claims/0010-admission-refusals.toml";

/// Every number `claims/0010` publishes, in the order the claim lists them.
///
/// Twelve come out of `f_sim::reserve::metrics` and two out of the boot, and
/// both halves print them under exactly these names. This is the list
/// [`admission_thresholds_match`] holds the registry's `[threshold]` table
/// against and [`admission_reached`] reads out of a run — the same guard
/// `hostile_thresholds_match` is for `claims/0008` and `entries_thresholds_match`
/// is for `claims/0009`, and it exists for the reason both of those do: a
/// published minimum with no counter behind it is a threshold nobody checks,
/// and a counter with no minimum behind it is a number that can go to zero
/// without a word.
const ADMISSION_METRICS: &[&str] = &[
    "deadlines_missed_granted",
    "reserved_slots_stolen",
    "oversubscribed_refusals",
    "periods_run_oversubscribed",
    "deadlines_missed_unreserved",
    "unreserved_slots_stolen",
    "placements_refused",
    "stretches_started",
    "bursts_at_release",
    "budget_overruns_clamped",
    "cores_held_idle",
    "reserved_slots_idle",
    "machine_grants",
    "described_grants",
];

/// The row `claims/0010` publishes and deliberately does **not** gate.
///
/// `machine_grants` is zero on QEMU — no thread level, no cache topology, no
/// RDT, so one contention domain with the frame in it — and non-zero on a part
/// that can deliver all four of RFC 0007's components. Both are legitimate, so
/// a threshold either way would make one of them a red build for the wrong
/// reason. The claim writes that out under R01; this is the same sentence in
/// code, so that a row quietly added for it later is a red build rather than a
/// silent narrowing of what this machine is allowed to be.
const ADMISSION_UNGATED: &[&str] = &["machine_grants"];

/// `claims/0010`'s `[threshold]` table, read.
///
/// Read rather than restated, for `hostile_thresholds`' reason: two copies of a
/// number are one number and one rumour, and the copy nobody reads is the one
/// that rots.
fn admission_thresholds() -> Result<std::collections::BTreeMap<String, Bound>, String> {
    let path = root().join(ADMISSION_CLAIM);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;

    let value = |rest: &str, which: &str| -> Option<u64> {
        let (_, after) = rest.split_once(which)?;
        after
            .trim_start()
            .strip_prefix('=')?
            .split_whitespace()
            .next()?
            .trim_end_matches([',', '}'])
            .parse()
            .ok()
    };

    let mut rows = std::collections::BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            inside = trimmed.trim_end().trim_end_matches('\r') == "[threshold]";
            continue;
        }
        if !inside || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        rows.insert(key.to_string(), Bound { min: value(rest, "min"), max: value(rest, "max") });
    }
    Ok(rows)
}

/// Require `claims/0010`'s `[threshold]` keys and [`ADMISSION_METRICS`] to be
/// one list, and the ungated row to stay ungated.
///
/// # Errors
///
/// A sentence naming which side a row is missing from.
fn admission_thresholds_match() -> Result<(), String> {
    let rows = admission_thresholds()?;
    let gated: std::collections::BTreeSet<&str> =
        ADMISSION_METRICS.iter().copied().filter(|key| !ADMISSION_UNGATED.contains(key)).collect();
    let stated: std::collections::BTreeSet<&str> = rows.keys().map(String::as_str).collect();

    let missing: Vec<&str> = gated.difference(&stated).copied().collect();
    let extra: Vec<&str> = stated.difference(&gated).copied().collect();
    if !missing.is_empty() || !extra.is_empty() {
        let say = |what: &str, list: &[&str]| {
            if list.is_empty() { String::new() } else { format!("\n  {what}: {}", list.join(", ")) }
        };
        return Err(format!(
            "{ADMISSION_CLAIM}'s [threshold] table and ADMISSION_METRICS have drifted.{}{}\n\n             A minimum with no metric behind it is a published number nothing checks, and a \n             metric with no minimum behind it is a count that can fall to zero without a word. \n             Move both, or say in the claim why the row is deliberately ungated and add it to \n             ADMISSION_UNGATED.",
            say("in the claim and not in ADMISSION_METRICS", &extra),
            say("in ADMISSION_METRICS and not in the claim", &missing),
        ));
    }

    // And the deliberate absence, checked rather than trusted. `claims/0010`
    // argues at length that `machine_grants` must not be gated; a row added for
    // it later would make either QEMU or RDT silicon a red build, and the
    // argument would still be sitting in the file saying it had not been.
    for key in ADMISSION_UNGATED {
        if rows.contains_key(*key) {
            return Err(format!(
                "{ADMISSION_CLAIM} now states a threshold for `{key}`, which it argues at \n                 length must not have one: zero is QEMU and non-zero is a part with RDT, and \n                 a bound either way makes one of those a red build for the wrong reason. If \n                 the argument has changed, change the argument first."
            ));
        }
    }
    Ok(())
}

/// Every number `claims/0010` publishes, taken out of the run that just
/// produced it, against the bound the registry states.
///
/// # Why the run is parsed rather than trusted
///
/// Because the verdicts inside `f_sim::reserve` and `kernel::admit` are the
/// model's own opinion of itself, and the registry is what a reader outside the
/// build is shown. Until this existed the thirteen rows of that `[threshold]`
/// table were read by nothing: they could drift from the checks that enforce
/// them, and both sides would read as agreement.
///
/// # Errors
///
/// A sentence naming the row, what the claim states and what the run produced —
/// or naming a row the run did not print at all, which is the same failure a
/// step earlier.
fn admission_reached(model: &str, boot: &str) -> Result<(), String> {
    admission_thresholds_match()?;
    let rows = admission_thresholds()?;

    let read = |key: &str| -> Option<u64> {
        for line in model.lines().chain(boot.lines()) {
            let mut words = line.split_whitespace();
            if words.next() != Some(key) {
                continue;
            }
            if let Some(value) = words.next().and_then(|v| v.parse::<u64>().ok()) {
                return Some(value);
            }
        }
        None
    };

    let mut short: Vec<String> = Vec::new();
    for key in ADMISSION_METRICS {
        let Some(seen) = read(key) else {
            return Err(format!(
                "the run did not print `{key}`, which {ADMISSION_CLAIM} publishes.\n\n                 A claim whose reproduction command does not print its own numbers is a \n                 claim nobody can check. `f_sim::reserve::metrics` prints twelve of them and \n                 the boot's admission stage prints two; this one reached neither."
            ));
        };
        let Some(bound) = rows.get(*key) else { continue };
        if let Some(min) = bound.min
            && seen < min
        {
            short.push(format!(
                "{key}: {ADMISSION_CLAIM} states min {min}, the run produced {seen}"
            ));
        }
        if let Some(max) = bound.max
            && seen > max
        {
            short.push(format!(
                "{key}: {ADMISSION_CLAIM} states max {max}, the run produced {seen}"
            ));
        }
    }

    if short.is_empty() {
        println!(
            "  {:<12} {} row(s) of {ADMISSION_CLAIM} met, read out of the run",
            "thresholds",
            ADMISSION_METRICS.len(),
        );
        return Ok(());
    }
    Err(format!(
        "{ADMISSION_CLAIM} states bounds this run did not meet:\n\n  {}\n\n         The minimums are the load-bearing half: without them every zero in that claim is \n         free, because a run in which the adversary did nothing reports the same zeros as a \n         run in which it did everything.",
        short.join("\n  "),
    ))
}

/// One run of the reservation model, as a subprocess, reduced to its digest.
fn admission_hash(seed: &str) -> Result<String, String> {
    let out =
        capture("cargo", &["run", "-q", "-p", "f-sim", "--", "--admission-hash", "--seed", seed])?;
    Ok(out.trim().to_string())
}

/// One run of the reservation model, as a subprocess, with its report and its
/// verdict.
///
/// Captured rather than streamed and answered rather than raised, for
/// [`chaos_report`]'s reason: a failing verdict has to print its report.
fn admission_report(seed: &str) -> Result<(bool, String), String> {
    let out = Command::new("cargo")
        .args(["run", "-q", "-p", "f-sim", "--", "--admission", "--seed", seed])
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run f-sim: {e}"))?;
    let text =
        String::from_utf8(out.stdout).map_err(|e| format!("f-sim printed non-UTF-8: {e}"))?;
    Ok((out.status.success(), text))
}

/// One chaos sweep, as a subprocess, with its report and its verdict.
///
/// The output is captured rather than streamed and the status is answered rather
/// than turned into an error, because a failing verdict has to print its report:
/// a gate that says only *failed* is a gate whose first debugging step is running
/// the command again by hand.
fn chaos_report(seed: &str, dir: &str) -> Result<(bool, String), String> {
    let out = Command::new("cargo")
        .args(["run", "-q", "-p", "f-sim", "--", "--chaos", "--seed", seed, "--components", dir])
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run f-sim: {e}"))?;
    let text =
        String::from_utf8(out.stdout).map_err(|e| format!("f-sim printed non-UTF-8: {e}"))?;
    Ok((out.status.success(), text))
}

/// The scenario set, as the simulator prints it.
fn sim_list() -> Result<(), String> {
    sh("cargo", &["run", "-q", "-p", "f-sim", "--", "--list"])
}

/// Print one scenario's trace hash and nothing else.
///
/// The same shape as `trace --hash` and for the same consumer: a CI job where
/// two runners each produce a line and a third compares them. It takes a seed as
/// well as a scenario, which `trace --hash` cannot yet do because the kernel does
/// not take one on its command line — that is the only asymmetry between the two
/// halves, and it is the simulator having the better of it. `E1-P03` sweeps by
/// calling this with a seed per run.
fn sim_hash_only(scenario: Option<&str>, seed: Option<&str>) -> Result<(), String> {
    // Built first and quietly: the `deployment` scenario's component set *is*
    // the compiled manifest records, so a hash of it taken against a stale or
    // missing build would be a hash of the wrong commit — which is the one thing
    // a `(seed, commit)` pair may not be wrong about.
    components_quietly()?;
    let name = scenario.unwrap_or(SIM_SCENARIO);
    println!("{:#018x}", sim(name, seed.unwrap_or(TRACE_SEED))?);
    Ok(())
}

/// The simulator's reproduction check.
///
/// # What is being claimed
///
/// That a `(seed, commit)` pair names one simulated run, byte for byte. Each
/// scenario is run twice at [`TRACE_SEED`] in two separate processes and once at
/// [`SIM_OTHER_SEED`]; the pair must agree and the odd one must not.
///
/// # Why the second seed is not optional
///
/// `trace_check` says it at length about a deliberate defect and it is the same
/// argument here: a reproduction check that has only ever passed is
/// indistinguishable from one that cannot fail. A scenario whose digest ignored
/// its seed would agree with itself forever, and a nightly sweep across
/// thousands of seeds would report having explored a space it never entered.
/// That is the one failure a test apparatus must not have, so the command
/// requires a different seed to give a different answer before it reports green.
///
/// # Digests, and the one comparison that is over bytes
///
/// Every scenario above is compared by its digest, which is what a CI job over
/// two runners can carry in a file. The exit criterion says *byte-identically*,
/// though, and a 64-bit FNV-1a digest is not a byte comparison — `sim/src/trace.rs`
/// says as much about itself. So one scenario is additionally compared **as
/// bytes**, in two processes, and it is [`SIM_DEPLOYMENT`] because that is the
/// scenario the exit criterion is about. What remains digest-identity and not
/// byte-identity is the *cross-runner* claim in `.github/workflows/ci.yml`, and
/// it is written down there so nobody quotes that job as the byte-level
/// evidence.
fn sim_check() -> Result<(), String> {
    components_quietly()?;
    println!(
        "simulation reproduction check — seed {TRACE_SEED}
"
    );
    println!("  {:<12} {:>18}  {:>18}  {:>18}", "scenario", "run 1", "run 2", SIM_OTHER_SEED);

    for name in sim_scenarios()? {
        let first = sim(&name, TRACE_SEED)?;
        let second = sim(&name, TRACE_SEED)?;
        let other = sim(&name, SIM_OTHER_SEED)?;
        println!("  {name:<12} {first:#018x}  {second:#018x}  {other:#018x}");

        if first != second {
            return Err(format!(
                "two runs of `{name}` at one seed produced different traces.

                 This is the determinism contract failing above the frame, and every
                 layer that reads a simulated run rests on it: a seed stops being a bug
                 report, a sweep stops shrinking, and a snapshot stops re-entering.
                 Something in the model is reading a clock, an address or an iteration
                 order the seed does not own. RFC 0004, RFC 0032."
            ));
        }
        if first == other {
            return Err(format!(
                "`{name}` produced the same trace at two different seeds.

                 That means the check above cannot fail, which makes it worth nothing:
                 a digest over something that does not vary agrees with itself forever.
                 Either the scenario has stopped taking any interleaving decision, or
                 the digest is being taken over something the run does not reach."
            ));
        }
    }

    // And one of them compared as bytes rather than as a digest, because that is
    // the word the exit criterion uses. Two processes, so the comparison is
    // between two artefacts that share nothing but the commit — the in-process
    // pair `sim/src/scenario.rs` asserts shares an address space and an
    // allocator with itself, which is the weaker shape by this file's own
    // argument for running the simulator as a subprocess at all.
    let first = sim_trace(SIM_DEPLOYMENT, TRACE_SEED)?;
    let second = sim_trace(SIM_DEPLOYMENT, TRACE_SEED)?;
    if first != second {
        let differs = first
            .lines()
            .zip(second.lines())
            .position(|(a, b)| a != b)
            .map_or_else(|| "in its length".to_string(), |line| format!("at line {}", line + 1));
        return Err(format!(
            "two runs of `{SIM_DEPLOYMENT}` at one seed produced artefacts differing {differs}.

             The digests above agreed, so this is either a difference too small for a
             64-bit hash to have caught or a digest taken over less than the artefact.
             Either way the word in the exit criterion is *byte-identically*, and this is
             the check that means it."
        ));
    }
    println!(
        "\n  {:<12} {} bytes of `{SIM_DEPLOYMENT}`, identical across two processes",
        "byte-for-byte",
        first.len()
    );

    println!(
        "
sim: ok — every scenario reproduced from its seed and moved when the seed did, and one
         of them was compared as bytes rather than as a digest.
         Two processes on one machine. The pair that matters is two runners, and that
         is the CI job: same commit, same seed, hashes compared — exactly as `trace` does
         for the boot, and digest-identity rather than byte-identity for the same reason.
         RFC 0032 says where the seam between the two lies."
    );
    Ok(())
}

/// The scenario whose component set is read from the compiled manifest records.
///
/// Named here as well as in the simulator because [`sim_join`] reports about it
/// and a message that said *some scenario* would be a message nobody could act
/// on. `f-sim --list` is still the one source of the scenario *set*.
const SIM_DEPLOYMENT: &str = "deployment";

/// The components the simulator runs that this boot does not spawn, by the name
/// their record declares.
///
/// # Why a list and not a tolerance
///
/// Because the two halves of `boot-to-workload` are supposed to be about one
/// component set, and where they are not, the difference has to be a quantity
/// somebody wrote down. A check that merely allowed the simulator to run *more*
/// components than the boot would go on passing while the workload half drifted
/// away from the boot half one component at a time — the failure RFC 0035 built
/// this command to catch, arriving through the door the first version of it left
/// open.
///
/// So [`sim_join`] requires the difference to equal this list exactly. Adding a
/// component the boot does not spawn is red until somebody says so here;
/// spawning one that is in the list is red until the entry goes.
///
/// # What is in it, and who removes it
///
/// **Nothing, and the emptiness is the evidence.** It held `virtio-blk` for as
/// long as the frame instantiated one place from the first module it was handed
/// — `kernel/src/component.rs`, `*modules.first()` — and RFC 0036 said in its
/// own reversal section that when a boot spawns the whole module set this list
/// is empty and the entry's removal is what says so. RFC 0044 is that change:
/// the frame fills a place per component file, each staked with an account its
/// own manifest sized, and the boot log now carries a spawn line with a content
/// hash for every record the build produced.
///
/// An empty list is not a weaker check than a full one. [`hold_the_gap`]
/// requires **equality**, so a component the simulator runs that this boot does
/// not spawn is red with nothing to compare it against — which is the direction
/// that catches a new component file, and the one that was live in the tree when
/// this constant was written. What an empty list can no longer exercise is the
/// *other* direction, a stale entry, because there is no entry left to go stale;
/// that half is held by a test at the foot of this file against a list it
/// supplies itself, and that test says so rather than implying this constant
/// still covers it.
const JOIN_GAP: &[&str] = &[];

/// The two halves of `boot-to-workload`, over one component set.
///
/// # What is being claimed, and what is not
///
/// E1-P01's exit says *a whole boot-to-workload run executes under simulation
/// and reproduces byte-identically from `(seed, commit)`*. RFC 0032 decided that
/// the simulator models the system above the frame, which makes that sentence a
/// claim about a **pair** of runs rather than about one process, and RFC 0035
/// makes the pair checkable rather than asserted:
///
/// - `cargo xtask trace --hash` boots the real kernel, which spawns components
///   from the compiled manifest records the loader hands it, and hashes the log
///   — a log in which each spawned component's content hash is printed.
/// - `cargo xtask sim --hash deployment` reads *those same component files*,
///   builds one actor per record with the model its declared protocol names,
///   drives a workload through them, and hashes its own artefact.
///
/// This command is what makes those two the same component set rather than two
/// commands in one paragraph: it boots, reads the hashes out of the log, asks
/// the simulator which components it would run, and compares the two sets **in
/// both directions**. Without it the seam is a shared filename, and a shared
/// filename is not evidence.
///
/// Both directions, because one of them was the hole. `spawned ⊆ modelled` is
/// satisfied while the simulator drives components the kernel never
/// instantiated — which is the tree as it stands, where the frame builds one
/// place from the first module. The set the simulator runs and the boot does
/// not is therefore computed and required to equal [`JOIN_GAP`], so that the
/// gap is a declared quantity somebody has to change rather than a silence.
/// RFC 0036.
///
/// What it does not claim: that the frame's instructions ran under the
/// simulator. They did not, they never do, and every artefact this simulator
/// writes says so in its own header.
fn sim_join() -> Result<(), String> {
    components_quietly()?;
    println!("the boot-to-workload seam — one component set, two runs\n");

    println!("[1/2] the boot: the real kernel in QEMU, spawning from compiled records");
    let (ending, log) = machine_with(None, &[], Capture::Quiet, BOOT_TIMEOUT, BOOT_MEMORY)?;
    if ending != Ending::Exited(33) {
        print!("{log}");
        return Err(format!("the boot {ending}; expected 33"));
    }
    let (files, spawned) = spawned_from(&log)?;
    println!("  component file(s)  {files}");
    for id in &spawned {
        println!("  spawned            {id:#018x}");
    }

    println!("\n[2/2] the workload: the simulator, reading the same records");
    let dir = component_dir()?;
    let listed = capture(
        "cargo",
        &["run", "-q", "-p", "f-sim", "--", "--deployment", "--components", &dir],
    )?;
    let mut modelled: Vec<(String, u64)> = Vec::new();
    for line in listed.lines() {
        let mut fields = line.split_whitespace();
        let (Some(name), Some(hash)) = (fields.next(), fields.next()) else {
            continue;
        };
        let id = hash
            .strip_prefix("0x")
            .and_then(|digits| u64::from_str_radix(digits, 16).ok())
            .ok_or_else(|| format!("f-sim printed `{line}`, which is not a component"))?;
        println!("  {name:<32} {id:#018x}");
        modelled.push((name.to_string(), id));
    }

    if modelled.is_empty() {
        return Err("the simulator read no component files, so there is nothing to join".into());
    }
    // Demoted, and the demotion is the finding. Both numbers are counts of
    // `target/component/`: the loader is handed what `cargo xtask component`
    // built, and the simulator reads the same directory. A disagreement says
    // one of the two saw a stale or partial build; it says *nothing* about
    // which modules the kernel instantiated. RFC 0035 rejected exactly this
    // shape as evidence about the boot — "a check that read the component
    // directory twice would agree with itself whatever the kernel had done" —
    // and this file shipped it anyway as half the join. It is kept for the one
    // failure it can see, and the two set checks below are what read the boot.
    if modelled.len() != files {
        return Err(format!(
            "the loader was handed {files} component file(s) and the simulator read \
             {}.\n\n\
             Both counts are of `target/component/`, so this is a stale or partial build \
             rather than a disagreement about the deployment — one of the two ran before \
             `cargo xtask component` finished. It says nothing about which components the \
             boot instantiated; the checks that read that are below.",
            modelled.len()
        ));
    }
    for id in &spawned {
        if !modelled.iter().any(|(_, modelled)| modelled == id) {
            return Err(format!(
                "the boot spawned the component whose manifest is {id:#018x}, and the \
                 simulator ran no such component.\n\n\
                 A content hash covers a component's record and its image together, so this \
                 is the two halves disagreeing about what a component *is* rather than about \
                 which one to run. RFC 0030, RFC 0035."
            ));
        }
    }

    // The other direction, which the first review of this command found
    // missing — and it is the direction that was live in the tree rather than
    // hypothetical. `spawned ⊆ modelled` above is satisfied by a boot that
    // instantiates one module while the simulator drives four, which is what
    // this tree did until RFC 0044: `kernel/src/component.rs` built one place
    // from `*modules.first()`. It builds one per component file now, so the
    // difference is empty — and it is still computed, printed, and required to
    // be *exactly* the gap `JOIN_GAP` declares, because a set and not a bound is
    // what makes a new component file that nobody spawns go red rather than pass
    // unmentioned. R04.
    let unspawned = unspawned(&modelled, &spawned);
    for name in &unspawned {
        println!("  not spawned        {name}");
    }
    hold_the_gap(&unspawned, JOIN_GAP)?;

    println!(
        "\njoin: ok — {} of the {} component(s) the simulator ran were spawned by this boot, and\n\
         \x20     the {} that were not are the gap this tree declares: {}. RFC 0036 required\n\
         \x20     that difference to be a declared set and RFC 0044 emptied it, so the emptiness\n\
         \x20     is the evidence rather than a relaxed check: this is equality in both\n\
         \x20     directions, and a component file nobody spawns is red with nothing to\n\
         \x20     compare it against.\n\
         \x20     `cargo xtask trace --hash` hashes the boot and `cargo xtask sim --hash \
         {SIM_DEPLOYMENT}`\n\
         \x20     hashes the workload. RFC 0035 states what the pair claims — and what it does\n\
         \x20     not: the frame's instructions run in QEMU and nowhere else.",
        spawned.len(),
        modelled.len(),
        unspawned.len(),
        if unspawned.is_empty() { "none".to_string() } else { unspawned.join(", ") }
    );
    Ok(())
}

/// The components the simulator ran and the boot did not, by name, in order.
///
/// A function rather than four lines inside [`sim_join`] because the check it
/// feeds is the one this command exists for, and a check that can only be
/// exercised by booting QEMU is a check nothing tests. Its tests are at the foot
/// of this file, and one of them is the input the review named: a third
/// component file that nothing spawns.
fn unspawned<'a>(modelled: &'a [(String, u64)], spawned: &[u64]) -> Vec<&'a str> {
    let mut names: Vec<&str> = modelled
        .iter()
        .filter(|(_, id)| !spawned.contains(id))
        .map(|(name, _)| name.as_str())
        .collect();
    names.sort_unstable();
    names
}

/// Refuse unless the two halves differ by exactly the gap this tree declares.
///
/// Equality and not containment, in both directions, which is the whole of
/// RFC 0036: a component the workload half covers that nobody declared is a
/// silent widening, and a declared component the boot has started spawning is a
/// stale exception — a hole a later check steps over. Neither is a state this
/// command may report as green.
///
/// # Errors
///
/// A sentence naming both sets and what to do about the difference.
fn hold_the_gap(unspawned: &[&str], gap: &[&str]) -> Result<(), String> {
    let mut declared: Vec<&str> = gap.to_vec();
    declared.sort_unstable();
    if unspawned == declared {
        return Ok(());
    }
    Err(format!(
        "the simulator ran {unspawned:?} that the boot did not spawn, against a declared \
         gap of {declared:?}.\n\n\
         The two halves of `boot-to-workload` are about one component set, and where they \
         are not, the difference is written down rather than discovered. A component here \
         that is not in `JOIN_GAP` is one the workload half covers and the boot half does \
         not, with nobody having said so — either the boot spawns it, or it goes in the \
         list with its reason and the task that removes it. A component in `JOIN_GAP` that \
         is no longer here is a boot that has started spawning it, and a stale entry is a \
         hole this check would step over. RFC 0035, RFC 0036."
    ))
}

/// What a boot log says it spawned: how many component files, and which
/// manifests.
///
/// Reads the log rather than the files, deliberately. The claim being checked is
/// about what the *kernel* did, and a check that read the same directory twice
/// would agree with itself whatever the kernel had done with it.
fn spawned_from(log: &str) -> Result<(usize, Vec<u64>), String> {
    const FILES: &str = " component file(s)";
    const MANIFEST: &str = "manifest 0x";

    let mut files = None;
    let mut spawned = Vec::new();
    for line in log.lines() {
        if let Some(at) = line.find(FILES)
            && let Some(head) = line.get(..at)
            && let Some(count) =
                head.split_whitespace().last().and_then(|w| w.parse::<usize>().ok())
        {
            files = Some(count);
        }
        if let Some(at) = line.find(MANIFEST)
            && let Some(rest) = line.get(at + MANIFEST.len()..)
        {
            let digits: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
            if let Ok(id) = u64::from_str_radix(&digits, 16) {
                spawned.push(id);
            }
        }
    }

    let files = files.ok_or(
        "the boot log did not say how many component files it was handed.\n\n\
         `kernel/src/component.rs` prints that line, and this check reads it. If the \
         wording moved, this check has to move with it — a join that silently stopped \
         finding its evidence would report green forever.",
    )?;
    if spawned.is_empty() {
        return Err("the boot spawned no component at all, so there is nothing to join".into());
    }
    Ok((files, spawned))
}

// ---------------------------------------------------------------------------
// E1-P03: the seed sweep, its minimiser and its corpus.
//
// The division of labour between this file and `f-sim` is the whole reason the
// sweep is trustworthy, so it is stated here rather than inferred:
//
//   f-sim  decides which trials to run, in what order, and what verdict each
//          gets, shrinks every failure, and prints the report. It reads no
//          clock — `cargo xtask lint-determinism` scans `sim/` with no
//          allow-list entry, so it could not — and its output is a function of
//          its arguments alone.
//   xtask  supplies the two things a pure function cannot know: the commit the
//          run belongs to, and where the build left the component files. It
//          then times the whole thing, because a sweep nobody can afford to run
//          is a sweep nobody runs, and prints the cost beside the report rather
//          than inside it.
//
// RFC 0040 is the record.
// ---------------------------------------------------------------------------

/// How many seeds `cargo xtask sweep` runs when it is not told. Unit: seeds.
///
/// The same number `f_sim::sweep::DEFAULT_SEEDS` states, passed explicitly for
/// [`TRACE_SEED`]'s reason: the contract is about a pair, and a pair with an
/// implicit half is a pair nobody can quote.
const SWEEP_SEEDS: u32 = 64;

/// The deliberate defect the sweep's own harness arms.
///
/// One, and it lives in `sim/src/dev.rs`, where the argument for putting it in
/// the shipped source rather than in a patch is written out. It is in
/// [`DEFECTS`] too, which is what `lint-mutations` reads.
const SWEEP_DEFECT: &str = "mutate-crossed-completion";

/// The second deliberate defect, and the check it is here to prove can fire.
///
/// One defect proves one signature. `mutate-crossed-completion` trips
/// `check::held`, which is the first entry in the oracle's table and therefore
/// the only signature it can ever produce — so a harness built on it alone
/// demonstrates that *a* check can fail and says nothing about the other four.
/// This one withholds the reset notification a device owes its client, which
/// leaves operations issued and never answered: `check::balance`, or
/// `check::bound` in the runs where nothing was in flight when the device fell
/// over. The harness requires a signature other than `held`, which is the
/// assertion that makes the table more than one property wide. RFC 0042.
const SWEEP_DEFECT_TWO: &str = "mutate-silent-reset";

/// Signatures the second defect is allowed to be found by. Unit: none — check
/// names from `f_sim::check::CHECKS`.
///
/// Two rather than one because which of them fires depends on whether the
/// client had work outstanding when its device fell over, and that is a seeded
/// property of the scenario rather than a choice. Either is a different check
/// from `held`, which is the whole requirement.
const SWEEP_DEFECT_TWO_CHECKS: &[&str] = &["balance", "bound"];

/// How many seeds the mutation harness sweeps. Unit: seeds.
///
/// Enough that the defect is reached — it needs two consecutive coalescing
/// decisions with work behind them, so a handful of seeds would be a coin toss —
/// and small enough that `verify` does not grow a minute. The harness fails
/// loudly if this stops being enough, which is the reversal condition rather
/// than a comment.
const MUTATE_SEEDS: u32 = 16;

/// How many threads a sweep is given.
///
/// A cost knob and never a verdict: `f_sim::sweep` lays the grid out before a
/// worker starts and assembles the report in grid order, and its own test runs
/// one sweep at one worker and at five and requires one report. Read from the
/// machine here rather than defaulted inside `f-sim`, so that the simulator's
/// output stays a function of its arguments and this file owns the one number
/// that depends on where it is running.
fn sweep_jobs() -> String {
    std::thread::available_parallelism().map_or(1, std::num::NonZeroUsize::get).to_string()
}

/// The commit half of `(seed, commit)`.
///
/// Fatal rather than `unknown`, on `release`'s argument and for the same reason:
/// a report naming an unidentified tree is not a degraded report, it is a
/// confident statement about nothing — and a seed without a commit reproduces
/// nothing at all.
///
/// # Why there is a second source, and why it is not a weakening
///
/// A release package is the one tree that is exactly a commit and has no
/// repository in it. `git archive` writes files and no `.git`, which is the
/// property that makes the source content-addressable at all — and it is also
/// why `E1-R01` measured the published sweep refusing to run from an unpacked
/// package with `cannot read the commit from git`. The refusal was right and
/// the tree was not unidentified: the packager had already written the commit
/// into `MANIFEST`, and nothing looked there.
///
/// So the order is git first and the manifest second, never the other way
/// round. In a checkout git is the only witness worth believing, because a
/// `MANIFEST` there is a file anybody can write; in an unpacked package git is
/// absent, and the manifest is the packager's own statement, hashed into the
/// address the package is named by. A tree with neither is still fatal, which
/// is the whole of what this function was protecting.
///
/// It does not make the tree *clean*. [`sweep_dirty`] reads a git that cannot
/// answer as dirty and keeps doing so here: an unpacked package is that commit
/// by construction, nothing in it can check that it still is, and so a finding
/// from one carries the commit without the `git switch` line in front of it.
/// RFC 0056.
fn sweep_commit() -> Result<String, String> {
    if let Ok(out) = capture("git", &["rev-parse", "HEAD"]) {
        return Ok(out.trim().to_string());
    }
    match manifest_commit() {
        Ok(commit) => Ok(commit),
        Err(why) => Err(format!(
            "cannot read the commit from git, and no release MANIFEST names one\n\n  \
             {why}\n\n{}",
            SWEEP_COMMIT_ADVICE
        )),
    }
}

/// The paragraph both halves of the refusal above end with.
const SWEEP_COMMIT_ADVICE: &str = "A sweep prints `(seed, commit)` pairs, so this is fatal. In a container it \
         is usually git refusing a working tree owned by another uid; \
         docker/Dockerfile marks the tree safe, and an image built before that does \
         not. In an unpacked release package the commit comes from `MANIFEST`, which \
         is a member of the package tar rather than of `source.tar`, so unpack both \
         into one directory. RELEASING.md, RFC 0056.";

/// The commit a release package's `MANIFEST` names, when there is no repository.
///
/// Fails closed three times over, because this is the path with no witness
/// behind it. The line has to begin `commit` at column zero — a hash row begins
/// with its own digest and cannot be mistaken for one — the value has to be
/// forty lowercase hex digits, which is what `git rev-parse HEAD` writes and
/// what a truncated or re-wrapped manifest does not, and at least one file the
/// manifest names has to be here with the hash it states.
///
/// # What the third check is for, and what it cannot do
///
/// It ties the manifest to *this* tree. The failure it catches is ordinary
/// rather than adversarial: two releases unpacked in the same place, or a
/// `MANIFEST` left behind from an earlier one, and a sweep that then prints
/// seeds against a commit whose source is not the source it just ran. One
/// matching row is enough and the search stops there, so the usual cost is one
/// small file hashed; requiring *the first* row to match would refuse a package
/// a stranger has legitimately edited to chase a finding, which is the reason
/// somebody unpacks one.
///
/// It cannot detect a manifest that is internally consistent and false, because
/// the hash rows and the commit line are in the same file and whoever edits one
/// edits the other. Nothing inside an unpacked package can — the package's own
/// address is over the archive and is not in it. That is the same limit
/// [`sweep_dirty`] states by calling such a tree dirty, and it is why the
/// commit travels without a `git switch` line in front of it. RFC 0056.
///
/// Every refusal returns *why*, because the three cases want three different
/// actions from a reader: unpack the second tar, get a manifest that is not
/// truncated, or unpack into a directory of its own.
fn manifest_commit() -> Result<String, String> {
    let text = std::fs::read_to_string(root().join("MANIFEST"))
        .map_err(|_| "no MANIFEST sits at the root of this tree".to_string())?;
    let value = text
        .lines()
        .find_map(|line| line.strip_prefix("commit"))
        .and_then(|rest| rest.split_whitespace().next())
        .ok_or_else(|| "the MANIFEST here has no `commit` line".to_string())?;
    let hex = value.len() == 40
        && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
    if !hex {
        return Err(format!(
            "the MANIFEST here names `{value}`, which is not a forty-digit commit"
        ));
    }
    manifest_names_this_tree(&text)?;
    Ok(value.to_string())
}

/// One hash row of a manifest, checked against the file on disk it names.
///
/// Rows are `<64 hex>  <path>` and nothing else in the file has that shape, so
/// the version, commit, sweep and claim lines and every comment are skipped by
/// the same test that recognises a row. A file the manifest names and the tree
/// does not have is skipped rather than fatal: `source.tar` unpacks over the
/// package's own copies, and a stranger is told to unpack it second.
fn manifest_names_this_tree(text: &str) -> Result<(), String> {
    let mut rows = 0usize;
    for line in text.lines() {
        let mut fields = line.split_whitespace();
        let (Some(hash), Some(name), None) = (fields.next(), fields.next(), fields.next()) else {
            continue;
        };
        let hex = hash.len() == 64
            && hash.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b));
        if !hex {
            continue;
        }
        rows += 1;
        let Ok(bytes) = std::fs::read(root().join(name)) else { continue };
        if pack::hex(&pack::sha256(&bytes)) == hash {
            return Ok(());
        }
    }
    Err(if rows == 0 {
        "the MANIFEST here lists no files, so nothing ties it to this tree".to_string()
    } else {
        format!(
            "none of the {rows} file(s) this MANIFEST names is here with the hash it \
             states, so it is a manifest for some other tree"
        )
    })
}

/// The other half of the same question: whether the tree is that commit.
///
/// # Why a commit alone was not enough, and why this is not fatal
///
/// `sweep_commit` answers *which commit is checked out*, and a report that
/// printed only that was asserting something nobody had checked. HEAD names a
/// commit; it says nothing about the files the compiler read. On a tree with
/// uncommitted work in it — the ordinary state of the audience `E1-R01`
/// published this for, because somebody sweeping their own checkout to find
/// something new is usually sweeping a checkout they have changed — the
/// report's own `repro` line said `git switch --detach <sha> && ...`, and
/// following it discards the changes that produced the finding and then runs a
/// different program.
///
/// Refusing would be the wrong end of R04: it would break the published command
/// on exactly the trees it was published for. So the fact is measured and passed
/// on, `f-sim` prints it, and the reproduction line is emitted in the form that
/// is true of the tree. `release` names its package `-dirty` and `reproduce`
/// prints *(dirty — not a quotable tree)* off the same two words; this is the
/// third caller and the mechanism is theirs. RFC 0055.
///
/// A git that cannot answer is read as *dirty*, which is the answer that claims
/// least: the alternative is a clean-looking report from a tree nothing
/// identified.
fn sweep_dirty() -> bool {
    !capture("git", &["status", "--porcelain"]).is_ok_and(|out| out.trim().is_empty())
}

/// `clean` or `dirty`, as `f-sim --tree` spells it.
fn sweep_tree() -> &'static str {
    if sweep_dirty() { "dirty" } else { "clean" }
}

/// Run `f-sim` with the given features and arguments, answering
/// `(clean, output)`.
///
/// The output is printed as well as returned, because a sweep's report is the
/// thing a person came for and a harness that swallowed it would make its own
/// summary the only evidence. A non-zero exit is *a finding* rather than an
/// error — `f-sim` uses the status that way deliberately — so this cannot use
/// [`capture`], which treats one as a failure.
fn f_sim(features: &[&str], args: &[&str]) -> Result<(bool, String), String> {
    let mut argv: Vec<String> =
        ["run", "-q", "-p", "f-sim"].iter().map(|s| (*s).to_string()).collect();
    if !features.is_empty() {
        argv.push("--features".into());
        argv.push(features.join(","));
    }
    argv.push("--".into());
    argv.extend(args.iter().map(|s| (*s).to_string()));

    let out = Command::new("cargo")
        .args(&argv)
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    print!("{text}");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    // `f-sim` answers 0 for a clean run and 1 for a finding, and refuses with a
    // message on standard error for anything it could not do at all. The two are
    // told apart by whether it printed a report: a refusal prints nothing on
    // standard output, which is the signal available without inventing a third
    // exit code that every caller would then have to know about.
    if text.trim().is_empty() {
        return Err(format!("f-sim refused: {}", stderr.trim()));
    }
    Ok((out.status.success(), text))
}

// ---------------------------------------------------------------------------
// E1-P08 — a long run is re-entered near its end rather than replayed.
// ---------------------------------------------------------------------------

/// The scenario `cargo xtask snapshot` runs.
///
/// Not in `f-sim --list` and not in the sweep's grid, because it is forty
/// simulated minutes long — `sim/src/scenario.rs`'s `LONG` table is where the
/// split by cost is argued.
const SNAPSHOT_SCENARIO: &str = "soak";

/// Where the strike goes in, in consultations of `peergone`.
///
/// Chosen so that the run goes wrong in **simulated minute 40**, which is the
/// minute `E1-P08`'s exit names. It is a number about the shipped scenario and
/// not about a machine: `soak` publishes one completion per operation, the run
/// is forty-four simulated minutes long over a hundred and twenty thousand of
/// them, and this is ninety-two per cent of the way through. The harness prints
/// the minute it actually landed in and refuses if it is not the one the exit
/// asks about, so a scenario change that moved it fails loudly rather than
/// quietly demonstrating something else.
const SNAPSHOT_STRIKE: &str = "peergone:110000:1";

/// The minute the failure must land in, and the minute it is re-entered at.
const SNAPSHOT_MINUTE: u64 = 40;

/// The saving `cargo xtask snapshot` requires before it calls the exit met.
///
/// Ten. The measured figure on the four-core development container is far above
/// it — `claims/0007` carries the number and its reproduction — and the
/// threshold is here rather than the measurement because a gate that asserted a
/// machine's number would go red on a slower machine for a reason that is not a
/// regression. What ten rules out is the thing worth ruling out: a re-entry that
/// costs the same order as the replay it exists to avoid, which is what the
/// *whole* snapshot below actually does and is why both are measured.
const SNAPSHOT_SAVING: u128 = 10;

/// What a re-entry from a **whole** mark is allowed to cost, as a multiple of
/// replaying.
///
/// Four, and it is a *ceiling* rather than a floor because the honest answer is
/// that a whole mark is **not a saving at all**. Measured on the four-core
/// development container it costs about what the replay costs — 526 ms against
/// 1055 ms in one run of this verb and 1252 ms against 515 ms in another, so the
/// ratio wanders either side of one — because reading half a million records
/// back costs the same order as producing them. RFC 0043 measured that, and it
/// is why there are two kinds of mark at all.
///
/// So this number is not a claim that the whole mark is fast. It is the guard
/// that stops it becoming *slow*: a change that made re-entry cost several
/// replays would turn the judgeable half of this verb into something nobody
/// would run, and today nothing would notice. Four rather than two because both
/// halves are single wall-clock samples on a shared container and the observed
/// spread already reaches 2.4x.
///
/// *Reversal:* the day `check::examine` can judge a tail, the whole mark stops
/// being the only judgeable artefact and this constant goes with it.
const SNAPSHOT_WHOLE_CEILING: u128 = 4;

/// How many times each timed command is run before its cost is believed. Unit:
/// runs.
const SNAPSHOT_SAMPLES: usize = 3;

/// Build `f-sim` in release with `features`, and put the binary somewhere it
/// will not be overwritten by the next build.
///
/// Two things, and both are about honest numbers.
///
/// **Release**, because this verb exists to answer *what does re-entering cost
/// against replaying*, and a wall-clock number measured on an unoptimised build
/// is a number about the build. `f_sim` above stays as it is: every other caller
/// wants a verdict rather than a stopwatch.
///
/// **Copied aside**, because the comparison needs two binaries — one with the
/// deliberate defect and one without — and cargo keeps one `f-sim` per target
/// directory whatever the features. Building the second would silently replace
/// the first, and the refusal in step five would then be a binary refusing its
/// own snapshot for no reason at all.
///
/// And the invocation is `cargo build` followed by running the binary, rather
/// than `cargo run`. That is not tidiness: `cargo run` spends several hundred
/// milliseconds deciding the build is fresh, and several hundred milliseconds is
/// two orders of magnitude more than a re-entry costs — so timing through cargo
/// would measure cargo and report it as the simulator.
fn f_sim_built(features: &[&str], name: &str) -> Result<PathBuf, String> {
    let mut argv: Vec<String> =
        ["build", "-q", "--release", "-p", "f-sim"].iter().map(|s| (*s).to_string()).collect();
    if !features.is_empty() {
        argv.push("--features".into());
        argv.push(features.join(","));
    }
    sh("cargo", &argv.iter().map(String::as_str).collect::<Vec<_>>())?;

    let built =
        target_dir().join("release").join(if cfg!(windows) { "f-sim.exe" } else { "f-sim" });
    let kept = target_dir().join("snapshot-bin");
    std::fs::create_dir_all(&kept).map_err(|e| format!("creating {}: {e}", kept.display()))?;
    let kept = kept.join(name);
    std::fs::copy(&built, &kept)
        .map_err(|e| format!("copying {} to {}: {e}", built.display(), kept.display()))?;
    Ok(kept)
}

/// Run a built `f-sim`, timed.
fn f_sim_timed(binary: &Path, args: &[&str]) -> Result<(bool, String, u128), String> {
    let started = std::time::Instant::now();
    let out = Command::new(binary)
        .args(args)
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run {}: {e}", binary.display()))?;
    let elapsed = started.elapsed().as_millis();
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if text.trim().is_empty() {
        return Err(format!("f-sim refused: {}", stderr.trim()));
    }
    Ok((out.status.success(), text, elapsed))
}

/// The buffer an oracle's evidence line names, as the sixteen hex digits a
/// trace prints it with.
///
/// Every finding in `check.rs` names one, and this verb needs it for one
/// purpose: to require that the failure is in the part of the run the re-entry
/// actually executes. Without that, every assertion here would still pass with
/// the defect moved to minute five — a whole mark carries the prefix, so it
/// would report a finding it read off disk — and the sentence this verb prints
/// would be false while it was green. That is the shape of failure this epoch
/// has already caught three times.
fn failing_buffer(evidence: &str) -> Result<String, String> {
    let at = evidence.find("0x").ok_or_else(|| {
        format!(
            "no buffer named in this evidence line, so the failure cannot be located in \
             the run:\n  {evidence}"
        )
    })?;
    let digits: String = evidence[at + 2..].chars().take_while(char::is_ascii_hexdigit).collect();
    if digits.len() != 16 {
        return Err(format!(
            "the evidence line names `0x{digits}`, which is not one of the sixteen-digit \
             identifiers a trace prints:\n  {evidence}"
        ));
    }
    Ok(digits)
}

/// Run a built `f-sim` [`SNAPSHOT_SAMPLES`] times and keep the fastest.
///
/// A minimum rather than a mean, and several samples rather than one, because
/// the number this verb reports is *what re-entering costs* and a single sample
/// on a shared four-core container measures the container as much as the code.
/// The first sample in particular pays for paging the binary in: a run of this
/// verb reported 85 ms for a re-entry that costs 8 ms warm, which is a tenfold
/// error on the smaller of the two numbers and enough to fail a threshold for a
/// reason that is not a regression.
///
/// A minimum is the right statistic for a cost floor, because noise on a shared
/// machine only ever adds. And every sample is required to produce the same
/// bytes, so the repetition is also a reproduction check rather than three
/// stopwatches: a command whose output moved between two runs is a determinism
/// failure and is reported as one, before any timing is reported at all.
fn f_sim_best(binary: &Path, args: &[&str]) -> Result<(bool, String, u128), String> {
    let (good, text, mut best) = f_sim_timed(binary, args)?;
    for _ in 1..SNAPSHOT_SAMPLES {
        let (again, more, ms) = f_sim_timed(binary, args)?;
        if again != good || more != text {
            return Err(format!(
                "two runs of one command answered differently. That is a reproduction failure \
                 and not a\n\
                 timing one, and it matters more than the number this verb was measuring:\n  \
                 {} {}",
                binary.display(),
                args.join(" ")
            ));
        }
        best = best.min(ms);
    }
    Ok((good, text, best))
}

/// One field out of an `f-sim` report, by the word it starts with.
fn field<'r>(report: &'r str, name: &str) -> Result<&'r str, String> {
    report
        .lines()
        .find(|line| line.starts_with(name))
        .map(|line| line[name.len()..].trim())
        .ok_or_else(|| format!("no `{name}` line in this report:\n{report}"))
}

/// A long run, marked as it goes, and re-entered one minute before it fails.
///
/// # What this verb is
///
/// **`E1-P08`'s exit, as a command.** A scenario that goes wrong in simulated
/// minute forty is run three ways and the three are compared:
///
/// 1. **replayed** from zero, which is what a person has today, and timed;
/// 2. **scanned** — the same run, with a snapshot written at every simulated
///    minute as it passes, and timed. This is the pass somebody was going to run
///    anyway, and the marks cost what they cost;
/// 3. **re-entered** from the minute-thirty-nine mark, and timed.
///
/// The third has to produce the second's ending exactly — the same digest, the
/// same finishing instant, the same step count — and to cost a small fraction of
/// the first. Both numbers are printed, because *bisects in seconds rather than
/// hours* is a claim about time and a claim about time that is not measured is a
/// hope.
///
/// # Why the run has to be built with a defect
///
/// Because a fault class in this tree **states its response and gets it**: RFC
/// 0039 is the whole of `E1-P02`, so injecting one produces a run that is
/// correct and a report that says nothing went wrong. To have a failure at
/// minute forty there has to be a bug at minute forty, and the honest way to
/// have one is the way RFC 0017 and RFC 0040 already established — a deliberate
/// defect in the shipped source, behind a feature that is off by default.
/// `mutate-silent-reset` is the one whose symptom needs a fall-over, and
/// `--inject peergone` is what places the fall-over in minute forty.
///
/// That the demonstration needs a defect at all is also what makes it a
/// demonstration: the same commit, without the feature, runs the same scenario
/// clean.
///
/// # And the two kinds of mark, because they answer different questions
///
/// A **whole** mark carries the artefact, so the re-entered run is
/// indistinguishable from the replay in every respect the oracle included — it
/// prints the same finding, with the same evidence. A **terse** mark carries the
/// artefact's running hash instead, so the re-entry is cheap and cannot be
/// judged. This verb requires both: the whole mark is what says *the same run*,
/// and the terse mark is what says *in a fraction of the time*. RFC 0043 argues
/// why one file format could not honestly be both.
fn snapshot() -> Result<(), String> {
    let commit = sweep_commit()?;
    let dir = root().join("target").join("snapshot");
    let _ = std::fs::remove_dir_all(&dir);
    // Two directories rather than one, because the two scans below write a file
    // of the same name — `minute-39.snap` — and the whole scan would otherwise
    // overwrite the terse mark that step five is written about. It did, and the
    // step still passed: the build fingerprint is refused before a body is
    // interpreted, so the refusal held while the file it named was the other
    // kind. A step whose subject is not what its name says is a step nobody can
    // read.
    let terse_dir = dir.join("terse");
    let whole_dir = dir.join("whole");
    let at = terse_dir.to_string_lossy().into_owned();
    let whole_at = whole_dir.to_string_lossy().into_owned();
    println!(
        "snapshot and restore — `{SNAPSHOT_SCENARIO}`, built with `{SWEEP_DEFECT_TWO}` so that\n\
         there is something to find at simulated minute {SNAPSHOT_MINUTE}. RFC 0043.\n"
    );

    // Built out of the timings, and both binaries built before either is timed:
    // everything below is measured, and a first invocation that also compiled
    // would report the compiler's seconds as the simulator's.
    println!("[0/5] building two simulators, with the defect and without (not timed)");
    let armed = f_sim_built(&[SWEEP_DEFECT_TWO], "f-sim-armed")?;
    let plain = f_sim_built(&[], "f-sim-plain")?;

    println!("\n[1/5] replaying the whole run — what a person has today");
    let (clean, replayed, replay_ms) =
        f_sim_timed(&armed, &["--check", "--inject", SNAPSHOT_STRIKE, SNAPSHOT_SCENARIO])?;
    if clean {
        return Err(format!(
            "`{SNAPSHOT_SCENARIO}` with `{SWEEP_DEFECT_TWO}` armed and a peer death at \
             {SNAPSHOT_STRIKE}\n\
             ran clean. There is then nothing at minute {SNAPSHOT_MINUTE} to re-enter and this \
             verb\n\
             demonstrates nothing. Either the defect no longer reaches the client — it needs a\n\
             fall-over, so a scenario that stopped resetting would hide it — or the scenario's\n\
             numbers moved and the strike no longer lands where it did."
        ));
    }
    let finding = field(&replayed, "check")?.to_string();
    let evidence = field(&replayed, "evidence")?.to_string();
    let failing = failing_buffer(&evidence)?;
    println!("       {replay_ms} ms");

    // The digest of that same run, which is the number a re-entry has to answer.
    let (_, hashed, _) =
        f_sim_timed(&armed, &["--hash", "--inject", SNAPSHOT_STRIKE, SNAPSHOT_SCENARIO])?;
    let digest = hashed.trim().to_string();
    let (_, reported, replay_hash_ms) =
        f_sim_best(&armed, &["--inject", SNAPSHOT_STRIKE, SNAPSHOT_SCENARIO])?;
    let finished: u64 = field(&reported, "finished")?
        .trim_end_matches(" ns")
        .parse()
        .map_err(|_| "the report's `finished` line is not a number".to_string())?;
    let minute = finished / 60_000_000_000;
    if minute != SNAPSHOT_MINUTE {
        return Err(format!(
            "the run goes wrong in simulated minute {minute}, and this verb is written about\n\
             minute {SNAPSHOT_MINUTE} — which is the minute `E1-P08`'s exit names. Move\n\
             `SNAPSHOT_STRIKE` until it lands there again, or change the exit."
        ));
    }
    println!(
        "       fails in simulated minute {minute} — {finding}\n       digest {digest}, {} steps",
        field(&reported, "steps")?
    );

    println!("\n[2/5] the same run, marked at every simulated minute — terse marks");
    let (_, scanned, scan_ms) = f_sim_timed(
        &armed,
        &[
            "--scan",
            "--terse",
            "--into",
            &at,
            "--every",
            "1",
            "--commit",
            &commit,
            "--inject",
            SNAPSHOT_STRIKE,
            SNAPSHOT_SCENARIO,
        ],
    )?;
    if field(&scanned, "digest")? != digest {
        return Err("marking the run changed it. A scan that moved a digest is a scan whose \
                    marks describe a run nobody had."
            .to_string());
    }
    let terse = terse_dir.join(format!("minute-{}.snap", SNAPSHOT_MINUTE - 1));
    let terse_bytes = std::fs::metadata(&terse).map(|m| m.len()).unwrap_or_default();
    println!("       {scan_ms} ms, and minute {} is {terse_bytes} bytes", SNAPSHOT_MINUTE - 1);

    println!("\n[3/5] re-entering at minute {} — terse", SNAPSHOT_MINUTE - 1);
    let (_, resumed, resume_ms) =
        f_sim_best(&armed, &["--resume", &terse.to_string_lossy(), "--commit", &commit])?;
    if field(&resumed, "digest")? != digest {
        return Err(format!(
            "a run re-entered at minute {} ended with a different digest from the run that\n\
             replayed. That is the one failure a snapshot must not have: it is a plausible\n\
             run that diverged, and it would send somebody looking for a bug at a point the\n\
             system never reached. RFC 0043.",
            SNAPSHOT_MINUTE - 1
        ));
    }
    if field(&resumed, "finished")? != field(&reported, "finished")?
        || field(&resumed, "steps")? != field(&reported, "steps")?
    {
        return Err("a re-entered run ended at a different instant or after a different number \
                    of steps."
            .to_string());
    }

    // And the tail is *read*, not merely hashed. A terse mark carries its prefix
    // as a number, so `check::examine` refuses to judge it — which left the
    // gated fast path unable to show anybody a bug, and left this verb unable to
    // say the failure was after the cut rather than before it. `--resume
    // --trace` prints the records from the cut onward, and the buffer the replay
    // failed on has to be among them.
    //
    // Untimed, deliberately: printing half a million lines is not part of what a
    // re-entry costs, and folding it into the number above would deflate the
    // saving this verb reports.
    let (_, tail, _) = f_sim_timed(
        &armed,
        &["--resume", &terse.to_string_lossy(), "--commit", &commit, "--trace"],
    )?;
    if !tail.contains(&failing) {
        return Err(format!(
            "the run fails on 0x{failing}, and that is nowhere in the tail re-entered at\n\
             minute {}. Either the failure is *before* the cut — in which case this verb\n\
             demonstrates re-entering a run whose bug it skipped, and the minute in its own\n\
             sentence is wrong — or the tail is not this run's. Move `SNAPSHOT_STRIKE` until\n\
             the failure lands after the cut again.",
            SNAPSHOT_MINUTE - 1
        ));
    }
    println!("       {resume_ms} ms, same digest {digest}, and the tail holds 0x{failing}");

    println!("\n[4/5] the same re-entry from a whole mark, which the oracle can judge");
    let (_, _, whole_scan_ms) = f_sim_timed(
        &armed,
        &[
            "--scan",
            "--into",
            &whole_at,
            "--every",
            "1",
            // Only the last few minutes are marked, and only two are kept: a
            // whole mark carries the artefact, so forty of them is forty copies
            // of a growing run and about a gigabyte of writing to throw away.
            // What that gives up is stated where it is chosen — a bisect that
            // wants an earlier mark scans again, and a scan is a function of
            // (seed, commit).
            "--after",
            "38",
            "--keep",
            "2",
            "--commit",
            &commit,
            "--inject",
            SNAPSHOT_STRIKE,
            SNAPSHOT_SCENARIO,
        ],
    )?;
    let whole = whole_dir.join(format!("minute-{}.snap", SNAPSHOT_MINUTE - 1));
    let whole_bytes = std::fs::metadata(&whole).map(|m| m.len()).unwrap_or_default();
    let (_, judged, whole_resume_ms) =
        f_sim_best(&armed, &["--resume", &whole.to_string_lossy(), "--commit", &commit])?;
    if field(&judged, "check")? != finding || field(&judged, "evidence")? != evidence {
        return Err(format!(
            "a run re-entered from a whole mark was judged differently from the run that\n\
             replayed.\n  replayed: {finding}\n            {evidence}\n  re-entered: {}\n\
             \x20           {}",
            field(&judged, "check")?,
            field(&judged, "evidence")?
        ));
    }
    println!(
        "       {whole_scan_ms} ms to scan, {whole_resume_ms} ms to re-enter, mark is \
         {whole_bytes} bytes"
    );
    println!("       and the same finding: {finding}");

    println!("\n[5/5] a mark from another build is refused rather than read");
    // The same commit, a different binary: no defect. The snapshot's build
    // fingerprint folds the compiled-in defects, so this is the case a commit
    // hash alone cannot catch — and it is the case that would otherwise restore
    // a run into a model that means something else.
    let refused = f_sim_timed(&plain, &["--resume", &terse.to_string_lossy(), "--commit", &commit]);
    match refused {
        Err(why) if why.contains("snapshot build") => println!("       refused: {}", why.trim()),
        Err(why) => {
            return Err(format!(
                "a snapshot from a build with `{SWEEP_DEFECT_TWO}` in it was refused by a build\n\
                 without it, but not for the right reason: {why}"
            ));
        }
        Ok(_) => {
            return Err(format!(
                "a binary built without `{SWEEP_DEFECT_TWO}` read a snapshot taken by one built\n\
                 with it. Those are two different models, so the run it would continue is a run\n\
                 nobody had. The build fingerprint in sim/src/snap.rs is meant to refuse this."
            ));
        }
    }

    let saving = replay_hash_ms.max(1) / resume_ms.max(1);
    println!(
        "\nsnapshot: ok — a failure in simulated minute {SNAPSHOT_MINUTE} was re-entered at \
         minute {},\n\
        \x20         without re-running the first {}.\n\n\
        \x20  replay from zero            {replay_hash_ms:>6} ms\n\
        \x20  scan, marking every minute  {scan_ms:>6} ms   (paid once, and this run was \
         happening anyway)\n\
        \x20  re-enter at minute {:<9}{resume_ms:>6} ms   ({saving}x)\n\
        \x20  terse mark                  {terse_bytes:>6} bytes\n\
        \x20  whole mark                  {whole_bytes:>6} bytes, {whole_resume_ms} ms to \
         re-enter, and judgeable\n\n\
        \x20         The ratio above is the *terse* mark's, and a terse mark is a bisect tool\n\
        \x20         rather than a verdict: `--resume --trace` reads its tail — which holds\n\
        \x20         0x{failing}, the buffer this run failed on — while `--check` refuses\n\
        \x20         to judge a partial artefact and says so. The whole mark is the judgeable\n\
        \x20         one and is **not** a saving: it costs about what the replay costs, so it\n\
        \x20         is gated by a ceiling of {SNAPSHOT_WHOLE_CEILING}x rather than a floor, \
         which is what makes a\n\
        \x20         regression in it visible. RFC 0043 says why one file could not be both\n\
        \x20         and claims/0007 carries both numbers.",
        SNAPSHOT_MINUTE - 1,
        SNAPSHOT_MINUTE - 1,
        SNAPSHOT_MINUTE - 1,
    );
    if saving < SNAPSHOT_SAVING {
        return Err(format!(
            "re-entering cost {resume_ms} ms against {replay_hash_ms} ms to replay, which is \
             {saving}x and\n\
             the threshold is {SNAPSHOT_SAVING}x. A re-entry that costs the same order as the \
             replay it\n\
             exists to avoid is not a bisect tool. claims/0007 is the claim and RFC 0043 is\n\
             where the two designs are measured against each other."
        ));
    }
    if whole_resume_ms > replay_hash_ms.max(1).saturating_mul(SNAPSHOT_WHOLE_CEILING) {
        return Err(format!(
            "re-entering from a whole mark cost {whole_resume_ms} ms against \
             {replay_hash_ms} ms to replay,\n\
             which is past the {SNAPSHOT_WHOLE_CEILING}x ceiling. A whole mark is not \
             expected to be a saving — it\n\
             is the artefact the oracle can judge — but it is expected to stay in the same \
             order as\n\
             the run it stands in for, and it no longer is. RFC 0043 is where the two \
             designs are\n\
             measured against each other."
        ));
    }
    Ok(())
}

/// The `sweep` verb, and the one argument it owns that is not N or M.
///
/// `--base <seed>` is pulled out here rather than taken positionally, because it
/// is the argument a nightly varies and the two positional ones are the argument
/// a person varies. Everything downstream takes the base explicitly: a sweep
/// whose base was implicit would be a report nobody could reproduce from its own
/// header, which is the argument [`TRACE_SEED`] already carries one level down.
fn sweep_verb(args: &[String]) -> Result<(), String> {
    let mut base: Option<String> = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        if arg == "--base" {
            let value = walk.next().ok_or("--base needs a seed: 0x-prefixed hex, or decimal")?;
            base = Some(value.clone());
        } else {
            rest.push(arg.as_str());
        }
    }
    let base = base.as_deref().unwrap_or(TRACE_SEED);

    match rest.first().copied() {
        // Before every other arm, because the one thing a stranger types when a
        // command is unfamiliar used to land in the `unknown option` arm below
        // and answer a question about the tool with a complaint about the
        // argument. E1-R01, RFC 0055.
        Some("--help" | "-h") => sweep_help(),
        Some("--mutate") => sweep_mutate(),
        Some("--corpus") => sweep_corpus(&[]),
        // `--record --mutate` is how the entries already in the corpus were
        // produced, and it is spelled rather than left as folklore: the corpus
        // is the trials that have found something, and on a tree with nothing
        // wrong with it the only thing to find is the deliberate defect.
        // Whoever regenerates the file runs this.
        Some("--record") => match rest.get(1).copied() {
            Some("--mutate") => sweep_record(base, None, &[SWEEP_DEFECT]),
            seeds => sweep_record(base, seeds, &[]),
        },
        Some(other) if other.starts_with('-') => Err(format!(
            "unknown option for sweep: {other}\n\n\
             `cargo xtask sweep --help` is the forms, what a seed is, and what to do \
             with a finding."
        )),
        seeds => sweep(base, seeds, rest.get(1).copied(), &[], false),
    }
}

/// What a sweep is, for somebody who has just cloned this.
///
/// # Why this is help text and not a page in `docs/`
///
/// Because `RELEASING.md` ships the seed corpus and the scenario set as one of
/// eight contents, and the exit this pays is *a third party runs a seed sweep
/// against their own checkout using the published command*. A stranger who
/// reaches for a command reaches for its `--help` before they reach for a
/// directory of Markdown; a document that answers this would be a second account
/// of the same tables, and the second account is the one that goes stale. So the
/// question *what does this tool do and what do I do with what it says* is
/// answered by the tools themselves, here and in `f-sim`'s own usage. RFC 0055.
///
/// # Why it does not print the scenario set
///
/// Because the scenario set is `f_sim::scenario::SCENARIOS` and this crate does
/// not link `f-sim` — it runs it. Rendering the table here would mean either a
/// second copy that drifts from the shipped one, or a `--help` that compiles a
/// binary before it can answer, and a help text with a build in it is one nobody
/// waits for. It names the two commands that print the table from the table
/// instead.
fn sweep_help() -> Result<(), String> {
    println!(
        "\
cargo xtask sweep — N seeds across M scenarios, and every failure minimised

  A sweep runs the simulator over a grid of (scenario, seed) pairs, checks each
  run against five properties that name no scenario and no defect, and reduces
  anything that fails to the smallest reproduction that keeps the same
  signature. What it hands back is a command line, not a symptom. RFC 0040.

the forms

  cargo xtask sweep              {SWEEP_SEEDS} seeds against every scenario in the table
  cargo xtask sweep <n>          n seeds
  cargo xtask sweep <n> <m>      n seeds against the first m scenarios
  cargo xtask sweep --base <s>   the seed the whole derivation starts from;
                                 the default is this tree's own, {TRACE_SEED}
  cargo xtask sweep --corpus     replay every trial that has ever found
                                 something, and require each to be clean
  cargo xtask sweep --mutate     arm a deliberate defect and require the sweep
                                 and the corpus to find it, then disarm it and
                                 require both to go quiet
  cargo xtask sweep --record [n] sweep, and merge what it finds into
                                 sim/corpus.txt
  cargo xtask sweep --record --mutate
                                 how the entries in sim/corpus.txt were produced

  A grid too large for one process is run as consecutive shards of the same seed
  derivation. That is a fact about memory rather than about coverage: shard k
  runs exactly the trials one process would have run at those indices. RFC 0042.

what a seed is

  The whole of a run's nondeterminism. A run is a function of (seed, commit) —
  every ordering, every arrival and every injected fault is drawn from the seed
  through `f_env::Env`, so the same pair on another machine produces the same
  bytes. That is what makes a finding a thing you can send somebody rather than
  describe to them. RFC 0004.

  The commit half is worth what the tree behind it is worth, and a commit is not
  a tree: `git rev-parse HEAD` names what is committed and says nothing about
  what you have changed since. This verb asks git both questions and tells the
  simulator, which prints the answer on the report's `tree` line and shapes its
  reproduction lines to match. A sweep of a modified checkout is a perfectly
  good way to find a bug — it is how you would find one in work you are doing —
  and it is not a bug report anybody else can run until the changes are
  committed.

what the scenario set is, printed from the table rather than from a document

  cargo xtask sim --list         the name and one line each
  cargo run -q -p f-sim -- --help
                                 the same, plus the long scenarios, the five
                                 properties, and what to do with a finding

  sim/corpus.txt's header is the same set, regenerated from the same table on
  every write, so a list of scenarios in a comment cannot stop matching.

what a finding looks like, and what to do with one

  finding 1  blk / held
    property   a client is never told about a token it does not hold
    evidence   ...
    seed       0x...  (seed 37 of the sweep)
    repro      git switch --detach <commit> && cargo run -q -p f-sim -- --check ...
    minimised  ...
    artefact   the same line with `--check` replaced by `--trace`

  The `git switch` half appears when the sweep ran in a tree that was exactly
  its commit, and is absent when it was not. That is not cosmetic: checking a
  commit out to run a line found in a tree that is not that commit discards the
  changes that found it and then runs a different program, so on a modified
  checkout the report prints the bare command — which reproduces where you
  already are — and says so on the finding's `tree` line.

  1. Paste the repro line. It judges itself: it runs `--check`, which exits
     non-zero and names the property that broke. Commit before sending one
     anywhere, or it names a program the receiver cannot build.
  2. Swap `--check` for `--trace` to read the artefact behind the verdict.
  3. Keep it: append the argument list to sim/corpus.txt, or run
     `cargo xtask sweep --record`. The file is append-only and
     `cargo xtask sweep --corpus` replays all of it, which is what turns one
     seed into a permanent regression test.

what it costs, and what it cannot catch

  The report prints its own wall clock and that number is in no verdict in it.
  A clean sweep is worth exactly what the oracle is worth, which is why
  `--mutate` exists: three of the five properties are falsifiable end to end by
  a defect in the shipped source, and `intact` and `clock` are not. RFC 0042
  states the count rather than leaving it to be discovered.

running this against your own checkout, with Docker as the only prerequisite

  docker compose -f docker/compose.yaml run --rm dev cargo xtask sweep

  README.md, \"Sweeping your own checkout\", is the whole route.
"
    );
    Ok(())
}

/// The largest `--seeds` one `f-sim` process will accept over `scenarios`
/// scenarios. Unit: seeds.
///
/// Asked of `f-sim` rather than computed here, and that is the point: the leak
/// this bounds belongs to `sim/src/client.rs`, the arithmetic that turns it into
/// a seed count is `f_sim::sweep::max_seeds`, and a second copy in this file
/// would be a number that drifts the first time a buffer geometry in the
/// scenario table changes. RFC 0042.
fn sweep_ceiling(scenarios: Option<&str>) -> Result<u32, String> {
    let mut argv: Vec<&str> = vec!["run", "-q", "-p", "f-sim", "--", "--ceiling"];
    if let Some(scenarios) = scenarios {
        argv.push("--scenarios");
        argv.push(scenarios);
    }
    let out = capture("cargo", &argv)?;
    out.trim()
        .parse::<u32>()
        .map_err(|_| format!("f-sim --ceiling printed `{}`, which is not a seed count", out.trim()))
}

/// The sweep, with the wall clock around it.
///
/// `seeds` and `scenarios` are the N and the M: how many seeds, and how many
/// scenarios from the top of the shipped table. Both have defaults, because a
/// command with no default is a command with a manual.
///
/// # Why this loops
///
/// Every trial in `f-sim` leaks the buffer regions of the clients it ran, so one
/// process can hold a bounded number of them and `f-sim` refuses a grid past
/// that bound rather than being killed for memory half way through a night. The
/// nightly asks for sixty-five thousand seeds, which is several times the bound,
/// so this runs it as consecutive shards of the *same* seed derivation — shard
/// `k` runs exactly the trials one process would have run at those indices, and
/// each report says which indices it covered. `sim/src/sweep.rs` has the test
/// that says the shards together are the sweep; RFC 0042 is the record.
///
/// One shard is the ordinary case and prints what this printed before: the
/// default sweep is 64 seeds against a ceiling in the thousands.
/// `record` merges what is found into `sim/corpus.txt` before returning, and
/// `features` is how the mutation harness arms a defect. Both are arguments
/// rather than separate functions because the sharding, the ceiling and the
/// verdict are the same in every case, and two copies of a loop that decides a
/// gate is two places for a shard to go missing.
fn sweep(
    base: &str,
    seeds: Option<&str>,
    scenarios: Option<&str>,
    features: &[&str],
    record: bool,
) -> Result<(), String> {
    components_quietly()?;
    let commit = sweep_commit()?;
    // Measured once and passed to every shard, so that a tree edited half way
    // through a long sweep cannot make one shard's reproduction lines a
    // different shape from another's. One report, one answer.
    let tree = sweep_tree();
    let jobs = sweep_jobs();
    let dir = component_dir()?;
    let wanted: u32 = match seeds {
        None => SWEEP_SEEDS,
        Some(text) => text.parse().map_err(|_| format!("sweep takes a count, not `{text}`"))?,
    };
    // Fail closed here as well as in `f-sim`, so that the verb refuses before it
    // builds anything. A sweep of no seeds is not a small sweep.
    if wanted == 0 {
        return Err("sweep 0 asks for a grid with no trials in it, which is a result that is \
                    green because it asserted nothing. R04."
            .to_string());
    }
    // The other axis of the same fail-open, refused here rather than left to
    // `f-sim`: a zero reaching `--ceiling` makes that call fail with a message
    // about a subprocess, which is a true statement about the wrong thing.
    if let Some(text) = scenarios {
        let count: u32 =
            text.parse().map_err(|_| format!("sweep takes a scenario count, not `{text}`"))?;
        if count == 0 {
            return Err("sweep <n> 0 asks for a grid with no scenarios in it, which is a \
                        result that is green because it asserted nothing. R04."
                .to_string());
        }
    }
    let ceiling = sweep_ceiling(scenarios)?;
    let shards = wanted.div_ceil(ceiling.max(1));

    // The one clock in this apparatus, and it is here rather than in the
    // simulator on purpose: a sweep nobody can finish is a sweep nobody runs, so
    // the cost has to be reported — and a cost that could reach a verdict would
    // make two machines disagree about what a commit does. `sim/src/sweep.rs`
    // states the same split from the other side.
    let started = std::time::Instant::now();
    let mut findings = 0u32;
    for shard in 0..shards {
        let from = shard.saturating_mul(ceiling);
        let take = wanted.saturating_sub(from).min(ceiling);
        let (from, take) = (from.to_string(), take.to_string());
        if shards > 1 {
            println!("\n--- shard {} of {shards}\n", shard + 1);
        }
        let mut args: Vec<&str> = vec![
            if record { "--record" } else { "--sweep" },
            "--commit",
            &commit,
            "--seed",
            base,
            "--seeds",
            &take,
            "--from",
            &from,
            "--jobs",
            &jobs,
            "--tree",
            tree,
            "--components",
            &dir,
        ];
        if let Some(scenarios) = scenarios {
            args.push("--scenarios");
            args.push(scenarios);
        }
        let (clean, _) = f_sim(features, &args)?;
        if !clean {
            findings = findings.saturating_add(1);
        }
    }
    let elapsed = started.elapsed();

    println!(
        "\nelapsed    {:.1} s of wall clock at {jobs} worker(s) over {shards} process(es),\n\
         \x20          and it is in no verdict above. Two machines that disagree about\n\
         \x20          this number still agree about every line of the report.",
        elapsed.as_secs_f64()
    );
    if findings == 0 {
        return Ok(());
    }
    Err(format!(
        "the sweep found something, in {findings} of {shards} shard(s). Every finding above \
         carries the one line that reproduces it, and that line judges itself: it runs \
         `--check`, which exits non-zero and names the property that broke."
    ))
}

/// The corpus, replayed.
///
/// Every trial that has ever found something, required to be clean now. That is
/// what makes `sim/corpus.txt` a regression suite rather than a list of numbers,
/// and it is the half of `E1-P03` that keeps paying after the sweep that found
/// an entry has been forgotten.
fn sweep_corpus(features: &[&str]) -> Result<(), String> {
    components_quietly()?;
    let dir = component_dir()?;
    let (clean, _) = f_sim(features, &["--corpus", "--components", &dir])?;
    if clean {
        return Ok(());
    }
    Err("a corpus entry that used to be clean is not.\n\n\
         Each `[--]` line above is an argument list: paste it after \
         `cargo run -q -p f-sim -- --trace` and read the artefact."
        .to_string())
}

/// Sweep, and merge what it found into the corpus.
///
/// A wrapper rather than a second loop, because a corpus-growing sweep is the
/// same sweep: the same grid, the same ceiling, the same shards and the same
/// verdict. What differs is one flag to `f-sim` and one consequence in the
/// tree.
///
/// **It still fails when it finds something**, and that is the change the
/// nightly needed: growing the corpus and going red are two things a scheduled
/// job has to do in one pass, and a `--record` that swallowed the verdict was a
/// job that could only do one of them per run of a night. Whoever regenerates
/// the corpus by hand under a deliberate defect gets entries written *and* a
/// non-zero exit, which is the honest answer — the sweep did find something.
fn sweep_record(base: &str, seeds: Option<&str>, features: &[&str]) -> Result<(), String> {
    sweep(base, seeds, None, features, true)
}

/// The argument list out of the smallest reproduction a report printed.
///
/// The `smallest` line when the minimiser shrank anything and the `repro` line
/// when it did not, because both are reproductions and only one of them exists
/// in the second case. Everything before ` -- ` is the `git switch` half and the
/// `cargo run` invocation, which this file supplies itself; everything after is
/// what `f-sim` is asked, which is the part under test.
///
/// Fail closed: a report with no reproduction line in it is a harness failure
/// and not a zero-length argument list, because an empty argv would run the
/// default scenario at the default seed and pass.
fn replayable(report: &str) -> Result<Vec<String>, String> {
    let line = report
        .lines()
        .find(|line| line.starts_with("  smallest   "))
        .or_else(|| report.lines().find(|line| line.starts_with("  repro      ")))
        .ok_or("the report carries neither a `repro` nor a `smallest` line")?;
    let args = line
        .split_once(" -- ")
        .map(|(_, rest)| rest)
        .ok_or_else(|| format!("`{line}` is not a command line this file can replay"))?;
    let argv: Vec<String> = args.split_whitespace().map(str::to_string).collect();
    if argv.is_empty() {
        return Err(format!("`{line}` names no arguments"));
    }
    Ok(argv)
}

/// Every check a report says fired, in the order the report lists them.
///
/// Read off the `finding N  <scenario> / <check>` headings, which is the one
/// place a report states a signature. A parser rather than a substring search
/// because the question asked of it is *which* checks fired and not *whether a
/// name appears* — `held` appears in the sentence `balance` prints, and a
/// harness that matched on that would accept the failure it exists to refuse.
fn signatures_in(report: &str) -> Vec<String> {
    report
        .lines()
        .filter(|line| line.starts_with("finding "))
        .filter_map(|line| line.rsplit_once(" / ").map(|(_, check)| check.trim().to_string()))
        .collect()
}

/// The mutation harness for the sweep: arm a deliberate defect, require the
/// sweep to find it, disarm it, require the sweep to be quiet.
///
/// # Why the sweep needs this, and why the boot's harness is the precedent
///
/// A sweep that has only ever printed *clean* is indistinguishable from a sweep
/// that cannot print anything else, and the ways it could be that are all cheap
/// mistakes: an oracle whose checks are vacuous, a grid that runs one trial, a
/// verdict that is discarded. `cargo xtask mutate` makes the same argument about
/// the boot suite and RFC 0017 is where it is written down; this is that
/// argument applied to the layer `E1-P03` builds, and RFC 0040 records the
/// extension.
///
/// # What is required, in order
///
/// 1. **The sweep goes red with the defect armed**, and the report has to carry
///    a minimised reproduction — not just a failure count, because the exit
///    criterion is about the reproduction, and a harness that accepted *red*
///    would pass on a sweep that found something and could say nothing about it.
/// 2. **The corpus goes red with the defect armed.** This is the half that stops
///    the corpus being decoration: a regression suite whose entries have never
///    been seen to fail is a file of command lines nobody has tested.
/// 3. **The corpus goes green with the defect disarmed**, which is what a
///    regression suite claims.
/// 4. **The sweep goes green with the defect disarmed**, because a red sweep on
///    a broken build is a broken build rather than a caught defect — the pair
///    `mutate` insists on, for the same reason.
///
/// The armed half runs first so the tree is left holding a clean build.
fn sweep_mutate() -> Result<(), String> {
    components_quietly()?;
    let commit = sweep_commit()?;
    let tree = sweep_tree();
    let jobs = sweep_jobs();
    let seeds = MUTATE_SEEDS.to_string();
    let dir = component_dir()?;
    let armed: &[&str] = &[SWEEP_DEFECT];
    let sweeping: Vec<&str> = vec![
        "--sweep",
        "--commit",
        &commit,
        "--seed",
        TRACE_SEED,
        "--seeds",
        &seeds,
        "--jobs",
        &jobs,
        "--tree",
        tree,
        "--components",
        &dir,
    ];
    // What a reproduction line has to start with, which is a function of the
    // tree this is running in and not a constant. On a committed tree the line
    // begins with the checkout, and requiring that is what says the report can
    // be sent to somebody else; on a modified tree `f-sim` deliberately omits
    // it, because checking that commit out would discard the changes under
    // test. Requiring the checkout unconditionally would have made this harness
    // fail on every tree anybody develops in, and requiring nothing would have
    // stopped it asserting the half of E1-P03 that matters. RFC 0055.
    let starts = if sweep_dirty() { "cargo run" } else { "git switch --detach" };

    println!(
        "sweep mutation harness — two defects, both in sim/src/dev.rs:\n\
        \x20 `{SWEEP_DEFECT}`, which trips `held`, and\n\
        \x20 `{SWEEP_DEFECT_TWO}`, which trips a different check.\n\
         Two rather than one because five properties with one defect between them is one\n\
         property under test and four decorations. RFC 0042.\n"
    );

    println!("[1/5] the first defect — the sweep must find it and minimise it");
    let (clean, report) = f_sim(armed, &sweeping)?;
    if clean {
        return Err(format!(
            "the sweep found nothing on a simulator built with `{SWEEP_DEFECT}`.\n\n\
             That means this sweep cannot fail, which makes every green result it has\n\
             ever printed worth nothing. Either the defect is no longer reached — it\n\
             needs two consecutive coalescing decisions, so a scenario set that stopped\n\
             coalescing would hide it — or the oracle in sim/src/check.rs has stopped\n\
             reading what the run wrote."
        ));
    }
    // The exit criterion is *a reproduction command, with no human triage*, so
    // the harness asserts on the report's shape rather than on its exit status.
    // A sweep that went red and printed a count would satisfy the status and
    // prove nothing about the deliverable.
    //
    // `smallest` is the one that would otherwise be easy to lose: a minimiser
    // that accepted no candidate still prints a `minimised` line and still calls
    // its answer 1-minimal, truthfully, because a trial nothing can shrink is
    // 1-minimal. Requiring the shrunken command line is what says shrinking
    // *happened* rather than that it was attempted.
    for wanted in [
        format!("repro      {starts}"),
        format!("smallest   {starts}"),
        "minimised  ".to_string(),
        "1-minimal against the move".to_string(),
    ] {
        if !report.contains(&wanted) {
            return Err(format!(
                "the sweep went red and its report has no `{wanted}` line in it.\n\n\
                 Finding a failure is half of E1-P03; the other half is that what comes\n\
                 out is a command rather than a symptom. The report is above."
            ));
        }
    }
    if report.contains("DID NOT REPRODUCE TWICE") {
        return Err("a minimised failure did not reproduce, so the command the sweep printed \
                    is not a bug report. RFC 0004."
            .to_string());
    }

    // And the sweep run a second time, in a second process, required to produce
    // the same bytes. This is the claim the exit criterion actually rests on —
    // *two machines running one sweep find the same failures* — asserted in the
    // one shape available on one machine, and it is the same shape `sim_check`
    // uses for a scenario and `trace_check` uses for a boot. It is asserted
    // against the **armed** sweep on purpose: a sweep that found nothing agrees
    // with itself trivially, so the comparison is made where there is something
    // to disagree about.
    let (_, again) = f_sim(armed, &sweeping)?;
    if again != report {
        let differs = report
            .lines()
            .zip(again.lines())
            .position(|(a, b)| a != b)
            .map_or_else(|| "in its length".to_string(), |line| format!("at line {}", line + 1));
        return Err(format!(
            "two processes ran the same sweep and reported differently, {differs}.\n\n\
             A sweep whose findings depend on the machine is a sweep whose `(seed, commit)`\n\
             pairs mean nothing to whoever receives them. Something in the grid, the\n\
             grouping or the minimiser is reading an address, an iteration order or a\n\
             clock the arguments do not own — RFC 0004, RFC 0040."
        ));
    }
    println!("\n{SWEEP_DEFECT}: and two processes reported it identically");
    let summary = report
        .lines()
        .find(|line| line.contains("distinct check(s)"))
        .unwrap_or("(the summary line moved)");
    println!("\n{SWEEP_DEFECT}: caught — {}", summary.trim());

    // And the printed line, executed. Everything above asserts on the *shape* of
    // the report — that a `smallest` line is there and starts the way a
    // reproduction starts — and a shape is not a reproduction: a change that
    // made `Trial::argv` emit a flag `f-sim` does not accept, or drop a field
    // that narrowed the trial, would leave every assertion above green while the
    // command in the report replayed a different run or none at all. So the
    // command the sweep just printed is taken off the report and run, twice.
    let smallest = replayable(&report)?;
    let (clean, _) = f_sim(armed, &smallest.iter().map(String::as_str).collect::<Vec<_>>())?;
    if clean {
        return Err(format!(
            "the sweep printed a minimised reproduction and it does not reproduce.\n\n\
             `{}`\n\
             exits zero on a simulator built with `{SWEEP_DEFECT}`. A report whose\n\
             reproduction command does not reproduce is a symptom with a command line\n\
             attached, which is the thing E1-P03 exists not to produce.",
            smallest.join(" ")
        ));
    }
    // The control beside it: the same line without the defect has to be quiet,
    // or what it reproduces is not the defect.
    let (clean, _) = f_sim(&[], &smallest.iter().map(String::as_str).collect::<Vec<_>>())?;
    if !clean {
        return Err(format!(
            "the minimised reproduction fails on a tree with no defect in it.\n\n\
             `{}`\n\
             is red either way, so the red half of this harness says nothing about\n\
             `{SWEEP_DEFECT}`. Fix the tree first — and this is a real finding.",
            smallest.join(" ")
        ));
    }
    println!(
        "\n{SWEEP_DEFECT}: and the line it printed judges itself, run twice:\n\
        \x20 `{}`\n\
        \x20 exits non-zero with the defect armed and zero without it",
        smallest.join(" ")
    );

    // The second defect, and the one thing it is here to say: a *different*
    // check fires. `mutate-crossed-completion` trips the first entry in the
    // oracle's table, which is the only signature it can produce, so a harness
    // built on it alone leaves the other four properties in the state review
    // found them in — never observed to fail on a run of the models, only on a
    // hand-built record vector. RFC 0042.
    println!("\n[2/5] the second defect — a different check must fire");
    let (clean, second) = f_sim(&[SWEEP_DEFECT_TWO], &sweeping)?;
    if clean {
        return Err(format!(
            "the sweep found nothing on a simulator built with `{SWEEP_DEFECT_TWO}`.\n\n\
             That defect withholds the reset notification a device owes its client, so a\n\
             client is left holding buffers nothing will answer. If no scenario reaches a\n\
             fall-over any more, this defect is unreachable and a new one is owed; if one\n\
             does, the oracle has stopped reading what the run wrote."
        ));
    }
    let fired = signatures_in(&second);
    if !fired.iter().any(|check| SWEEP_DEFECT_TWO_CHECKS.contains(&check.as_str())) {
        return Err(format!(
            "`{SWEEP_DEFECT_TWO}` was found, but by {fired:?} rather than by any of\n\
             {SWEEP_DEFECT_TWO_CHECKS:?}.\n\n\
             The point of a second defect is that a second property is shown to fail on a\n\
             run rather than on a forged trace. If the check that fires has legitimately\n\
             changed, change the list here and say so in RFC 0042 — do not widen it to\n\
             whatever fired."
        ));
    }
    if !second.contains(&format!("repro      {starts}")) {
        return Err("the second defect was found and the report carries no reproduction \
                    line for it."
            .to_string());
    }
    println!("\n{SWEEP_DEFECT_TWO}: caught by {fired:?}, which is not `held`");

    println!("\n[3/5] the first defect — the corpus must go red");
    let (corpus_armed, _) = f_sim(armed, &["--corpus", "--components", &dir])?;
    if corpus_armed {
        return Err(format!(
            "every corpus entry stayed clean on a simulator built with `{SWEEP_DEFECT}`.\n\n\
             The corpus is the trials that found this, kept so that they keep finding it.\n\
             If none of them does, the file is a list of command lines nobody has tested —\n\
             which is the failure sim/corpus.txt exists to prevent.\n\
             `cargo xtask sweep --record` is what puts entries in it."
        ));
    }
    println!("\n{SWEEP_DEFECT}: the corpus catches it too");

    println!("\n[4/5] without it — the corpus must go green");
    sweep_corpus(&[])?;

    println!("\n[5/5] without it — the sweep must go quiet");
    let (clean, _) = f_sim(&[], &sweeping)?;
    if !clean {
        return Err("the sweep finds something on a simulator with no defect in it, so the red\n\
             result above says nothing about the defect. Fix the tree first — and the\n\
             finding above is a real one, with a reproduction command already written."
            .to_string());
    }

    println!(
        "\nsweep --mutate: ok — the sweep and the corpus each go red on `{SWEEP_DEFECT}`\n\
        \x20              and green without it; the line the red half printed was run and\n\
        \x20              exits non-zero armed and zero disarmed; and `{SWEEP_DEFECT_TWO}`\n\
        \x20              is found by a different check, so the oracle is more than one\n\
        \x20              property wide."
    );
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

/// Where the checker's crate is.
///
/// Outside the workspace, because it is compiled by a toolchain this tree does
/// not pin — RFC 0022's decision, applied a second time and to a tool that
/// takes it further than RustMC did: `cargo kani setup` installs a rustc of its
/// own. Nothing `cargo xtask verify` runs builds this directory, which is what
/// makes deleting it the whole of undoing the arrangement.
const PROOFS: &str = "kernel/proofs";

/// Every harness `prove` requires, and the sentence each one is.
///
/// The list is here rather than left to whatever the crate happens to contain,
/// for the reason `PROVOCATIONS` is: a proof that stopped being compiled would
/// otherwise take a whole property with it and the run would still say
/// `SUCCESSFUL`. `prove` runs them by name, so a harness that went missing is a
/// red run rather than a shorter green one.
///
/// The middle field is **run this one again at the wider bound**. It was
/// *does this harness grow a table*, negated, until `ring/proofs` arrived and
/// the two crates wanted opposite answers from the same question: the kernel's
/// wide pass re-runs the harnesses whose cost the page size does *not* touch,
/// to show the reduction binds only `total_bought`, and the ring's re-runs
/// exactly the harnesses that *do* read the fixture, because the ones that take
/// an argument list never see the region at all and running those twice would
/// be a check that cannot fail. Stating the field as what `prove` does with it
/// lets both crates say what they mean. RFC 0057.
const PROOF_HARNESSES: &[(&str, bool, &str)] = &[
    (
        "unnamed",
        true,
        "a handle is not a global name: it resolves only in the table that issued it",
    ),
    ("forged", true, "exactly the handles a table issued resolve, over all 2^32 of them"),
    ("forged_across_a_process", true, "nothing the last process held resolves in the next one"),
    (
        "stale",
        true,
        "after a revoke, the one handle still standing is the one it was told to spare",
    ),
    ("narrowing", true, "a derive weakens and never widens, over the whole 256x256 rights lattice"),
    ("total_lookup", true, "inspect and invoke refuse every handle the table did not issue"),
    ("total_derive", true, "and so does derive, over every rights bitmap at once"),
    ("total_revoke", true, "and condemn, condemn_own and relinquish"),
    ("total_frame_side", true, "and the four the frame performs on a component's behalf"),
    ("total_bought", false, "the same lookups on a table that has bought part of itself"),
    // There is no second entry for a bought table, and the absence is argued
    // rather than accidental: the harness that would have said *a bought slot
    // does not answer the handle it answered last* was written, run, and taken
    // out because it did not terminate in forty-five minutes. `kernel/proofs`
    // states the gap where the harness was, and `cap::properties::forged`
    // checks the same sentence at every boot at the real page size.
];

/// The bound this crate is proved inside, and the harnesses it actually binds.
///
/// `kernel/proofs/src/mem.rs` sets `FRAME_SIZE` to 256 rather than 4096, so a
/// page of slots is eight rather than a hundred and twenty-eight and the
/// revocation walk is a loop a checker finishes. RFC 0053 argues it.
///
/// **The reduction binds one of the ten, and that is checked rather than
/// asserted.** `FRAME_SIZE` reaches `cap.rs` in exactly two places — the
/// arithmetic in `retype` and the page `grow` buys — so a harness that never
/// grows its table cannot depend on it. Nine of these never grow, and rather
/// than leave that as an argument this verb runs all nine a second time with
/// `wide-page`, which is the kernel's own 4096. If the nine are really
/// independent of the page size they pass both ways; if one of them is not,
/// this is where that is discovered instead of being reasoned about.
///
/// `total_bought` is the one that grows, and it is proved at the reduced size
/// only. That is the whole of the bound, and it is one harness wide rather than
/// ten.
const PROOF_WIDE: &str = "wide-page";

/// The harness the deliberate defect has to break.
///
/// `total_lookup` and not one of the others, because it is the narrowest
/// harness that hands an arbitrary handle to `Table::resolve`, which is the
/// function the defect removes the bounds check from. It is also the property
/// that has no fixture: `kernel/src/cap.rs` argues at `Table::slot` why *a process cannot
/// make the kernel panic by trying* cannot be broken by a table handed to the
/// suite, and `MUTATIONS` is the boot that answers it. This is the third
/// instrument on that one property, and the only one of the three that says
/// *no handle does this* rather than *these handles did not*.
///
/// Most of the others fail under the defect too — every harness that hands an
/// arbitrary handle to a lookup does — and
/// requiring one is deliberate: an armed run costs what a clean one costs, and
/// what the pair has to demonstrate is that the proof *can* fail, not how
/// widely it does.
const PROOF_DEFECT_BREAKS: &str = "total_lookup";

/// The deliberate defect this verb arms, which is the one `mutate` boots.
const PROOF_DEFECT: &str = "mutate-unchecked-index";

/// The module the harnesses live in, which is half of the name Kani knows them
/// by.
///
/// Named in full and matched with `--exact`, because Kani's harness filter is a
/// substring: `--harness forged` selects `forged_across_a_process` and
/// `forged_across_a_bought_page` as well, and a run that quietly did three
/// proofs when it was asked for one is a timing nobody can read and a failure
/// attributed to the wrong harness.
const PROOF_MODULE: &str = "proofs";

/// Where the ring's proofs are.
///
/// A second crate rather than a second module of the first, because the two are
/// built by one checker over two *different* dependency graphs: `kernel/proofs`
/// compiles a file out of a bare-metal binary against stand-ins, and this one
/// takes `f-ring` as an ordinary path dependency because `f-ring` builds for the
/// host. RFC 0057 argues why that difference is worth two directories.
const RING_PROOFS: &str = "ring/proofs";

/// The bound the ring's proofs are stated inside, widened.
///
/// `ring/proofs/src/peer.rs` sets `REGION` to 640 bytes, which `f_abi::layout`
/// turns into a ring of one or two entries. This is the same fixture at 1216,
/// which holds a ring of eight. Same role as [`PROOF_WIDE`] one crate over: the
/// harnesses whose cost does not depend on the ring size are run twice, so
/// *the small ring binds only the harnesses that walk it* is a check rather
/// than an argument.
const RING_PROOF_WIDE: &str = "wide-ring";

/// Every harness the ring's proofs require, and the sentence each one is.
///
/// The middle field is **run this one again at the wider bound**, and here that
/// means the four harnesses that adopt `peer::Region` and walk it — because
/// they are the only ones the region's size reaches. The twelve that take an
/// argument list never construct a region at all, so running them under
/// `wide-ring` would be a check that cannot fail, which is the one kind of
/// check this file refuses to add.
///
/// `draining_an_arbitrary_channel` is the fifth that reads a region and is
/// **not** in the wide pass, and the reason is cost rather than principle: it
/// inlines `f_ring::execute` once per loop iteration, which is where its
/// minutes go, and a region holding a ring of eight is eight iterations rather
/// than two. What it would add over `popping_an_arbitrary_entry` at that bound
/// is the budget arithmetic, which does not read the region. Stated here rather
/// than left as an absence somebody has to notice.
const RING_PROOF_HARNESSES: &[(&str, bool, &str)] = &[
    (
        "adopting_an_arbitrary_layout",
        false,
        "a layout is adopted from exactly the headers this build would have written",
    ),
    (
        "negotiating_with_an_arbitrary_peer",
        false,
        "RFC 0011: peers meet in the middle, over every header and every feature pair",
    ),
    ("adopting_arbitrary_bytes", true, "a mapping is bound over bytes a solver chose, or refused"),
    (
        "popping_an_arbitrary_entry",
        true,
        "`pop` refuses every cursor pair and every slot number rather than panicking",
    ),
    ("taking_an_arbitrary_completion", true, "and `take`, over every cursor pair"),
    (
        "submitting_against_an_arbitrary_cursor",
        true,
        "and `submit`, which is peer-facing too because the consumer owns `tail`",
    ),
    (
        "draining_an_arbitrary_channel",
        false,
        "a drain does no more work than its budget, over every channel",
    ),
    (
        "executing_an_arbitrary_entry",
        false,
        "`execute` checks the envelope in R04's order, over every entry there is",
    ),
    (
        "reading_an_arbitrary_registration",
        false,
        "and so does `Request::read`, which is reached instead of the executor",
    ),
    ("reading_an_arbitrary_buffer_name", false, "both readings of the twelve bytes at offset 32"),
    (
        "believing_an_arbitrary_completion",
        false,
        "a client believes a set id only where the wire type says it may",
    ),
    (
        "registering_from_an_arbitrary_entry",
        false,
        "a translation is outstanding exactly when a slot is live",
    ),
    (
        "resolving_an_arbitrary_buffer_name",
        false,
        "what a resolve answers is inside the registration it names",
    ),
    ("retiring_an_arbitrary_set", false, "after a retirement no id resolves, over all 2^32"),
    (
        "lending_a_buffer_over_an_arbitrary_completion",
        false,
        "a lent buffer comes back exactly when the completion carrying its token does",
    ),
    (
        "both_transports_refuse_a_name_of_the_wrong_kind",
        false,
        "RFC 0028's two paths differ in nothing but the name they take",
    ),
    (
        "narrowing_a_granted_window",
        false,
        "RFC 0033: a sub-window cannot name a byte the whole one did not",
    ),
];

/// One harness: its name, whether the wide pass runs it again, and the sentence
/// it is.
///
/// A named type rather than a tuple written out at each use, because `prove`
/// carries a borrowed one of these per crate and the resulting signature is
/// the kind clippy asks to be given a name.
type Harness = (&'static str, bool, &'static str);

/// A crate of proofs: where it is, what it is about, and what must break it.
///
/// Two of these, and the shape is the argument for a table rather than a second
/// copy of `prove`: the *phases* are identical — verify, verify again at the
/// wider bound, then arm a defect and require a failure — and only the lists
/// differ. A second verb would have been a second place for the third phase to
/// quietly stop being run.
struct ProofCrate {
    /// The directory, relative to the root.
    dir: &'static str,
    /// What it proves, in one clause, for the report's first line.
    about: &'static str,
    /// The feature that widens the bound.
    wide: &'static str,
    /// What that feature widens it *to*, for the phase-two banner.
    wide_says: &'static str,
    /// The harnesses, and whether each pays for the bound the wide pass moves.
    harnesses: &'static [Harness],
    /// Whether every harness here owes a `kani::cover!` for each answer it can
    /// produce, and every one of them must be satisfiable.
    ///
    /// The checker will not enforce this and says so plainly: Kani 0.67.0
    /// prints `1 of 2 cover properties satisfied (1 unreachable)` and then
    /// `VERIFICATION:- SUCCESSFUL`, exit 0. There is no flag to buy the
    /// behaviour — `cargo kani --help` in the image this job names has no
    /// `--fail-uncoverable` — so a rule stated in five comments and enforced by
    /// nothing is CONTRIBUTING R01's worst case: a check somebody believes is
    /// happening. `prove_one` reads the count and refuses instead, and this
    /// field is which crates it refuses for.
    ///
    /// False for `kernel/proofs`, whose harnesses quantify over handles rather
    /// than over bytes and carry no covers: there the fixture cannot fail to
    /// reach the code, because there is no fixture between the harness and the
    /// table. Turning this on there would demand a line no report has, which is
    /// a check that fails for a reason nobody chose.
    covered: bool,
    /// The deliberate defects a proof here has to fail on.
    armed: &'static [Armed],
}

/// One deliberate defect, the harness it must break, and where it must break.
///
/// The `site` is the part that stops the third phase from passing for the wrong
/// reason. A checker that fails to compile, an unwinding assertion, a harness
/// with a bug of its own — all three exit non-zero and none of them is the
/// proof noticing the defect. So what is asserted is that a check which
/// *failed* carries this text, and `what` is the sentence printed when it does
/// not.
struct Armed {
    /// The feature to arm, which is the same name `mutate`, `hostile` or
    /// `entries` arms for the boot or the fuzzer.
    feature: &'static str,
    /// The harness that must fail with it on.
    harness: &'static str,
    /// Text a *failing* check must carry, in its location or its description.
    site: &'static str,
    /// What that text is, for the message when no failing check has it.
    what: &'static str,
}

/// The two crates, in the order `prove` runs them.
///
/// The kernel's first, because it is E1-P07's and the exit clause about a
/// schedule is stated against it; the ring's second, because E1-P12 needed the
/// apparatus to exist before it could use it.
const PROOF_CRATES: &[ProofCrate] = &[
    ProofCrate {
        dir: PROOFS,
        about: "the five capability properties, over arbitrary handles",
        wide: PROOF_WIDE,
        wide_says: "the kernel's own 4096 — the nine the page size does not reach",
        harnesses: PROOF_HARNESSES,
        covered: false,
        armed: &[Armed {
            feature: PROOF_DEFECT,
            harness: PROOF_DEFECT_BREAKS,
            site: "cap.rs",
            what: "the shipped file the `#[path]` reaches",
        }],
    },
    ProofCrate {
        dir: RING_PROOFS,
        about: "the ring's validation paths, over arbitrary peer bytes",
        wide: RING_PROOF_WIDE,
        wide_says: "a region holding a ring of eight — the four that read one",
        harnesses: RING_PROOF_HARNESSES,
        // Every harness here is a fixture standing between the solver and the
        // crate, so every one of them can stop reaching it — and a harness that
        // reaches nothing verifies instantly. `prove_one` requires the report's
        // cover line to be present and every cover in it satisfied.
        covered: true,
        // Five of the ring's nine deliberate defects, and the pairing is the
        // assertion rather than the exit code: each must break *the harness
        // that states the property it breaks*, and break it where the defect
        // is. RFC 0042's arithmetic is why they are not one — a harness with a
        // single defect behind it demonstrates that one property can fail and
        // decorates the rest. The other four are in `RING_PROOF_BLIND`, which
        // says why a proof here cannot see them.
        armed: &[
            Armed {
                feature: "mutate-trusted-slot",
                harness: "popping_an_arbitrary_entry",
                site: "f_ring::Consumer",
                what: "`Consumer::pop` in the shipped `ring/src/lib.rs`, where the \
                       bounds check on a peer's slot number is",
            },
            Armed {
                feature: "mutate-believed-header",
                harness: "adopting_arbitrary_bytes",
                site: "unwrap_failed",
                what: "the panic a failing `expect` makes, which is where this defect turns \
                       `Layout::adopt`'s refusal in `ring/src/mapping.rs`. Named as the \
                       helper rather than as the call site because that is what the report \
                       says: Kani attributes the failure to `core`'s `unwrap_failed` and \
                       cannot format the message, so `Mapping::adopt` appears only on \
                       *passing* checks. `adopting_arbitrary_bytes` contains no `unwrap` \
                       or `expect` of its own — grep is the check on that sentence — which \
                       is what makes the attribution unambiguous",
            },
            Armed {
                feature: "mutate-unbounded-drain",
                harness: "draining_an_arbitrary_channel",
                site: "a drain did more work than its budget",
                what: "the harness's own assertion, named rather than a location \
                       because this defect produces a wrong answer and not a fault: \
                       the loop still returns, and only the count is wrong",
            },
            Armed {
                feature: "mutate-ignored-flag",
                harness: "executing_an_arbitrary_entry",
                site: "the envelope is checked in the wrong order, or with the wrong list",
                what: "the harness's own assertion again, and for the same reason: R04's \
                       failure is two peers disagreeing about what happened, which \
                       nothing in the process faults on. `ring/tests/entries.rs` needed \
                       an oracle for exactly this, and so does a proof",
            },
            Armed {
                feature: "mutate-lenient-index",
                harness: "resolving_an_arbitrary_buffer_name",
                site: "a buffer past the end of the set was resolved",
                what: "the harness's own assertion, and it has to be: with the bounds \
                       check gone the mask is all that is left, and the address the \
                       arithmetic then produces is a *plausible* one inside somebody \
                       else's buffer rather than a fault. RFC 0048 calls that the reach \
                       oracle, and this is the same oracle over every index at once",
            },
        ],
    },
];

/// What a bounded model checker over this tree is blind to, as a set.
///
/// `PROVE_RUN_GAP` is the precedent and the argument is the same one. Five of
/// `ring/Cargo.toml`'s nine deliberate defects fail a harness in
/// `ring/proofs`; four cannot, and the honest move is to name them rather than
/// to let a reader infer from a page of green harnesses that the ring is
/// proved.
///
/// Arming one of these and requiring a failure would be the mistake the comment
/// on `MUTATIONS` records nearly making — a run that fails for the wrong reason
/// satisfies an exit status and proves nothing. Arming one and *not* requiring
/// a failure would be worse, because it would look like coverage.
///
/// Each names its own reason and they are not the same reason: three are the
/// memory model, which E0-P16 is the task that owes an instrument, and one is
/// the unwinding bound. The test
/// `every_ring_defect_is_either_armed_by_a_proof_or_declared_invisible_to_one`
/// is what keeps this list and that manifest in step, in both directions.
const RING_PROOF_BLIND: &[&str] = &[
    "mutate-relaxed-submission — the submission ring's publishing store weakened to \
     `Relaxed`, in both the single-entry and the batch path. CBMC is a sequential checker \
     with no weak memory model in it, so a proof here is insensitive to this by \
     construction — as is the stress suite: the one CI run that asked it to catch this \
     found that it does not",
    "mutate-relaxed-completion — the same on the completion ring, which RFC 0018 built \
     as the mirror and gave the ordering argument to by inheritance. Not caught either",
    "mutate-no-doorbell-fence — the `SeqCst` fence between publishing an entry and \
     reading `NEED_WAKEUP` removed. The one of the three the litmus suite *does* catch, \
     on the x86 runner, because store-load is the reordering total store order performs. \
     Still outside a proof here, and named so that two green instruments are not read as \
     one",
    "mutate-reusable-slot — a registration slot refilled after its generations have run \
     out. Not an ordering question and not invisible in principle: it needs a slot at \
     `SetId::RETIRED_GENERATION`, which is sixty-five thousand five hundred and \
     thirty-four retirements of one slot, and a bounded checker unrolls that loop rather \
     than summarising it. This is the *depth* bound rather than the memory model, and it \
     is in the same list because the consequence for a reader is identical. \
     `ring/tests/entries.rs` keeps a ledger of every id a table has issued for exactly \
     this defect, and is where it is caught",
];

/// Prove what the checker can prove, then require a defect to break it.
///
/// # Why this is a command and not a test
///
/// For `mutate`'s reason, one layer up. What it asserts about the last phase is
/// a *failure*, and the only place a failed proof is observable is the exit
/// status of a checker `cargo test` cannot run: Kani brings its own rustc, so
/// both proof crates are outside the workspace and nothing in `verify` reaches
/// them.
///
/// # Why every phase
///
/// Neither half means anything alone, and this is the fourth time that sentence
/// is written in this file. A green proof says nothing if the same proof is
/// green on a build with the check taken out — that is a harness that has
/// stopped reading the code, which is exactly what a proof over a stale copy
/// would be. `kernel/proofs` compiles the shipped file through `#[path]` and
/// `ring/proofs` links the shipped crate; the armed phase is what demonstrates
/// that either arrangement still reaches what it claims to.
///
/// # Why the covers are checked here and not left to the checker
///
/// Because a proof over arbitrary bytes has a failure mode a proof over
/// arbitrary handles does not: a fixture that never gets past the first check
/// verifies instantly and proves nothing. `ring/proofs` carries `kani::cover!`
/// for every answer a harness can produce, and the rule is that an
/// unsatisfiable cover is a *failed* proof.
///
/// Kani does not implement that rule. It prints
/// `1 of 2 cover properties satisfied (1 unreachable)` and then
/// `VERIFICATION:- SUCCESSFUL` with exit 0, and the version in the image this
/// verb runs in has no flag that changes it. So `prove_one` reads the count and
/// refuses on it — see `cover_check`. Until it did, the rule was written in
/// five places and mechanised in none, which is the one shape CONTRIBUTING R01
/// calls worse than an honestly manual check. RFC 0057.
fn prove(only: Option<&str>) -> Result<(), String> {
    let version = kani_version()?;
    println!("prove: {version}");

    let mut wanted: Vec<(&ProofCrate, Vec<&Harness>)> = Vec::new();
    for krate in PROOF_CRATES {
        let picked: Vec<_> = match only {
            Some(name) => {
                krate.harnesses.iter().filter(|(harness, _, _)| *harness == name).collect()
            }
            None => krate.harnesses.iter().collect(),
        };
        if !picked.is_empty() {
            wanted.push((krate, picked));
        }
    }
    if wanted.is_empty() {
        let name = only.unwrap_or("");
        let list = PROOF_CRATES
            .iter()
            .flat_map(|krate| {
                krate
                    .harnesses
                    .iter()
                    .map(move |(harness, _, what)| format!("  {harness:<48} {what}"))
            })
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("no harness called `{name}`. They are:\n{list}"));
    }

    let total: usize = wanted.iter().map(|(_, picked)| picked.len()).sum();
    println!("       {total} harness(es), in {} crate(s)", wanted.len());

    let mut wide_total = 0usize;
    let mut armed_total = 0usize;
    for (krate, picked) in &wanted {
        println!("\n=== {} — {}", krate.dir, krate.about);

        println!("\n[1/3] the proofs, at the bound the fixture reduces");
        for (harness, _, what) in picked {
            println!("\n  {harness}: {what}");
            prove_one(krate, harness, &[])?;
        }

        let wide: Vec<&str> =
            picked.iter().filter(|(_, again, _)| *again).map(|(harness, _, _)| *harness).collect();
        wide_total += wide.len();
        println!("\n[2/3] the {} of them worth running again, at {}", wide.len(), krate.wide_says);
        println!("      — which is what turns the reduced bound from an argument into a");
        println!("      check that fails on the day it stops being true");
        for harness in &wide {
            println!("\n  {harness}");
            prove_one(krate, harness, &["--features", krate.wide])?;
        }

        let armed: Vec<&Armed> = krate
            .armed
            .iter()
            .filter(|a| picked.iter().any(|(harness, _, _)| *harness == a.harness))
            .collect();
        armed_total += armed.len();
        println!("\n[3/3] with the defects — {} of them, each on its own harness", armed.len());
        for defect in armed {
            prove_armed(krate, defect)?;
        }
    }

    println!(
        "\nprove: ok — {total} proof(s) hold, {wide_total} of them at both bounds, and\n\
        \x20      {armed_total} deliberate defect(s) each fail the harness that states\n\
        \x20      the property they break, where they break it."
    );
    if wanted.iter().any(|(krate, _)| krate.dir == RING_PROOFS) {
        println!(
            "\n  {} thing(s) a sequential checker cannot see, declared rather than \
             checked (RING_PROOF_BLIND):",
            RING_PROOF_BLIND.len()
        );
        for gap in RING_PROOF_BLIND {
            println!("  - {gap}");
        }
    }
    Ok(())
}

/// One harness, verified, or the report that says why not.
fn prove_one(krate: &ProofCrate, harness: &str, extra: &[&str]) -> Result<(), String> {
    let name = format!("{PROOF_MODULE}::{harness}");
    let mut args = vec!["--exact", "--harness", name.as_str()];
    args.extend_from_slice(extra);
    let (ok, log) = kani(krate.dir, &args, None)?;
    if !ok || kani_verdict(&log) != Some(true) {
        return Err(format!(
            "`{harness}` did not verify{}.\n\n{}\n\n\
             A failed check here is a counterexample rather than a flaky run:\n\
             `cargo kani --exact --harness {name} --concrete-playback=print`, in\n\
             {}, turns the assignment that produced it into a test case.\n\n\
             An *unsatisfiable cover* is the other way this fails, and it arrives\n\
             with the opposite status: the checker calls such a run SUCCESSFUL, so\n\
             `cover_check` is what turns it into a different error instead.",
            if extra.is_empty() { String::new() } else { format!(" with {}", extra.join(" ")) },
            kani_findings(&log),
            krate.dir,
        ));
    }
    if krate.covered {
        cover_check(&log).map_err(|why| {
            format!(
                "`{harness}` verified{}, and that verdict says nothing.\n\n{why}\n\n\
                 A cover this harness cannot satisfy is a fixture that has stopped\n\
                 reaching the code it is about — `peer::REGION` drifting against\n\
                 `f_abi::layout`, an assume that now excludes what it used to admit,\n\
                 a region no longer adopted. It is the failure this arrangement is\n\
                 exposed to and it looks exactly like a fast green run: Kani does\n\
                 not fail on one, it prints the count and reports SUCCESSFUL, which\n\
                 is why the count is read here rather than trusted.\n\n\
                 Which cover: `cargo kani --exact --harness {name}`, in {}, lists\n\
                 every cover property with its status.",
                if extra.is_empty() { String::new() } else { format!(" with {}", extra.join(" ")) },
                krate.dir,
            )
        })?;
    }
    println!("  {harness}: verified{}", covers(&log));
    Ok(())
}

/// One deliberate defect, and the failure it has to produce.
fn prove_armed(krate: &ProofCrate, defect: &Armed) -> Result<(), String> {
    let Armed { feature, harness, site, what } = defect;
    println!("\n  {feature}: `{harness}` must fail, at {what}");
    let name = format!("{PROOF_MODULE}::{harness}");
    let (ok, log) = kani(krate.dir, &["--exact", "--harness", &name], Some(feature))?;

    if ok || kani_verdict(&log) == Some(true) {
        return Err(format!(
            "`{harness}` verified on a build carrying `{feature}`.\n\n\
             That is the proof failing rather than holding. Either the feature no\n\
             longer reaches the code it names — check that {} still depends on the\n\
             crate it proves rather than on a copy — or the harness has stopped\n\
             presenting the input the defect needs.",
            krate.dir
        ));
    }
    if kani_verdict(&log).is_none() {
        return Err(format!(
            "the armed run did not reach a verdict, so it says nothing about\n\
             `{feature}`. A checker that fails to start also exits non-zero, and that\n\
             is not a proof failing. The tail of its output:\n\n{}",
            kani_findings(&log)
        ));
    }

    // The *failing checks*, not the log. A Kani report names the file under
    // proof in hundreds of ordinary check locations whether it passes or fails
    // — a clean `SUCCESSFUL` run carries several in its last forty lines — so
    // asking whether the log mentions it is a guard every possible armed run
    // satisfies, which is a guard that is not there. What has to be true is
    // that a check *which failed* is the one the defect was supposed to break.
    let sites = kani_failure_sites(&log);
    if !sites.iter().any(|found| found.contains(*site)) {
        let where_they_are = if sites.is_empty() {
            "  (the report located no failing check at all)".to_string()
        } else {
            sites.iter().map(|found| format!("  {found}")).collect::<Vec<_>>().join("\n")
        };
        return Err(format!(
            "`{harness}` failed on the armed build, and not for the reason it was\n\
             supposed to: no *failing* check carries `{site}`, which is {what}.\n\n\
             A proof that fails somewhere else — an unwinding assertion, a fixture\n\
             that has stopped matching the crate, a panic in the harness itself —\n\
             satisfies an exit status and proves nothing. Where the failures are:\n\n\
             {where_they_are}\n\n\
             The report:\n\n{}",
            kani_findings(&log)
        ));
    }
    println!("  {harness}: fails, at `{site}`");
    Ok(())
}

/// What a report says about its cover properties, or nothing when it has none.
///
/// Printed beside every verified harness because an unsatisfiable cover is the
/// failure mode a proof over arbitrary bytes has and a proof over arbitrary
/// handles does not — see `prove`'s own documentation. The count is read out of
/// the report rather than counted here, so a harness that lost a cover shows a
/// smaller number in the log rather than nothing at all.
fn covers(log: &str) -> String {
    cover_line(log).map(|line| format!("  ({line})")).unwrap_or_default()
}

/// The report's cover summary, as it wrote it.
fn cover_line(log: &str) -> Option<&str> {
    log.lines()
        .find(|line| line.contains("cover properties"))
        .map(|line| line.trim().trim_start_matches("** ").trim())
}

/// Every cover the report lists is satisfiable, or why that is not known.
///
/// # Why this is a check and not a printed number
///
/// Because the checker will not make it one. Kani 0.67.0 treats an
/// unsatisfiable cover as information: it prints
/// `** 1 of 2 cover properties satisfied (1 unreachable)` and then
/// `VERIFICATION:- SUCCESSFUL` with exit 0, and the image the nightly names
/// carries no flag that changes that. So the sentence RFC 0057 rests its own
/// honesty on — *an unsatisfiable cover is a failed proof* — was true of
/// nothing until it was read here. That is the shape CONTRIBUTING R01 calls
/// worse than an honestly manual rule: a check somebody believes is happening.
///
/// # Why a missing line is also a failure
///
/// For the crates this runs on, a report with no cover summary in it is a
/// harness whose covers compiled out — the same vacuum, arriving by a different
/// route and reading as an even quieter green. `ProofCrate::covered` is what
/// says which crates owe the line at all.
fn cover_check(log: &str) -> Result<(usize, usize), String> {
    let Some(line) = cover_line(log) else {
        return Err("the report carries no cover summary at all, so this harness states\n\
                    no reachability. Either its `kani::cover!` calls compiled out, or\n\
                    it never had any — and a harness over arbitrary bytes with no\n\
                    cover cannot be told from a fixture that refuses everything."
            .to_string());
    };
    let words: Vec<&str> = line.split_whitespace().collect();
    let parsed = match (words.first(), words.get(1), words.get(2)) {
        (Some(satisfied), Some(&"of"), Some(total)) => {
            satisfied.parse::<usize>().ok().zip(total.parse::<usize>().ok())
        }
        _ => None,
    };
    let Some((satisfied, total)) = parsed else {
        return Err(format!(
            "the cover summary `{line}` is not the shape this reads — `N of M cover\n\
             properties satisfied`. The checker's report format moved, and a count\n\
             that cannot be parsed must not be read as a count that is fine."
        ));
    };
    if satisfied != total {
        return Err(format!(
            "{}, so {} of them cannot be reached at all: `{line}`",
            if total == 1 {
                "1 cover property".to_string()
            } else {
                format!("{total} cover properties")
            },
            total - satisfied,
        ));
    }
    Ok((satisfied, total))
}

/// Which Kani is installed, or the sentence that says where to find one.
fn kani_version() -> Result<String, String> {
    let out =
        Command::new("cargo").args(["kani", "--version"]).current_dir(root().join(PROOFS)).output();
    match out {
        Ok(out) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        _ => Err("no `cargo kani` on PATH.\n\n\
             The checker brings its own toolchain — RFC 0022, and Kani takes that\n\
             further than RustMC did by shipping a rustc — so it is in the `full`\n\
             image rather than in `dev`:\n\n\
             \x20 docker compose -f docker/compose.yaml build full\n\
             \x20 docker compose -f docker/compose.yaml run --rm -T full cargo xtask prove\n\n\
             docker/README.md says what that costs. Nothing else in this tree needs\n\
             it, and `cargo xtask verify` does not run this verb."
            .to_string()),
    }
}

/// Run the checker in one proof crate, armed or not, and hand back its report.
///
/// Captured rather than streamed, because every phase of this verb is an
/// assertion about the *report*: a run that fails for the wrong reason
/// satisfies an exit status and proves nothing, which is the mistake the
/// comment on `MUTATIONS` records having nearly made.
fn kani(dir: &str, args: &[&str], armed: Option<&str>) -> Result<(bool, String), String> {
    let mut command = Command::new("cargo");
    command.arg("kani").args(args).current_dir(root().join(dir));
    if let Some(feature) = armed {
        command.args(["--features", feature]);
    }
    let out = command.output().map_err(|e| format!("could not run cargo kani: {e}"))?;
    let mut log = String::from_utf8_lossy(&out.stdout).into_owned();
    log.push_str(&String::from_utf8_lossy(&out.stderr));
    Ok((out.status.success(), log))
}

/// What the report says, or `None` when it does not say.
///
/// Read out of the text rather than taken from the exit status, because a
/// checker that failed to *start* also exits non-zero and would otherwise read
/// as a proof that failed — which is exactly the half of this verb that must
/// not pass for the wrong reason.
fn kani_verdict(log: &str) -> Option<bool> {
    log.lines().rev().find_map(|line| match line.trim() {
        "VERIFICATION:- SUCCESSFUL" => Some(true),
        "VERIFICATION:- FAILED" => Some(false),
        _ => None,
    })
}

/// The lines of a report worth putting in front of a person.
fn kani_findings(log: &str) -> String {
    let mut lines: Vec<&str> = Vec::new();
    let mut carry = 0usize;
    for line in log.lines() {
        if line.contains("Status: FAILURE") || line.starts_with("VERIFICATION") {
            lines.push(line.trim_end());
            carry = 3;
        } else if carry > 0 && (line.contains("Description:") || line.contains("Location:")) {
            lines.push(line.trim_end());
            carry -= 1;
        }
    }
    if lines.is_empty() {
        // A report with no failing check in it is a checker that did not run,
        // and its tail is where that always says so.
        let tail: Vec<&str> = log.lines().collect();
        let from = tail.len().saturating_sub(30);
        return tail[from..].join("\n");
    }
    lines.join("\n")
}

/// Both proof crates still compile against the code they prove.
///
/// # Why this is in `lint` and not left to the schedule
///
/// RFC 0053 names one standing cost of compiling `kernel/src/cap.rs` a second
/// time: adding a `use crate::` naming a fourth kernel module to that file
/// breaks a build that nothing in `cargo xtask verify` runs. It then offers as
/// the mitigation that the crate compiles under the *pinned* nightly as well
/// as under Kani's — "which is what makes *the stand-ins still match `mem`* a
/// thing an ordinary `cargo build` can find out rather than something the
/// schedule discovers". Nothing was running that build, so the mitigation was
/// a sentence: the first thing to notice a fourth dependency would have been a
/// nightly `prove` job costing twenty minutes, hours after the person who
/// wrote the line had moved on. That is the shape CONTRIBUTING R01 calls a
/// rule written as a mechanism while remaining a plan.
///
/// So the build runs here. It needs no Kani — `cfg(kani)` is off, the
/// harnesses compile out, and what is left is `cap.rs` and three stand-ins.
///
/// `ring/proofs` is here for a related but not identical reason, and the
/// difference is worth a sentence because it is the whole of what RFC 0057
/// decided differently. That crate takes `f-ring` as a path dependency rather
/// than compiling a file out of it, so there is no stand-in to fall out of
/// step and no `#[path]` to stop reaching anything. What it buys instead is
/// every *call* the proofs make: `mod proofs` is compiled by this build too —
/// only the `kani::proof` and `kani::unwind` attributes are conditional, and
/// `kani::any`, `kani::assume` and `kani::cover!` have a shim — so
/// `Consumer::pop`, `Mapping::adopt`, `Table::register`, `execute` and
/// `BufferSet::carve` are all typechecked against their real signatures here.
/// While `mod proofs` was `#[cfg(kani)]` this build saw the three trait
/// implementations in `peer` and none of that, which is most of what the check
/// exists for; it found a dead import the moment it stopped being.
///
/// What it still cannot catch is the *fixture's arithmetic*: `peer::REGION` is
/// computed against `f_abi::layout`'s offsets, and a change to those moves
/// which ring sizes the proofs admit without moving a type. Only the covers
/// catch that, at `prove` time, and `proofs::reached` is the pair of them that
/// names a ring size. So this check is weaker there than here and says so.
///
/// # Why every feature configuration
///
/// Because all but the first are built by nothing else in the gate: the wide
/// bound by `prove`'s second pass and each deliberate defect by its third. A
/// feature that stopped compiling would otherwise be found by the run whose
/// job is to assert a *failure*, where a build error and a failed proof are
/// the same exit status. `prove` distinguishes them by reading the verdict
/// rather than the status, and that is still a worse place to find it than
/// here.
///
/// `cargo fmt --check` is here for a smaller reason with the same shape:
/// `lint_all`'s own `cargo fmt --all` is workspace-scoped and both crates are
/// excluded from the workspace, so nothing was checking their formatting.
fn lint_proofs() -> Result<(), String> {
    let mut built = 0usize;
    for krate in PROOF_CRATES {
        let dir = root().join(krate.dir);
        if !dir.join("Cargo.toml").is_file() {
            // Deleting the directory is the documented whole of undoing this
            // arrangement — RFC 0053's last reversal condition, and RFC 0057
            // keeps it — so the lint refuses to be the thing that makes that
            // expensive.
            println!("lint-proofs: skipped  ({} is not present)", krate.dir);
            continue;
        }
        // Every configuration `prove` builds, because all but the first are
        // built by nothing else in the gate: the wide bound by phase two and
        // each defect by phase three. A feature that stopped compiling would
        // otherwise be found by the run whose job is to assert a *failure*,
        // where a build error and a failed proof are the same exit status.
        // `prove` distinguishes them by reading the verdict rather than the
        // status, and that is still a worse place to find it than here.
        let mut configurations: Vec<Vec<&str>> = vec![vec![], vec!["--features", krate.wide]];
        configurations.extend(krate.armed.iter().map(|defect| vec!["--features", defect.feature]));
        for extra in &configurations {
            let mut args = vec!["check", "--quiet"];
            args.extend_from_slice(extra);
            // `RUSTFLAGS` and not `-- -D warnings`, which `cargo check` does not
            // take. It is here for the reason every other invocation in `lint_all`
            // carries `-D warnings`: in this tree a `warning:` line is a failure,
            // and these crates are outside the workspace, so they inherit no lint
            // table and nothing else would turn one into one.
            run_in_with(&dir, "cargo", &args, &[("RUSTFLAGS", "-D warnings")])?;
        }
        run_in(&dir, "cargo", &["fmt", "--", "--check"])?;
        built += configurations.len();
    }
    let dependents = proof_schedule()?;
    println!(
        "lint-proofs: ok  ({} crate(s) build against the code they prove under the pinned \
         toolchain, in {built} configuration(s) between them; the nightly still runs \
         `cargo xtask prove`, and {dependents} job depends on the checker's image)",
        PROOF_CRATES.len()
    );
    println!(
        "  {} thing(s) a sequential checker cannot see (RING_PROOF_BLIND, declared \
         rather than checked): {}",
        RING_PROOF_BLIND.len(),
        RING_PROOF_BLIND
            .iter()
            .filter_map(|gap| gap.split_once(' ').map(|(name, _)| name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  {} propert{} of E1-P07's exit the schedule establishes and this machine \
         cannot (PROVE_RUN_GAP, declared rather than checked):",
        PROVE_RUN_GAP.len(),
        if PROVE_RUN_GAP.len() == 1 { "y" } else { "ies" }
    );
    for gap in PROVE_RUN_GAP {
        println!("  - {gap}");
    }
    Ok(())
}

/// The schedule still runs the proofs, and still only the proofs pay for the
/// checker.
///
/// # What this can and cannot say
///
/// `E1-P07`'s exit is *the proofs run in CI on a schedule, and a mutation to
/// the capability code fails them*. The second clause is an assertion
/// `cargo xtask prove` makes on this machine. The first is a statement about
/// GitHub, and nothing in this repository can observe a workflow run — so the
/// honest move is not to claim it but to check the half that is here: that the
/// file still says what the clause rests on. A `prove` job renamed, or a
/// `cargo xtask prove` quietly dropped out of its `run:`, is how that clause
/// stops being true without anybody deciding it should, and this is the only
/// place noticing is cheap.
///
/// # The third check, which is the one review added
///
/// `image_full` is a job of its own, and exactly one job may depend on it.
///
/// It was a second step inside `image` first. Six nightly jobs carry
/// `needs: [environment, image]`, so a Kani layer there — a `cargo install`
/// from crates.io and a 483 MB download from GitHub releases, both at
/// image-build time — put the sweep, both fuzzers, the Miri job and the join
/// behind the checker's toolchain, none of which have ever heard of it. That is
/// not hypothetical: `docker/README.md` records that exact download failing on
/// one machine. The count is *checked* rather than described because the
/// cheapest way to reintroduce it is to add `image_full` to a `needs:` list
/// while thinking about something else.
fn proof_schedule() -> Result<usize, String> {
    let text = std::fs::read_to_string(root().join(NIGHTLY)).map_err(|e| {
        format!(
            "reading {NIGHTLY}: {e}\n\n\
             The schedule half of E1-P07's exit is a job in that file, so a check on it \
             cannot be run without it."
        )
    })?;
    for (needle, what) in [
        ("cron:", "a schedule at all"),
        ("cargo xtask prove", "the verb the `prove` job exists to run"),
        ("outputs.image_full", "the image that carries the checker"),
    ] {
        if !text.contains(needle) {
            return Err(format!(
                "{NIGHTLY} no longer contains `{needle}`, which is {what}.\n\n\
                 E1-P07's exit says the proofs run on a schedule. Nothing in this tree can\n\
                 watch GitHub run them, so what it checks instead is that the file still\n\
                 says so — and it no longer does. If the job moved, move this check with\n\
                 it. If it went, `docs/TESTING-STATUS.md`'s L3 row and RFC 0053 now\n\
                 describe a schedule that does not exist."
            ));
        }
    }
    let dependents = text
        .lines()
        .filter(|line| line.trim_start().starts_with("needs:") && line.contains("image_full"))
        .count();
    if dependents != 1 {
        return Err(format!(
            "{dependents} job(s) in {NIGHTLY} depend on `image_full`, and exactly one may.\n\n\
             That image carries Kani's own rustc, built by fetching a crate and a 483 MB\n\
             release at image-build time. Every job waiting on it is a job the checker's\n\
             toolchain can take down — and the sweep, the two fuzzers and the Miri job\n\
             assert things that have nothing to do with a proof. A check that does not run\n\
             asserts nothing, so the blast radius of that layer is one job on purpose.\n\
             RFC 0053, and the header of the `image_full` job itself."
        ));
    }
    Ok(dependents)
}

/// The schedule E1-P07's exit names.
const NIGHTLY: &str = ".github/workflows/nightly.yml";

/// What the local loop cannot observe about the scheduled proofs, as a set
/// rather than a sentence.
///
/// `ARCH_RUN_GAP` is the precedent and the argument is the same one. E1-P07's
/// exit has two clauses. *A mutation to the capability code fails them* is
/// decided by running something, so it is decided here, by `cargo xtask
/// prove`'s third phase. *The proofs run in CI on a schedule* is decided by
/// GitHub, and nothing in this repository can watch GitHub — so rather than
/// write "CI covers it" and move on, the honest move is to name exactly what
/// is unobserved and to check the part that is not.
///
/// [`proof_schedule`] is the part that is not: the file still holds a
/// schedule, still runs the verb, still names the checker's image, and exactly
/// one job depends on that image. Everything below is what remains, and the
/// list is short on purpose — a long one would mean the verb had stopped being
/// worth running locally.
const PROVE_RUN_GAP: &[&str] = &[
    "that GitHub runs the `prove` job at all — the schedule, the container pull and the \
     runner's own two cores are outside anything this tree can execute, so a green \
     `cargo xtask prove` here is evidence about the *proofs* and not about the cadence",
    "that the `full` image builds on a GitHub runner — the Kani layer fetches a crate from \
     crates.io and a 483 MB release from GitHub, and the only builder it has ever run on \
     needed `--network=host` to do it (docker/README.md). `image_full` is a job of its own \
     so that being wrong about this costs one check rather than seven",
];

/// [`sh`], somewhere other than the root.
///
/// One caller: `kernel/proofs` is not a workspace member, so a `cargo` run
/// against it has to start inside it — from the root, cargo finds the
/// workspace this crate is deliberately excluded from.
fn run_in(dir: &Path, program: &str, args: &[&str]) -> Result<(), String> {
    run_in_with(dir, program, args, &[])
}

/// [`run_in`], with environment variables set for the child.
fn run_in_with(
    dir: &Path,
    program: &str,
    args: &[&str],
    env: &[(&str, &str)],
) -> Result<(), String> {
    let status = Command::new(program)
        .args(args)
        .envs(env.iter().copied())
        .current_dir(dir)
        .status()
        .map_err(|e| format!("could not run {program}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{program} {} failed in {}", args.join(" "), relative(dir)))
    }
}

/// Where the checks that *failed* are, and nothing about the ones that passed.
///
/// [`kani_findings`] is text for a person to read; this is what the armed half
/// asserts on, and the two must not be the same lines. Kani emits a
/// `Location:` for every check it generates, so a report over `kernel/proofs`
/// names `cap.rs` hundreds of times on a run where nothing failed at all — a
/// clean `narrowing` carries three in its last forty lines. Asking whether the
/// *log* mentions the file is therefore a question every possible armed run
/// answers yes to. Asking where the failures are is not.
///
/// A `Status: FAILURE` line is followed by that check's `Description:` and
/// `Location:`, so the first of each after a failure is the failure's and no
/// other line qualifies.
///
/// # Why the description is here as well as the location
///
/// Because three of the ring's five armed defects produce a **plausible wrong
/// answer** rather than a fault — an ignored flag, an unbounded drain, a
/// lenient index — and nothing in the process faults on one. What fails for
/// those is a harness assertion, and what identifies it is the sentence that
/// assertion carries. A guard that could only read locations would have to
/// accept *some check in proofs.rs failed*, which every unwinding assertion in
/// the file also satisfies. So a site is the description and the location
/// together, and `RING_PROOFS`'s table says for each defect which half it is
/// matching on. RFC 0057.
fn kani_failure_sites(log: &str) -> Vec<String> {
    let mut sites: Vec<String> = Vec::new();
    let mut failing = false;
    let mut open: Option<String> = None;
    for line in log.lines() {
        // Kani indents these under the check they belong to, and the leading
        // bullet is part of the format rather than of the text.
        let line = line.trim().trim_start_matches("- ").trim();
        if line.starts_with("Status:") {
            // Any `Status:` closes the previous check, so a failure whose
            // location the report omits cannot inherit the next check's.
            if let Some(partial) = open.take() {
                sites.push(partial);
            }
            failing = line.contains("FAILURE");
        } else if failing && line.starts_with("Description:") {
            open = Some(line.to_string());
        } else if failing && line.starts_with("Location:") {
            let described = open.take().map_or_else(String::new, |text| format!("{text}  "));
            sites.push(format!("{described}{line}"));
            failing = false;
        }
    }
    if let Some(partial) = open {
        sites.push(partial);
    }
    sites
}

#[cfg(test)]
mod proof_report {
    /// A report where nothing failed still names the file all over.
    ///
    /// This is the fixture for the mistake the armed half made first:
    /// `log.contains("cap.rs")` is true of the text below, which is a
    /// `SUCCESSFUL` run. A guard satisfied by a passing report is a guard
    /// satisfied by every armed run there could be, and the exit clause it was
    /// standing for — *a mutation to the capability code fails them* — would
    /// have been green on a proof that never noticed the defect.
    const CLEAN: &str = "Check 411: cap::Table::place.assertion.1
	 - Status: SUCCESS
	 - Description: \"assertion failed\"
	 - Location: src/../../src/cap.rs:1346:26 in function cap::Table::place

VERIFICATION:- SUCCESSFUL
";

    /// The armed report, in the shape `cargo xtask prove` actually reads.
    const ARMED: &str = "Check 12: cap::Table::slot.assertion.1
	 - Status: SUCCESS
	 - Description: \"\"
	 - Location: src/../../src/cap.rs:1300:9 in function cap::Table::slot

Check 13: cap::Table::slot.bounds.1
	 - Status: FAILURE
	 - Description: \"index out of bounds: the length is less than or equal to the given index\"
	 - Location: src/../../src/cap.rs:1283:12 in function cap::Table::slot

VERIFICATION:- FAILED
";

    /// A failure that is the harness's own, which must not satisfy the guard.
    const ELSEWHERE: &str = "Check 7: proofs::stale.unwind.0
	 - Status: FAILURE
	 - Description: \"unwinding assertion loop 0\"
	 - Location: src/proofs.rs:174:5 in function proofs::stale

VERIFICATION:- FAILED
";

    #[test]
    fn a_passing_report_yields_no_failure_sites() {
        assert!(super::kani_failure_sites(CLEAN).is_empty());
        // And the thing that makes the test worth having: the naive guard the
        // careful one replaced is satisfied by exactly this text.
        assert!(CLEAN.contains("cap.rs"));
    }

    #[test]
    fn an_armed_report_locates_its_failure_in_the_shipped_file() {
        let sites = super::kani_failure_sites(ARMED);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert!(sites[0].contains("cap.rs"), "{sites:?}");
        assert!(sites[0].contains("cap::Table::slot"), "{sites:?}");
    }

    #[test]
    fn the_half_of_the_exit_this_machine_cannot_see_is_declared_and_still_unseen() {
        // Both directions, the way `ARCH_RUN_GAP` and `JOIN_GAP` are checked.
        // An empty list would say the local loop observes the schedule, which
        // is false and is the shape of a gap quietly deleted rather than
        // closed. The second assertion is the other direction: the half that
        // *is* local has to still hold, so that the day somebody renames the
        // job, `cargo xtask test-host` says so as well as `lint`.
        assert!(
            !super::PROVE_RUN_GAP.is_empty(),
            "nothing here can watch GitHub run the proofs, so this owes a list of what it              therefore does not know"
        );
        super::proof_schedule().expect("the nightly no longer says what the exit rests on");
    }

    #[test]
    fn a_failure_in_the_harness_is_not_a_failure_in_cap_rs() {
        let sites = super::kani_failure_sites(ELSEWHERE);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert!(
            !sites.iter().any(|site| site.contains("cap.rs")),
            "an unwinding assertion in the harness would satisfy the guard: {sites:?}"
        );
    }

    /// A wrong answer fails an assertion, and the sentence is what names it.
    ///
    /// Three of the ring's five armed defects produce no fault at all, so the
    /// only thing that tells their failure from an unwinding assertion in the
    /// same file is the message the assertion carries. This is the fixture for
    /// that: the location alone would be satisfied by any failure in
    /// `proofs.rs`, and the pair is not.
    const WRONG_ANSWER: &str = "Check 91: proofs::draining_an_arbitrary_channel.assertion.2
	 - Status: FAILURE
	 - Description: \"a drain did more work than its budget\"
	 - Location: src/proofs.rs:395:13 in function proofs::draining_an_arbitrary_channel

VERIFICATION:- FAILED
";

    #[test]
    fn a_wrong_answer_is_identified_by_its_sentence_and_not_by_its_file() {
        let sites = super::kani_failure_sites(WRONG_ANSWER);
        assert_eq!(sites.len(), 1, "{sites:?}");
        assert!(sites[0].contains("a drain did more work than its budget"), "{sites:?}");
        assert!(sites[0].contains("src/proofs.rs"), "{sites:?}");
        // And the guard that would have been satisfied by any failure in the
        // file, which is the one this replaced.
        let unwinding = super::kani_failure_sites(ELSEWHERE);
        assert_eq!(unwinding.len(), 1, "{unwinding:?}");
        assert!(
            !unwinding[0].contains("a drain did more work than its budget"),
            "an unwinding assertion satisfied a wrong-answer defect's site: {unwinding:?}"
        );
    }

    /// The harness the header defect must break carries no `expect` of its own.
    ///
    /// `RING_PROOFS` matches that defect against `unwrap_failed`, because Kani
    /// attributes a failing `expect` to `core`'s helper rather than to the call
    /// site — so the sentence that makes the attribution unambiguous is *this
    /// harness has no other one*. That sentence is checkable, so it is checked
    /// rather than written in the table and left.
    #[test]
    fn the_harness_the_header_defect_breaks_has_no_unwrap_of_its_own() {
        let path = super::root().join(super::RING_PROOFS).join("src/proofs.rs");
        let Ok(text) = std::fs::read_to_string(&path) else {
            // Deleting the directory is the documented whole of undoing the
            // arrangement; this test refuses to be what makes that expensive.
            return;
        };
        let Some(start) = text.find("fn adopting_arbitrary_bytes()") else {
            panic!("the harness `mutate-believed-header` is matched against is gone");
        };
        let body = &text[start..];
        let end = body.find("\n}").map_or(body.len(), |at| at + 2);
        let body = &body[..end];
        for reached in ["unwrap(", "expect("] {
            assert!(
                !body.contains(reached),
                "`adopting_arbitrary_bytes` now contains `{reached}`, so a panic from it \
                 would be attributed to `unwrap_failed` exactly as the defect's is, and \
                 `cargo xtask prove`'s third phase could no longer tell the two apart. \
                 Either take it out or give that entry in `RING_PROOFS` a site that can."
            );
        }
    }

    /// A verified report whose fixture stopped reaching the code.
    ///
    /// Captured from the checker in the image the nightly names rather than
    /// written from memory: a throwaway crate with one satisfiable cover and
    /// one made unreachable by an `assume` produces exactly this, verdict
    /// included. That verdict is the finding — Kani does not fail on an
    /// unreachable cover — so this is the fixture for the thing `prove_one`
    /// had to start doing instead.
    const VACUOUS: &str = "Check 2: mixed.cover.2
	 - Status: UNREACHABLE
	 - Description: \"impossible\"
	 - Location: src/lib.rs:9:9 in function mixed

SUMMARY:
 ** 0 of 1 failed

 ** 1 of 2 cover properties satisfied (1 unreachable)


VERIFICATION:- SUCCESSFUL
";

    /// The same report with every cover reached.
    const REACHED: &str = "SUMMARY:
 ** 0 of 301 failed (3 unreachable)

 ** 3 of 3 cover properties satisfied


VERIFICATION:- SUCCESSFUL
";

    #[test]
    fn a_cover_nothing_can_reach_is_a_failure_the_verdict_does_not_carry() {
        // The half that is the finding: the checker verified it.
        assert_eq!(super::kani_verdict(VACUOUS), Some(true));
        assert!(super::kani_failure_sites(VACUOUS).is_empty());
        // And the half that is the fix.
        let why = super::cover_check(VACUOUS).expect_err("an unreachable cover was accepted");
        assert!(why.contains("1 of them cannot be reached"), "{why}");
        assert_eq!(super::cover_check(REACHED), Ok((3, 3)));
    }

    #[test]
    fn a_report_with_no_cover_summary_is_refused_where_one_is_owed() {
        // `kernel/proofs` carries no covers and is not asked for any; the ring's
        // crate is, and a report that lost the line is the same vacuum arriving
        // more quietly than an unreachable cover does.
        assert!(super::cover_check(CLEAN).is_err());
        let owed: Vec<bool> = super::PROOF_CRATES.iter().map(|krate| krate.covered).collect();
        assert!(owed.contains(&true), "no crate owes its covers, so nothing is checked");
        assert!(
            super::PROOF_CRATES
                .iter()
                .any(|krate| krate.dir == super::RING_PROOFS && krate.covered),
            "the crate whose harnesses are fixtures over bytes stopped owing its covers"
        );
    }

    /// Every harness in `ring/proofs` carries at least one `kani::cover!`.
    ///
    /// The other direction of the same rule. `cover_check` refuses a report
    /// with no summary in it, but that is a twenty-minute run away; a harness
    /// added with no cover at all is findable here in a second, and it is the
    /// cheapest way this proof goes quietly vacuous.
    #[test]
    fn every_ring_harness_states_something_it_can_reach() {
        let path = super::root().join(super::RING_PROOFS).join("src/proofs.rs");
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        for (harness, _, _) in super::RING_PROOF_HARNESSES {
            let start = text
                .find(&format!("fn {harness}()"))
                .unwrap_or_else(|| panic!("`{harness}` is in the table and not in the file"));
            let body = &text[start..];
            let end = body.find("\n}").map_or(body.len(), |at| at + 2);
            assert!(
                body[..end].contains("kani::cover!"),
                "`{harness}` states no cover, so nothing says its fixture reaches the\n\
                 code it is about. A harness over arbitrary bytes whose first check\n\
                 refuses everything verifies instantly and proves nothing."
            );
        }
    }

    /// Every deliberate defect in `ring/Cargo.toml` is either armed by a proof
    /// or declared as one a proof cannot see.
    ///
    /// The check that stops `RING_PROOF_BLIND` from becoming decoration. Nine
    /// defects, six armed by a fuzzer and three by a boot or a litmus test —
    /// what matters here is that none of them is in *neither* list, because a
    /// defect nobody has decided about is how a reader comes to believe that
    /// ten green harnesses mean the ring is proved. Adding a tenth defect to
    /// that manifest and to neither list fails here, which is the cheapest
    /// place to find out.
    #[test]
    fn every_ring_defect_is_either_armed_by_a_proof_or_declared_invisible_to_one() {
        let manifest = std::fs::read_to_string(super::root().join("ring/Cargo.toml"))
            .expect("ring/Cargo.toml");
        let declared: Vec<&str> = manifest
            .lines()
            .filter_map(|line| line.split_once(" = []"))
            .map(|(name, _)| name.trim())
            .filter(|name| name.starts_with("mutate-"))
            .collect();
        assert!(!declared.is_empty(), "the manifest stopped declaring defects at all");

        let armed: Vec<&str> = super::PROOF_CRATES
            .iter()
            .filter(|krate| krate.dir == super::RING_PROOFS)
            .flat_map(|krate| krate.armed.iter().map(|defect| defect.feature))
            .collect();
        assert!(!armed.is_empty(), "no ring defect is armed, so the third phase asserts nothing");

        for defect in &declared {
            let blind = super::RING_PROOF_BLIND.iter().any(|gap| gap.starts_with(defect));
            assert!(
                armed.contains(defect) || blind,
                "`{defect}` is in ring/Cargo.toml and in neither RING_PROOFS nor \
                 RING_PROOF_BLIND. A defect nobody has decided about is how a reader \
                 comes to believe a green `prove` covers more than it does: either a \
                 harness must fail on it, or this file must say why one cannot."
            );
            assert!(
                !(armed.contains(defect) && blind),
                "`{defect}` is both armed and declared invisible, which is two answers \
                 to one question"
            );
        }
        // And the other direction, so the list cannot outlive the manifest.
        for gap in super::RING_PROOF_BLIND {
            let name = gap.split(' ').next().unwrap_or_default();
            assert!(
                declared.contains(&name),
                "RING_PROOF_BLIND names `{name}`, which ring/Cargo.toml no longer \
                 declares — a gap that outlived the thing it was about"
            );
        }
    }
}

/// A deliberate defect must never be on by default.
///
/// The feature exists so that a build can be broken on purpose. The one way
/// that could reach an image nobody meant to break is a default feature list,
/// so this reads the manifest and refuses one — which is the same shape as the
/// other four policy lints: a rule that lives only in a comment is a rule
/// somebody edits around.
fn lint_mutations() -> Result<(), String> {
    // Every manifest, not the kernel's alone. The kernel held the only defect
    // until E0-P17 put one in `ring/`, and a check that reads one file while
    // the list it checks against covers two is a check that passes by not
    // looking — the same shape as the forged slot that was never read.
    for manifest in manifests()? {
        let rel = relative(&manifest);
        let text =
            std::fs::read_to_string(&manifest).map_err(|e| format!("could not read {rel}: {e}"))?;

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
                        "{rel} has `{feature}` in its default features.\n\n\
                         That feature is a deliberate defect. It is meant to be turned on\n\
                         for exactly the runs that require it to break something, and by\n\
                         nothing else; on by default it is the defect, shipped."
                    ));
                }
            }
        }
    }

    println!("lint-mutations: ok  ({} deliberate defect(s), none on by default)", DEFECTS.len());
    Ok(())
}

/// Every crate manifest in the workspace, `third_party` excluded.
///
/// Excluded because the licence boundary is the isolation boundary: what an
/// imported crate puts in its own default features is its business, and it is
/// reachable only over a ring.
fn manifests() -> Result<Vec<PathBuf>, String> {
    fn walk(dir: &Path, build: &Path, out: &mut Vec<PathBuf>) -> std::io::Result<()> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if path.is_dir() {
                if !matches!(name, "target" | ".git" | "third_party" | "docs") && path != build {
                    walk(&path, build, out)?;
                }
            } else if name == "Cargo.toml" {
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
/// `quota` and `beyond` are E1-B13's, and both are about a bound that moved.
/// The table is bought a page at a time out of the component's own `Untyped`
/// since RFC 0008, so *the table is full* is no longer a constant: `flood` now
/// buys the one page its untyped region can pay for and stops a page later than
/// it used to, `quota` spends that region first and stops where `flood` used
/// to, and `beyond` names slots past the end of what it bought. The difference
/// between `flood`'s count and `quota`'s is the evidence that growth is paid
/// for rather than served out of anything the frame keeps back.
///
/// `unmap` is the odd one and worth reading the kernel's side of. The other
/// nine are refused by the capability table and the process carries on. This
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
    ("flood", "derive until it has bought every slot its untyped region can pay for"),
    ("unmap", "read a page after the capability that mapped it was revoked"),
    ("state", "write to the state tree it was granted read-only"),
    ("quota", "fill its table with the untyped region already spent"),
    ("beyond", "name slots past the end of the table it bought"),
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

/// The two halves of E1-B01's exit, as boots.
///
/// A real device sets up a real virtqueue and performs a real transfer, once
/// with its destination buffer translated in the device's own IOMMU domain and
/// once with it deliberately not. The first must land bytes; the second must be
/// refused by the remapping unit and recorded in its fault registers.
///
/// Neither means anything alone, which is the same argument `mutate` makes
/// about defects and `panic_path` about endings. A refusal proves nothing if
/// the identical setup also refuses when it should not — that is a device that
/// was never started, and it is exactly what the first version of this check
/// measured before the legacy virtio path was found to bypass translation
/// entirely.
const DMA_PROVOCATIONS: &[(&str, &str)] = &[
    ("inside", "a transfer into a buffer the domain translates: it must land"),
    ("outside", "a transfer into a buffer it does not: it must fault, and nothing may land"),
];

/// Boot into a device transfer that is meant to be refused, and one that is
/// not.
///
/// Both must end at 33 — the transfer happened or was refused, and the kernel
/// finished. That is the sameness with `user` and `cap` and the difference from
/// `fault`: a device addressing memory it was not given is an event the frame
/// handles, and a kernel that dies handling one has failed the property rather
/// than enforced it.
///
/// The verdict is the kernel's. It knows which half it was asked for, what the
/// unit recorded, and what is in the buffer afterwards; this reads an exit code,
/// because a harness that judged from the log alone could not tell a refused
/// transfer from a device that never answered.
fn iommu(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => DMA_PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = DMA_PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> = DMA_PROVOCATIONS
                    .iter()
                    .map(|(name, what)| format!("  {name:<8} {what}"))
                    .collect();
                return Err(format!("unknown dma provocation: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- dma={name}: {what}");
        }
        let (ending, log) = machine_devices(
            Some(&format!("dma={name}")),
            &[],
            Capture::Printed,
            BOOT_TIMEOUT,
            BOOT_MEMORY,
            DMA_DEVICE,
        )?;
        match ending {
            Ending::Exited(33) => {}
            Ending::Exited(35) => {
                return Err(format!(
                    "the kernel refused to finish after `dma={name}`. Either a device \
                     addressed memory outside its grant and the remapping unit did not \
                     stop it — which is the failure this exists to find — or a granted \
                     transfer was refused, which is the same failure pointing the other \
                     way. The serial log above says which."
                ));
            }
            Ending::Exited(0) => {
                return Err(format!(
                    "the machine reset with no output during `dma={name}`. A fault taken \
                     while the remapping unit is enabled and this kernel's own tables are \
                     under it is the frame having programmed a device wrong."
                ));
            }
            other => return Err(format!("the boot {other}; expected exit 33")),
        }

        // The exit code says the kernel agreed with itself. This says the
        // provocation happened at all — a build where the device is absent
        // would print the refusal and stop, and `dma_provocation` turns that
        // into a failed boot, so this is belt and braces rather than the
        // assertion. It is here because the *device* is a command-line option
        // and a typo in `DMA_DEVICE` is the one way this check could quietly
        // stop testing anything.
        if !log.contains("dma verdict") {
            return Err(format!(
                "`dma={name}` finished without reaching a verdict.\n\n\
                 The kernel prints one for every provocation it runs, so this means the \
                 stage did not run: no remapping unit was found, or the device this boot \
                 adds was not there to provoke."
            ));
        }
        println!("\ndma={name}: the kernel reached its own verdict and finished");
    }

    if all {
        println!(
            "\nboth halves held: a granted transfer landed, and one outside the grant was a \
             fault rather than a corruption"
        );
    }
    Ok(())
}

/// The two halves of E1-B02's exit, as boots.
///
/// A driver that lives outside the frame — `user/virtio-blk`, a crate that
/// forbids `unsafe` — brings a real virtio-blk device up through four granted
/// register windows; a client registers a buffer set and writes a sector to it
/// through a ring, then reads that sector back into a *different* buffer and
/// compares the bytes. The driver's own copy counter must be zero, and the
/// counter beside it — moved by the same function, on purpose, in the same boot
/// — must not be.
///
/// `outside` is the same run with the client's page taken out of the driver's
/// device domain between the write and the read, which is RFC 0024's stated
/// case: *the memory is the client's and it is entitled to take it back*. The
/// driver still holds a live registration naming it, so it hands the device a
/// descriptor pointing outside its grant, and the transaction must be a fault
/// the remapping unit records rather than a transfer into memory the driver no
/// longer has.
///
/// **That second half is the clause E1-B01's exit could not observe.** E1-B01
/// proved the property at the device with the frame's own adversary and wrote
/// down that *the word component in it belongs to E1-B02*; this is that word,
/// and `TODO.md` already records why one criterion belonging to two tasks is a
/// defect rather than a convenience.
///
/// Neither half means anything alone, which is `mutate`'s argument about
/// defects and `iommu`'s about its own two halves: a refusal proves nothing if
/// the identical setup also refuses when it should not.
///
/// Both must end at 33. A device addressing memory it was not given is an event
/// the frame handles, and a kernel that died handling one would have failed the
/// property rather than enforced it.
/// The three block-datapath halves, and what each one is asking.
///
/// `outside` and `escape` are two different questions and the list keeps them
/// apart on purpose. `outside` withdraws a translation under a descriptor the
/// driver built correctly — RFC 0024's reclaim, which is the *frame's* property.
/// `escape` takes nothing away and has the driver point past what it was
/// answered — which is the one E1-B01's exit could not observe, because it is
/// the component's own arithmetic that produces the address. A suite with only
/// the first would be claiming the second.
const BLK_PROVOCATIONS: &[(&str, &str)] = &[
    ("inside", "the client's buffer stays in the driver's grant: the sector must come back"),
    ("outside", "it is taken back before the read: the transfer must fault, and nothing may land"),
    ("escape", "the driver points the device past what it was answered: the unit must fault it"),
];

fn blk(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => BLK_PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = BLK_PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> = BLK_PROVOCATIONS
                    .iter()
                    .map(|(name, what)| format!("  {name:<8} {what}"))
                    .collect();
                return Err(format!("unknown block provocation: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- blk={name}: {what}");
        }
        let disk = blk_disk()?;
        let device = blk_device(&disk)?;
        let borrowed: Vec<&str> = device.iter().map(String::as_str).collect();
        let (ending, log) = machine_devices(
            Some(&format!("blk={name}")),
            &[],
            Capture::Printed,
            BOOT_TIMEOUT,
            BOOT_MEMORY,
            &borrowed,
        )?;
        match ending {
            Ending::Exited(33) => {}
            Ending::Exited(35) => {
                return Err(format!(
                    "the kernel refused to finish after `blk={name}`. Either a sector did \
                     not survive the round trip, or the driver copied something on the data \
                     path, or a descriptor pointing outside the driver's grant was not \
                     stopped — which is the failure this exists to find. The serial log \
                     above says which."
                ));
            }
            Ending::Exited(0) => {
                return Err(format!(
                    "the machine reset with no output during `blk={name}`. A fault taken \
                     while the remapping unit is enabled and this kernel's own tables are \
                     under it is the frame having programmed a device wrong."
                ));
            }
            other => return Err(format!("the boot {other}; expected exit 33")),
        }

        // The exit code says the kernel agreed with itself. This says the
        // datapath ran at all: the device is a command-line option, and a typo
        // in `blk_device` is the one way this check could quietly stop testing
        // anything. `blk_datapath` turns an absent device into a failed boot,
        // so this is belt and braces rather than the assertion.
        if !log.contains("blk verdict") {
            return Err(format!(
                "`blk={name}` finished without reaching a verdict.\n\n\
                 The kernel prints one for every run it makes, so this means the stage did \
                 not run: no remapping unit was found, or the device this boot adds was not \
                 there to drive."
            ));
        }
        println!("\nblk={name}: the kernel reached its own verdict and finished");
    }

    if all {
        println!(
            "\nall three halves held: a sector went out and came back through a ring with \
             nothing copied; the same path with the client's grant withdrawn was a fault \
             rather than a corruption; and the driver reaching past what its client's \
             registration answered was faulted at the address it invented"
        );
    }
    Ok(())
}

/// The three halves of `E1-B03`'s exit, as boots.
///
/// A **second** driver that lives outside the frame — `user/virtio-net`, a crate
/// that forbids `unsafe` — brings a real virtio-net device up through four
/// granted register windows; a client registers a buffer set, posts one receive
/// buffer through a ring, puts a hand-formed address-resolution request on the
/// link, and requires the reply to land in the registered buffer. The driver's
/// own copy counter must be zero, and the counter beside it — moved by the same
/// function, on purpose, in the same boot — must not be.
///
/// # Why there are three and what each one is for
///
/// `silent` is the control and it is the reason `inside` means anything. It is
/// the identical client with the transmit removed: the buffer is posted and
/// nothing is sent, and nothing may land. Without it, *a frame arrived* is
/// satisfied by any link with traffic on it, and this suite would be reporting
/// the emulator's backend rather than this driver's transmit.
///
/// `escape` is the isolation half, and it is the block datapath's `escape` in
/// the other direction. There, the driver points the device at memory it was not
/// granted and the device *reads* it. Here the displacement is applied to a
/// **receive** descriptor, so what the remapping unit has to refuse is the device
/// **writing** into memory the component never held — at a moment nothing in this
/// system chose, for as long as the buffer stays posted. Same provocation, and
/// the consequence of an unrefused one is not the same consequence.
///
/// # What this does not show, before somebody reads more into it
///
/// One frame, one protocol, one backend, no stack, and no measurement of
/// anything. It cannot show that the transmit was *delivered* — virtio-net
/// answers a transmit with no status at all, which `sim/src/net.rs` models
/// deliberately — so the only evidence here that a frame left the machine is
/// that something outside it answered. `restrict=on` means that something is the
/// emulator's own user-mode stack and never a real network.
///
/// All three must end at 33. A device addressing memory it was not given is an
/// event the frame handles, and a kernel that died handling one would have failed
/// the property rather than enforced it.
const NET_PROVOCATIONS: &[(&str, &str)] = &[
    ("inside", "a frame goes out and the answer must land in a registered buffer"),
    ("silent", "the same client sends nothing: nothing may arrive, and the buffer comes back"),
    ("escape", "the driver points the device past what it was answered, on the receive side"),
];

fn net(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => NET_PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = NET_PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> = NET_PROVOCATIONS
                    .iter()
                    .map(|(name, what)| format!("  {name:<8} {what}"))
                    .collect();
                return Err(format!("unknown network provocation: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- net={name}: {what}");
        }
        let (ending, log) = machine_devices(
            Some(&format!("net={name}")),
            &[],
            Capture::Printed,
            BOOT_TIMEOUT,
            BOOT_MEMORY,
            NET_DEVICE,
        )?;
        match ending {
            Ending::Exited(33) => {}
            Ending::Exited(35) => {
                return Err(format!(
                    "the kernel refused to finish after `net={name}`. Either a frame did not \
                     survive the round trip, or the driver copied something on the data path, \
                     or a receive descriptor pointing outside the driver's grant was not \
                     stopped, or a posted buffer was not given back — which is the failure \
                     this exists to find. The serial log above says which."
                ));
            }
            Ending::Exited(0) => {
                return Err(format!(
                    "the machine reset with no output during `net={name}`. A fault taken \
                     while the remapping unit is enabled and this kernel's own tables are \
                     under it is the frame having programmed a device wrong."
                ));
            }
            other => return Err(format!("the boot {other}; expected exit 33")),
        }

        // The exit code says the kernel agreed with itself. This says the
        // datapath ran at all: the device is a command-line option, and a typo
        // in `NET_DEVICE` is the one way this check could quietly stop testing
        // anything.
        if !log.contains("net verdict") {
            return Err(format!(
                "`net={name}` finished without reaching a verdict.\n\n\
                 The kernel prints one for every run it makes, so this means the stage did \
                 not run: no remapping unit was found, or the device this boot adds was not \
                 there to drive."
            ));
        }
        println!("\nnet={name}: the kernel reached its own verdict and finished");
    }

    if all {
        println!(
            "\nall three halves held: a frame went out and the answer came back into a \
             registered buffer with nothing copied; the identical client that sent nothing \
             received nothing, so the first half measured a reply rather than a link; and \
             the driver reaching past what its client's registration answered — on the \
             direction the device writes — was faulted at the address it invented"
        );
    }
    Ok(())
}

/// The display device `cargo xtask gpu` adds, and the one the machine loses to
/// make room for it.
///
/// `-vga none` is the half a reader will not expect. The q35 machine adds a
/// standard VGA adapter by default, and QEMU numbers displays in the order the
/// devices were created — so with the default adapter still there the display
/// controller this check drives would be console **one**, and a screen capture
/// with no console named takes console zero. That would capture a blank VGA
/// text screen on every half of this check, on which the pattern never appears,
/// and the two halves that must *not* show the pattern would pass for a reason
/// that has nothing to do with them. Naming the console in the capture would
/// work equally well and would be one more number to get wrong; taking the other
/// adapter away means there is only one display in the machine and it is the one
/// under test.
///
/// Two of the options are `DMA_DEVICE`'s and the reasoning is not repeated:
/// `disable-legacy=on` forces the modern register layout, and
/// `iommu_platform=on` is the device half of the feature bit that routes the
/// device's transfers through the remapping unit. On a *display* controller
/// getting that wrong is worse than a green test, and worse than it is on either
/// of the other two devices: what this device reads it puts on a screen, so a
/// transfer that bypassed translation would be a page of somebody's memory shown
/// to whoever is looking at the machine.
///
/// There is deliberately no `-display` option here. `machine_devices` passes
/// `-display none` on every boot in this file, and it is worth saying plainly
/// that it does **not** mean *no framebuffer*: QEMU still models the display and
/// still keeps a surface for it, and `screendump` reads that surface whether or
/// not anybody is drawing it in a window. What `-display none` removes is a user
/// interface, which is exactly what a check must not depend on.
const GPU_DEVICE: &[&str] =
    &["-vga", "none", "-device", "virtio-gpu-pci,disable-legacy=on,iommu_platform=on"];

/// The line the kernel prints when the picture is on the display and the machine
/// is about to hold still.
///
/// The harness waits for this and not for the verdict, because the verdict comes
/// first: a boot whose datapath failed has already exited by then, so a capture
/// is only ever taken from a run that reached its own verdict. `kernel/src/main.rs`
/// prints them in that order for exactly this reason.
const GPU_MARKER: &str = "gpu display";

/// The byte the harness writes back when it has looked.
///
/// One byte on the serial port, which the kernel polls for. Any byte would do
/// and the value is not read; what matters is that *something* arrived, which is
/// the only thing the guest needs to know. `kernel/src/arch/x86_64/serial.rs`
/// argues why a kernel that only ever printed now reads.
const GPU_ACK: &[u8] = b"k\n";

/// The three halves, and the middle one is what makes the first mean anything.
///
/// `inside` puts a client's pixels on a scanout through a ring and the harness
/// must find them on the host's display. `blank` is the identical client with
/// the submission removed — the pixels are in guest memory the whole time and
/// must not reach the display, which is a sharper control than a client that
/// wrote nothing. `escape` submits the same entry and has the driver point the
/// device one page past what the registration answered, so the remapping unit
/// must fault a **read** and nothing may appear.
///
/// All three must end at 33. A device reading memory it was not given is an
/// event the frame handles, and a kernel that died handling one would have
/// failed the property rather than enforced it.
const GPU_PROVOCATIONS: &[(&str, &str)] = &[
    ("inside", "a client's pixels reach the host's display through a ring"),
    ("blank", "the same pixels, submitted to nothing, must not reach it"),
    ("escape", "the driver points the device past what it was answered, at what a display reads"),
];

/// What one watched boot produced.
struct Watched {
    ending: Ending,
    log: String,
    /// The screen capture, when the marker appeared and the monitor answered.
    shot: Option<PathBuf>,
}

/// A frame captured from the emulator, as a screen capture reports it.
struct Frame {
    /// Unit: pixels.
    width: u32,
    /// Unit: pixels.
    height: u32,
    /// Unit: none — an FNV-1a-64 digest over the red, green and blue bytes.
    hash: u64,
}

/// FNV-1a over sixty-four bits.
///
/// The second of two copies, and the first is `kernel/src/gpu.rs`'s. A hash is
/// duplicated here rather than a *pattern*, and that is the whole design of this
/// check: the kernel knows what it drew and how a display reports it, this side
/// knows neither, and what crosses between them is one number. A wrong belief
/// about the pixel layout on the kernel's side therefore produces a capture that
/// does not match rather than two sides agreeing with each other.
///
/// Not a cryptographic hash, and the cost is stated rather than hidden: a
/// deliberate collision is constructible. Nothing on the far side of this
/// comparison is chosen by anybody — the adversary is a defect — and the day
/// something adversarial writes to a framebuffer this is a comparison over the
/// bytes and not over a digest.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Read a binary portable-pixmap: the format QEMU's screen capture writes.
///
/// `P6`, three ASCII numbers, one whitespace byte, then three bytes per pixel in
/// row-major order. Parsed by hand for the reason this tree parses every format
/// by hand — a dependency for eleven lines is a dependency — and refused rather
/// than guessed at: a capture this function cannot read is a capture nothing
/// should be concluded from.
///
/// # Errors
///
/// A magic that is not `P6`, a header that does not hold three numbers, a
/// maximum value other than 255, or a body that is not three bytes per pixel.
fn read_ppm(bytes: &[u8]) -> Result<Frame, String> {
    let mut at = 0usize;
    let token = |bytes: &[u8], at: &mut usize| -> Result<String, String> {
        while bytes.get(*at).is_some_and(|b| b.is_ascii_whitespace()) {
            *at += 1;
        }
        // A comment runs to the end of its line. QEMU writes none; the format
        // allows them, and skipping them is three lines against a capture that
        // would otherwise be unreadable for a reason nobody would guess.
        while bytes.get(*at) == Some(&b'#') {
            while bytes.get(*at).is_some_and(|b| *b != b'\n') {
                *at += 1;
            }
            while bytes.get(*at).is_some_and(|b| b.is_ascii_whitespace()) {
                *at += 1;
            }
        }
        let start = *at;
        while bytes.get(*at).is_some_and(|b| !b.is_ascii_whitespace()) {
            *at += 1;
        }
        if start == *at {
            return Err("the capture ended in the middle of its header".to_string());
        }
        String::from_utf8(bytes.get(start..*at).unwrap_or_default().to_vec())
            .map_err(|_| "the capture's header is not text".to_string())
    };

    if token(bytes, &mut at)? != "P6" {
        return Err("the capture is not a binary portable pixmap".to_string());
    }
    let number = |text: String| -> Result<u32, String> {
        text.parse::<u32>().map_err(|_| format!("`{text}` is not a number in the capture's header"))
    };
    let width = number(token(bytes, &mut at)?)?;
    let height = number(token(bytes, &mut at)?)?;
    let maximum = number(token(bytes, &mut at)?)?;
    if maximum != 255 {
        return Err(format!("the capture reports {maximum} as its maximum value, not 255"));
    }
    // Exactly one whitespace byte separates the header from the body, and it is
    // consumed rather than skipped: skipping would eat a pixel whose red channel
    // happens to be a space.
    at += 1;
    let body = bytes.get(at..).ok_or("the capture has a header and no pixels")?;
    let needed = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or("the capture claims more pixels than could exist")?;
    if body.len() < needed {
        return Err(format!(
            "the capture claims {width}x{height} and carries {} of the {needed} bytes that needs",
            body.len()
        ));
    }
    let body = body.get(..needed).unwrap_or_default();
    Ok(Frame { width, height, hash: fnv1a(body) })
}

/// What the kernel said it drew, read out of its own boot log.
///
/// Parsed rather than assumed, and this is the one place the harness learns
/// anything about the picture. It holds no copy of the pattern, no copy of the
/// geometry and no copy of the pixel layout — `kernel/src/gpu.rs` holds all
/// three — so a change to any of them on that side changes this line and is
/// checked against a capture rather than against a second belief.
///
/// # Errors
///
/// A log with no such line, or one this cannot read.
fn gpu_claim(log: &str) -> Result<Frame, String> {
    let line = log
        .lines()
        .find(|line| line.contains(GPU_MARKER))
        .ok_or("the boot printed no display line")?;
    // `  gpu display   16 x 16 pixels, client rgb fnv1a 0x????????????????`
    let fields: Vec<&str> = line.split_whitespace().collect();
    let at = |index: usize| -> Result<&str, String> {
        fields.get(index).copied().ok_or_else(|| format!("the display line is too short: {line}"))
    };
    let number = |text: &str| -> Result<u32, String> {
        text.parse::<u32>().map_err(|_| format!("`{text}` is not a number in: {line}"))
    };
    let width = number(at(2)?)?;
    let height = number(at(4)?)?;
    let digits = at(fields.len() - 1)?.trim_start_matches("0x");
    let hash = u64::from_str_radix(digits, 16)
        .map_err(|_| format!("`{digits}` is not a hash in: {line}"))?;
    Ok(Frame { width, height, hash })
}

/// Ask the emulator's monitor one question and wait for its answer.
///
/// The monitor speaks a line of JSON per message. This writes one and reads
/// until a line carrying `"return"` or `"error"`, which is a substring test
/// rather than a parser — deliberately, because the alternative is a JSON reader
/// in a build tool for three messages, and because everything this needs to know
/// is whether the emulator did what it was asked.
///
/// Events arrive on the same connection and are stepped over by the same test,
/// which is why the loop reads until it finds an answer rather than reading one
/// line.
///
/// # Errors
///
/// A monitor that refused, or that stopped answering inside the read timeout its
/// caller set.
fn monitor_ask(
    reader: &mut std::io::BufReader<std::net::TcpStream>,
    writer: &mut std::net::TcpStream,
    request: &str,
) -> Result<(), String> {
    use std::io::{BufRead, Write};
    writeln!(writer, "{request}").map_err(|e| format!("writing to the monitor: {e}"))?;
    writer.flush().map_err(|e| format!("flushing the monitor: {e}"))?;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).map_err(|e| format!("reading the monitor: {e}"))?;
        if read == 0 {
            return Err(format!("the monitor closed before answering `{request}`"));
        }
        if line.contains("\"error\"") {
            return Err(format!("the monitor refused `{request}`: {}", line.trim()));
        }
        if line.contains("\"return\"") {
            return Ok(());
        }
    }
}

/// Capture the emulator's framebuffer to `into`.
///
/// Three messages: the greeting the monitor sends unprompted, the handshake it
/// requires before it will take a command, and the capture itself. The file is
/// written by the **emulator** and not by this process, which is why the path
/// crosses the connection as text.
///
/// # Errors
///
/// Anything [`monitor_ask`] refuses, and a path that is not valid UTF-8.
fn monitor_capture(stream: &std::net::TcpStream, into: &Path) -> Result<(), String> {
    use std::io::BufRead;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| format!("setting a bound on the monitor: {e}"))?;
    let mut reader = std::io::BufReader::new(
        stream.try_clone().map_err(|e| format!("cloning the monitor connection: {e}"))?,
    );
    let mut writer =
        stream.try_clone().map_err(|e| format!("cloning the monitor connection: {e}"))?;

    let mut greeting = String::new();
    reader.read_line(&mut greeting).map_err(|e| format!("reading the monitor's greeting: {e}"))?;
    if !greeting.contains("QMP") {
        return Err(format!("the monitor did not greet this connection: {}", greeting.trim()));
    }
    monitor_ask(&mut reader, &mut writer, "{\"execute\":\"qmp_capabilities\"}")?;

    if let Some(parent) = into.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", relative(parent)))?;
    }
    // Removed first, so that a capture the emulator refused to write cannot be
    // read as this run's. A stale file from a previous run is exactly the shape
    // of pass this whole family of commands exists to refuse.
    let _ = std::fs::remove_file(into);
    let path = into.to_str().ok_or("the capture path is not valid UTF-8")?;
    monitor_ask(
        &mut reader,
        &mut writer,
        &format!("{{\"execute\":\"screendump\",\"arguments\":{{\"filename\":\"{path}\"}}}}"),
    )
}

/// Boot the display datapath and watch it from outside.
///
/// # Why this is not [`machine_devices`]
///
/// Because it has to act *while the machine is running*. Every other check in
/// this file spawns a boot, waits for it to end and then reads its log; this one
/// reads the log as it arrives, and when the kernel says the picture is on the
/// display it captures the emulator's framebuffer and writes a byte back to tell
/// the kernel it may carry on. Both halves of that are the same fact: a scanout
/// is on the far side of the emulator, so the only moment it can be observed is
/// while the emulator exists.
///
/// [`emulator`] is what keeps the machine described once. What is here is the
/// spawning, the watching and the two messages.
///
/// # Errors
///
/// A boot that could not be started, a monitor that never connected, or a
/// capture the emulator refused.
fn watched_boot(append: &str, shot: &Path) -> Result<Watched, String> {
    use std::io::{BufRead, Write};

    // Bound before the emulator is spawned and held for the whole run, so there
    // is no window in which the port is free for something else to take: the
    // emulator connects *out* to this listener rather than listening itself,
    // which is what makes the port safe to choose here and impossible to race.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")
        .map_err(|e| format!("could not open a monitor socket: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("could not read the monitor socket's port: {e}"))?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|e| format!("could not poll the monitor socket: {e}"))?;
    let monitor = format!("tcp:127.0.0.1:{port}");

    let mut devices: Vec<&str> = GPU_DEVICE.to_vec();
    devices.push("-qmp");
    devices.push(&monitor);

    let mut qemu = emulator(Some(append), &[], BOOT_MEMORY, &devices)?;
    qemu.stdout(Stdio::piped());
    // The other direction, which no other boot in this file needs: the byte that
    // says the capture has been taken.
    qemu.stdin(Stdio::piped());
    let mut child = qemu.spawn().map_err(|e| format!("could not run qemu-system-x86_64: {e}"))?;
    let mut stdin = child.stdin.take();

    // A line at a time down a channel rather than a string read to the end,
    // because this side has to notice a line *before* the process ends. The
    // thread is what keeps a full pipe from deadlocking the reader, which is the
    // same reason `machine_devices` has one.
    let (lines, arriving) = std::sync::mpsc::channel::<String>();
    let reader = child.stdout.take().map(|out| {
        std::thread::spawn(move || {
            for line in std::io::BufReader::new(out).lines() {
                let Ok(line) = line else { break };
                if lines.send(line).is_err() {
                    break;
                }
            }
        })
    });

    let mut log = String::new();
    let mut captured = None;
    let mut connection: Option<std::net::TcpStream> = None;
    let mut trouble: Option<String> = None;

    // The same counted sleeps `machine_devices` uses, and its comment is the
    // argument: sleep drift errs towards waiting too long, and only one of the
    // two failure directions is survivable.
    const TICK_MS: u64 = 20;
    let mut ticks = BOOT_TIMEOUT.saturating_mul(1000 / TICK_MS);

    let ending = loop {
        if connection.is_none()
            && let Ok((stream, _)) = listener.accept()
        {
            connection = Some(stream);
        }

        let mut marker = false;
        while let Ok(line) = arriving.try_recv() {
            let line = line.trim_end_matches('\r').to_string();
            println!("{line}");
            log.push_str(&line);
            log.push('\n');
            if line.contains(GPU_MARKER) {
                marker = true;
            }
        }

        if marker && captured.is_none() && trouble.is_none() {
            match connection.as_ref() {
                Some(stream) => match monitor_capture(stream, shot) {
                    Ok(()) => captured = Some(shot.to_path_buf()),
                    Err(why) => trouble = Some(why),
                },
                None => {
                    trouble = Some("the emulator's monitor never connected".to_string());
                }
            }
            // The byte back, and it goes whether or not the capture worked: a
            // machine left holding still for a harness that has given up is a
            // boot that ends on its own bound minutes later, which turns one
            // failure into a slow one.
            if let Some(pipe) = stdin.as_mut() {
                let _ = pipe.write_all(GPU_ACK);
                let _ = pipe.flush();
            }
        }

        match child.try_wait().map_err(|e| format!("waiting for qemu: {e}"))? {
            Some(status) => break status.code().map_or(Ending::Signalled, Ending::Exited),
            None if ticks == 0 => {
                let _ = child.kill();
                let _ = child.wait();
                break Ending::TimedOut(BOOT_TIMEOUT);
            }
            None => {
                ticks -= 1;
                std::thread::sleep(Duration::from_millis(TICK_MS));
            }
        }
    };

    // Whatever the reader had not handed over yet. The sender is dropped when
    // its thread ends, which is what stops this loop.
    while let Ok(line) = arriving.recv() {
        let line = line.trim_end_matches('\r').to_string();
        println!("{line}");
        log.push_str(&line);
        log.push('\n');
    }
    if let Some(handle) = reader {
        let _ = handle.join();
    }
    if let Some(why) = trouble {
        return Err(why);
    }
    Ok(Watched { ending, log, shot: captured })
}

/// The exit criterion of E1-B04, as a command.
///
/// # What it asserts, and why one of the assertions is not the kernel's
///
/// Every other datapath check in this file reads an exit code and one line of a
/// boot log, because the kernel reached its own verdict and a harness that
/// second-guessed it would be a second implementation of the check. That holds
/// here for everything except the picture. A scanout has no read-back command,
/// so nothing inside the machine can observe what is on the display: the
/// kernel's verdict covers the commands the display accepted, the client's
/// buffer coming back unwritten, the copy counter, the refused grant and the
/// remapping unit's fault record, and it stops there.
///
/// So this captures the emulator's framebuffer while the boot holds still, and
/// compares it with a number the kernel printed: the hash of the client's own
/// pixels in the order a screen capture reports them. The harness holds no copy
/// of the pattern and no copy of the pixel layout, which is what makes the
/// comparison a comparison rather than two sides agreeing with each other.
///
/// The control is `blank`: the identical client with the submission removed. The
/// pixels are in guest memory for the whole of that boot and the capture must
/// **not** match — without it, `inside` would establish that a picture appeared
/// and not that this ring put it there.
fn gpu(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => GPU_PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = GPU_PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> = GPU_PROVOCATIONS
                    .iter()
                    .map(|(name, what)| format!("  {name:<8} {what}"))
                    .collect();
                return Err(format!("unknown display provocation: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- gpu={name}: {what}");
        }
        let shot = target_dir().join("gpu").join(format!("{name}.ppm"));
        let watched = watched_boot(&format!("gpu={name}"), &shot)?;

        match watched.ending {
            Ending::Exited(33) => {}
            Ending::Exited(35) => {
                return Err(format!(
                    "the kernel refused to finish after `gpu={name}`. Either the display did \
                     not accept the commands it was sent, or the driver copied something on \
                     the data path, or a backing pointing outside the driver's grant was not \
                     stopped, or the client's own buffer came back written — which is the \
                     failure this exists to find. The serial log above says which."
                ));
            }
            Ending::Exited(0) => {
                return Err(format!(
                    "the machine reset with no output during `gpu={name}`. A fault taken \
                     while the remapping unit is enabled and this kernel's own tables are \
                     under it is the frame having programmed a device wrong."
                ));
            }
            other => return Err(format!("the boot {other}; expected exit 33")),
        }

        if !watched.log.contains("gpu verdict") {
            return Err(format!(
                "`gpu={name}` finished without reaching a verdict.\n\n\
                 The kernel prints one for every run it makes, so this means the stage did \
                 not run: no remapping unit was found, or the device this boot adds was not \
                 there to drive."
            ));
        }
        // The kernel was told the capture had been taken. Without this the boot
        // would still pass, having waited a minute and said so — and a check
        // that quietly stopped talking to the machine it is watching is a check
        // measuring a timeout.
        if !watched.log.contains("the harness acknowledged the frame") {
            return Err(format!(
                "`gpu={name}` never saw this harness acknowledge its frame, so the boot \
                 waited out its own bound. The capture, if there was one, was taken from a \
                 machine that had already given up on being watched."
            ));
        }

        let claimed = gpu_claim(&watched.log)?;
        let Some(shot) = watched.shot else {
            return Err(format!("`gpu={name}` produced no screen capture"));
        };
        let bytes = std::fs::read(&shot)
            .map_err(|e| format!("reading the capture at {}: {e}", relative(&shot)))?;
        let seen = read_ppm(&bytes)?;

        println!(
            "\ngpu={name}: the kernel drew {} x {} with hash {:#018x}; the display showed \
             {} x {} with hash {:#018x}",
            claimed.width, claimed.height, claimed.hash, seen.width, seen.height, seen.hash,
        );

        if *name == "inside" {
            if seen.width != claimed.width || seen.height != claimed.height {
                return Err(format!(
                    "the display is {} x {} and the client drew {} x {}. A capture at the \
                     emulator's own default size is a display no scanout was ever set on, \
                     which is a driver that did not reach `SET_SCANOUT`.",
                    seen.width, seen.height, claimed.width, claimed.height,
                ));
            }
            if seen.hash != claimed.hash {
                return Err(format!(
                    "the display is the size the client asked for and does not hold the \
                     client's pixels: {:#018x} on the screen against {:#018x} in the \
                     buffer.\n\n\
                     The geometry matching and the contents not is the interesting failure: \
                     the scanout was set, so the resource exists and is the right shape, and \
                     what did not happen is the transfer or the flush. A driver that dropped \
                     `TRANSFER_TO_HOST_2D` produces exactly this.",
                    seen.hash, claimed.hash,
                ));
            }
        } else if seen.hash == claimed.hash {
            return Err(format!(
                "`gpu={name}` put the client's pixels on the display. That is the control \
                 failing, and it fails the whole check rather than one third of it: if the \
                 pattern reaches the screen on a boot that submitted nothing to the ring — \
                 or on one whose backing the remapping unit refused — then `gpu=inside` \
                 established that a picture appeared and not that this driver put it there."
            ));
        }
        println!("gpu={name}: the kernel reached its own verdict and the display agreed");
    }

    if all {
        println!(
            "\nall three halves held: a client's pixels went out through one ring, six \
             display commands and a device that reads guest memory, and came back off the \
             emulator's own framebuffer byte for byte; the identical client that submitted \
             nothing put nothing on the screen, so the first half measured a datapath and \
             not a display; and the driver reaching one page past what its client's \
             registration answered — on the direction a display reads — was faulted at the \
             address it invented, refused by the device, and drew nothing"
        );
    }
    Ok(())
}

/// What `E1-B06` still cannot show, declared as a set rather than left in a
/// paragraph.
///
/// Two rows, and they are RFC 0025's bounds 3 and 4 — the two of the four this
/// tree does not run. Bound 1 (a callee is never promoted above the class it
/// was admitted for) and bound 2 (a caller may not carry a class it does not
/// hold) are what `cargo xtask deadline` boots.
///
/// The first row is the time floor. RFC 0025 bounds an inherited deadline
/// below by *arrival plus the callee's floor*, and the arrival a component can
/// supply is zero: RFC 0004 gives a component no clock, so
/// `user/virtio-blk/src/component.rs` passes a literal zero into
/// `Driver::admit` and the floor is measured from the channel epoch's origin
/// rather than from when the entry turned up. Bound 3 is therefore **not
/// exercised by any boot in this tree**, and an absurd deadline still sorts
/// ahead of an honest one at this driver.
///
/// The needle is that literal. The day a component can read a clock, the call
/// stops being `admit(&entry, 0)`, this goes red, and whoever closed it is told
/// which documents describe a tree that no longer exists. `CHAOS_GAP` and
/// `OWED_REVERSALS` are the precedents and `gap_holds` is the same reading.
///
/// It is deliberately *not* a claim that the ordering is untested — `cargo
/// xtask deadline` orders a real device queue by class and by deadline, and
/// that is bounds 1 and 2. **Two of RFC 0025's four bounds, and only two.**
/// This paragraph used to say *bounds 1, 2 and 4*, which was wrong in the one
/// place a wrong sentence costs most: this constant exists so that three green
/// boots cannot imply four bounds, so a sentence inside it claiming a bound
/// nobody runs is that failure with the mechanism's own name on it.
/// [`DEADLINE_DEPTH_GAP`] is the fourth bound, declared rather than asserted.
const DEADLINE_GAP: &[Gap] = &[
    (
        "user/virtio-blk/src/component.rs",
        "driver.admit(&entry, 0)",
        "RFC 0025 bound 3: a component has no clock, so the deadline floor is measured from \
         the channel epoch's origin and not from arrival, and no boot exercises it",
        "docs/rfc/0049's *What would reverse this*; docs/rfc/0025's bound 3; \
         user/virtio-blk/src/pending.rs's `Admission::floor`; kernel/src/blk.rs's \
         `HARD_DEADLINE_NS`, which is two orders of magnitude outside the floor so that a \
         shortfall on this boot has exactly one cause; sim/src/dev.rs's `FLOOR_NS`, which is \
         the model exercising what no boot here does",
    ),
    DEADLINE_DEPTH_GAP,
];

/// RFC 0025's fourth bound, and why it is checked by an **absence**.
///
/// Bound 4 is the depth decay: a caller's urgency reaches
/// `f_abi::deadline::MAX_DEPTH` rings and then ends, which is the bound that
/// stops a component claiming urgency forever. Nothing in this tree runs it,
/// and the reason is structural rather than an oversight: there is no *chain*
/// here. Every entry that reaches a scheduler was written at depth zero by
/// whoever originated it — `kernel/src/blk.rs` writes `pack(class::HARD, 0)`
/// and says why in the comment above it — the block driver is a leaf that
/// answers its client and submits to nobody, and `Inherited::rank` does not
/// read the depth. So `inherit` runs the decay arithmetic on every entry in
/// this tree and the result of it changes nothing.
///
/// The needle is the accessor a forwarder would have to call, and
/// [`deadline_depth_unforwarded`] takes two readings of it. The accessor still
/// exists — a bound nobody can reach is not the same thing as a bound somebody
/// deleted — and **nothing outside `abi/` calls it**. `abi/` is excluded
/// because that is where the rule is defined and unit-tested rather than
/// applied, and because there are no services in it to forward anything.
///
/// *Who closes it.* The cheap half is a `sim` scenario with two services in a
/// chain, whose clock is the model's own; the expensive half is a second
/// component that submits downstream on a caller's behalf, which is `E1-B05`'s
/// supervisor or `E1-B07`'s admission path. `docs/test-taxonomy.toml`'s
/// `deadline-inheritance-unbounded` row is the same statement in the tree's own
/// map, and it stays a gap until one of those lands.
const DEADLINE_DEPTH_GAP: Gap = (
    "abi/src/deadline.rs",
    "pub const fn class_field(&self)",
    "RFC 0025 bound 4: nothing outside abi/ forwards an inherited class, so every entry a \
     scheduler here sees is at depth zero and the decay never triggers",
    "docs/rfc/0049's *What would reverse this*; docs/rfc/0025's bound 4; \
     docs/test-taxonomy.toml's deadline-inheritance-unbounded row, which is a gap on \
     purpose; abi/src/deadline.rs's `the_depth_bound_is_enforced`, which is where the bound \
     is exercised today and it is a unit test",
);

/// What a service forwarding an inherited class downstream has to call.
const FORWARDED_CLASS: &str = "class_field(";

/// [`DEADLINE_DEPTH_GAP`], both readings.
///
/// # Errors
///
/// The accessor gone — the bound deleted rather than reached — or a caller of
/// it outside `abi/`, which is the gap closing and a red build on purpose.
fn deadline_depth_unforwarded() -> Result<(), String> {
    gap_holds("DEADLINE_DEPTH_GAP", &[DEADLINE_DEPTH_GAP])?;
    let mut callers = Vec::new();
    for path in rust_sources()? {
        let rel = relative(&path);
        if rel.starts_with("abi/") {
            continue;
        }
        let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {rel}: {e}"))?;
        // Comments stripped and `#[cfg(test)]` a stop, for [`code_mentions`]'s
        // reason: the sentence saying that nothing forwards a class must not be
        // the line that reports something does.
        if code_mentions(&text, FORWARDED_CLASS) > 0 {
            callers.push(rel);
        }
    }
    if callers.is_empty() {
        return Ok(());
    }
    let (_, needle, _, describes) = DEADLINE_DEPTH_GAP;
    Err(format!(
        "`{needle}` is called outside abi/, in:\n\x20  {}\n\n\
         That is RFC 0025's fourth bound becoming reachable, which is good news and a red\n\
         build on purpose: a service forwards an inherited class, so the depth decay now\n\
         has something to decay and `DEADLINE_DEPTH_GAP` in xtask describes a tree that no\n\
         longer exists.\n\n\
         These are those documents. Update them in the diff that closes it, not the one\n\
         after — and the boot or scenario showing a request lose its urgency at MAX_DEPTH\n\
         belongs in that diff too, because a bound that is reachable and unexercised is\n\
         worse than one that is neither:\n\
         \x20  {describes}",
        callers.join("\n\x20  ")
    ))
}

/// The three ordering halves, and what each one is asking.
///
/// `arrival` is a **control** and not a mode a deployment would choose, which
/// is why it is in this list rather than behind a flag: `E1-B06`'s exit is that
/// a hard-class read *overtakes* queued batch work, and an overtake with
/// nothing beside it is an array that happened to come out in a convenient
/// order. The identical client submits the identical burst to the identical
/// driver in both, and one of the two must fail to overtake.
const DEADLINE_PROVOCATIONS: &[(&str, &str)] = &[
    (
        "ordered",
        "six batch reads are queued and a hard-class read arrives last: it must be \
                 handed to the device first",
    ),
    (
        "arrival",
        "the control — the same burst, ordered by arrival: the read must come back \
                 last, or the half above compared a configuration with itself",
    ),
    (
        "unadmitted",
        "a client admitted for the batch class writes HARD: refused ADMISSION, \
                    never demoted and never served",
    ),
];

/// E1-B06's exit, as boots: *a hard-class read overtakes queued batch work in a
/// device queue, measurably*.
///
/// The same datapath `blk` boots, with the client submitting a burst instead of
/// a sector: six batch reads, then one hard-class read carrying a deadline. The
/// driver holds until it has the whole burst — a fixture, and
/// `f_virtio_blk::routing::at::HOLD` says at length why the alternative is a
/// race — and then hands the device whatever `f_abi::deadline::inherit` ranked
/// first.
///
/// **What makes it a measurement rather than an assertion** is that the number
/// is read twice from two places that cannot see each other: the frame observes
/// the order its own completion ring hands entries back in, the component
/// counts what its queue put ahead of what, and `Report::ordering_verdict`
/// requires the two to agree. And it is read on two configurations that differ
/// in one ordinal, so the half that must not overtake is what the half that must
/// is measured against.
///
/// The cost is printed rather than left out: `f_virtio_blk::pending::IN_FLIGHT`
/// is how many requests are inside the device at once, a request already there
/// cannot be overtaken by anything, and that is the granularity of every number
/// below. R12.
fn deadline(kind: Option<&str>) -> Result<(), String> {
    // The gap first, before a boot spends four minutes proving the three bounds
    // that *are* exercised. A declared quantity checked after the thing it
    // qualifies is a qualification nobody reads.
    gap_holds("DEADLINE_GAP", DEADLINE_GAP).map_err(|why| {
        format!(
            "{why}\n\nRead `DEADLINE_GAP` in xtask before deciding what to do here: it is\n\
             what this command cannot show, and it closing is good news."
        )
    })?;
    // The fourth bound's other half, which is an *absence* and so cannot be a
    // needle: nothing outside `abi/` forwards an inherited class, so the decay
    // has nothing to decay. Checked here rather than only read, because a gap
    // whose reason nobody re-derives is a gap that closes without anybody
    // noticing — which is the failure `DEADLINE_DEPTH_GAP` was added for.
    deadline_depth_unforwarded()?;
    for (file, _, why, _) in DEADLINE_GAP {
        println!("deadline gap  {file}: {why}");
    }

    let chosen: Vec<&(&str, &str)> = match kind {
        None => DEADLINE_PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = DEADLINE_PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> = DEADLINE_PROVOCATIONS
                    .iter()
                    .map(|(name, what)| format!("  {name:<11} {what}"))
                    .collect();
                return Err(format!("unknown ordering half: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- deadline={name}: {what}");
        }
        let disk = blk_disk()?;
        let device = blk_device(&disk)?;
        let borrowed: Vec<&str> = device.iter().map(String::as_str).collect();
        let (ending, log) = machine_devices(
            Some(&format!("deadline={name}")),
            &[],
            Capture::Printed,
            BOOT_TIMEOUT,
            BOOT_MEMORY,
            &borrowed,
        )?;
        match ending {
            Ending::Exited(33) => {}
            Ending::Exited(35) => {
                return Err(format!(
                    "the kernel refused to finish after `deadline={name}`. Either the \
                     hard-class read did not come back where this half requires, or the two \
                     readings of the overtake disagree, or a request was served below its \
                     class without the completion saying so. The serial log above says which."
                ));
            }
            Ending::Exited(0) => {
                return Err(format!(
                    "the machine reset with no output during `deadline={name}`. A fault taken \
                     while the remapping unit is enabled and this kernel's own tables are \
                     under it is the frame having programmed a device wrong."
                ));
            }
            other => return Err(format!("the boot {other}; expected exit 33")),
        }
        // The exit code says the kernel agreed with itself; this says the burst
        // happened at all. `blk overtake` is printed on every block boot, so a
        // run that never reached the datapath prints neither.
        if !log.contains("blk overtake") {
            return Err(format!(
                "`deadline={name}` finished without reaching the ordering stage.\n\n\
                 No remapping unit was found, or the device this boot adds was not there \
                 to drive."
            ));
        }
        println!("\ndeadline={name}: the kernel reached its own verdict and finished");
    }

    if all {
        println!(
            "\nall three halves held: a hard-class read submitted behind queued batch work \
             was handed to the device first; the identical burst ordered by arrival put it \
             last, so the first half measured an ordering rather than an array; and a client \
             that does not hold the hard class was refused it rather than served at it"
        );
    }
    Ok(())
}

/// The four halves of E1-B08's exit, as boots.
///
/// A component holds a core and schedules its own work inside it. `load` is the
/// exit criterion — *async work under load produces zero kernel entries on the
/// hot path, counted* — and the other three are what stop that zero from being
/// worthless.
///
/// `provoke` is the same run with one door call in the middle of the work loop
/// on purpose. It requires the hot-path count to be non-zero **and** to equal
/// what the component says it made: the two numbers are taken on opposite sides
/// of the boundary, so a build in which the counting had stopped publishes zero
/// on both halves and fails rather than looking clean. That is the shape
/// `blk copies` and `blk provoked` already have, and `state::node::MEMORY_FORCED`
/// before them. It is required to be *the same run* and not merely a run — the
/// same load, finished, ending the same way — because the provocation fires
/// after the first quantum and a run that stopped there would otherwise pass.
///
/// `reclaim` posts the notice from inside the timer handler after the runtime
/// has been working for a tick, so it arrives *under load* rather than at a
/// first polling point with nothing behind it. It requires the runtime to park
/// at its next allocation boundary with its own queue empty — the frame reads
/// the queue itself rather than believing the report — and requires that
/// nothing crossed the boundary while it did. It also rings this core's own
/// doorbell as the notice goes out, which is the fifth entry bucket's
/// provocation: an interrupt that is not the clock, taken at ring 3, counted,
/// and not on the hot path. That bucket did not exist when this landed and the
/// three vectors in it were counted nowhere, which RFC 0038 records.
///
/// This half is the one whose greenness depends on the machine. The notice
/// rides a timer tick, so it needs a ring-3 tick to land between a quarter of
/// the load and the end of it; under QEMU that window is tens of ticks wide and
/// on hardware it may be empty. `kernel::runtime::RECLAIM_AFTER_ITEMS` states
/// the bound and RFC 0038 argues it, so a red run on a fast machine is read as
/// the harness and not as the scheduler.
///
/// `hostile` scribbles the control ring's header before entry, which makes the
/// frame the untrustworthy peer for one boot. Safe adoption must refuse with a
/// structured error rather than fault, hang, or believe it. Without this half
/// the other three would show that adoption is *available* and not that it is
/// safe.
const RUNTIME_PROVOCATIONS: &[(&str, &str)] = &[
    ("load", "a component schedules its own work; nothing may cross the boundary until it exits"),
    ("provoke", "one crossing on purpose: the count must move, and by exactly as many"),
    ("reclaim", "the timer posts a reclaim under load; the runtime must park cleanly"),
    ("hostile", "a scribbled control ring header: adoption must refuse rather than believe it"),
];

fn runtime(kind: Option<&str>) -> Result<(), String> {
    let chosen: Vec<&(&str, &str)> = match kind {
        None => RUNTIME_PROVOCATIONS.iter().collect(),
        Some(name) => {
            let found = RUNTIME_PROVOCATIONS.iter().find(|(known, _)| *known == name);
            let Some(found) = found else {
                let list: Vec<String> = RUNTIME_PROVOCATIONS
                    .iter()
                    .map(|(name, what)| format!("  {name:<8} {what}"))
                    .collect();
                return Err(format!("unknown runtime provocation: {name}\n\n{}", list.join("\n")));
            };
            vec![found]
        }
    };

    let all = chosen.len() > 1;
    for (name, what) in chosen {
        if all {
            println!("\n--- runtime={name}: {what}");
        }
        let (ending, log) = machine_with(
            Some(&format!("runtime={name}")),
            &[],
            Capture::Printed,
            BOOT_TIMEOUT,
            BOOT_MEMORY,
        )?;
        match ending {
            Ending::Exited(33) => {}
            Ending::Exited(35) => {
                return Err(format!(
                    "the kernel refused to finish after `runtime={name}`. Either something \
                     crossed the boundary on the hot path, or the counter that would have \
                     said so stopped counting, or a runtime that was told to park abandoned \
                     work instead — which is the failure this exists to find. The serial log \
                     above says which."
                ));
            }
            Ending::TimedOut(_) => {
                return Err(format!(
                    "`runtime={name}` never finished. A component that holds a core and does \
                     not give it back is the one failure a scheduler has that a frame does \
                     not: the boot core is waiting on a mailbox word that will never move."
                ));
            }
            other => return Err(format!("the boot {other}; expected exit 33")),
        }

        // The exit code says the kernel agreed with itself. This says the stage
        // ran at all — a boot that carried no component file prints a refusal
        // and stops, and `runtime_demonstration` turns that into a failed boot,
        // so this is belt and braces rather than the assertion.
        if !log.contains("runtime verdict") {
            return Err(format!(
                "`runtime={name}` finished without reaching a verdict.\n\n\
                 The kernel prints one for every run it makes, so this means the stage did \
                 not run: no component file among the boot modules, or no core free to \
                 allocate."
            ));
        }
        println!("\nruntime={name}: the kernel reached its own verdict and finished");
    }

    if all {
        println!(
            "\nall four halves held: a component scheduled its own work with nothing crossing \
             the boundary, the counter that says so was shown to move, a reclaim arriving \
             under load was parked at an allocation boundary, and a scribbled header was \
             refused"
        );
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

/// The AArch64 target every crate that reaches the machine is compiled for.
///
/// A bare-metal target rather than a hosted one, and the choice is not a
/// convenience: it is the AArch64 target `rust-toolchain.toml` pins, so it is
/// the only one guaranteed to be installed, and it is also the honest one —
/// nothing in this workspace that runs *on the machine* has a `std` under it.
const AARCH64_TARGET: &str = "aarch64-unknown-none";

/// One workspace member, and what each architecture is asked to do with it.
///
/// # Why this is an exception table rather than a list of crates
///
/// It was a list of crate names, twice, and both times it stopped matching the
/// workspace without saying so. The host list named crates individually until
/// `f-bench` and `f-init` turned out to have tests nothing ran; the AArch64
/// cross-check named six crates beside a workspace of ten, and a crate added to
/// that workspace joined neither side of it. A list written *beside* the thing
/// it describes is a habit rather than a check — correct on the day it is
/// written, silently wrong afterwards, and the silence is the whole defect.
///
/// So the workspace is the source of truth and this is the exception table. A
/// member with no row here is a hard failure rather than a skip, which is what
/// makes a new crate architecture-checked **by default** and makes leaving one
/// out cost a sentence rather than nothing. RFC 0045.
#[derive(Debug)]
struct Portability {
    /// The package name, spelled the way the crate's own `Cargo.toml` spells it.
    krate: &'static str,
    /// Why this crate's tests do not run on both architectures, or `None` when
    /// they do. `cargo xtask test-host` is run on x86-64 *and* on the arm
    /// runner, and this field is the only thing that may keep a crate off
    /// either of them — which is what "no test is skipped on AArch64 without a
    /// recorded reason" means mechanically.
    host: Option<&'static str>,
    /// Why this crate is not compiled for [`AARCH64_TARGET`], or `None` when it
    /// is.
    bare: Option<&'static str>,
}

/// Every workspace member, and its two answers.
///
/// A row per member, checked against `Cargo.toml`'s `members` in both
/// directions by [`classify`]: a member with no row fails, and a row naming no
/// member fails. The second direction matters as much as the first — a crate
/// deleted from the workspace leaves an exclusion behind, and an exclusion
/// nobody can trace to a crate is how the *next* crate inherits a reason that
/// was written about something else.
const PORTABILITY: &[Portability] = &[
    Portability { krate: "f-abi", host: None, bare: None },
    Portability { krate: "f-env", host: None, bare: None },
    Portability { krate: "f-ring", host: None, bare: None },
    Portability {
        krate: "f-kernel",
        host: Some(
            "the frame has no host harness at all. It is `no_std` with its own entry point, \
             its own panic handler and a linker script, so `cargo test` has nowhere to put a \
             test binary — which is why the host run excludes it rather than failing on it. \
             What the frame is checked by instead is the boot suite: `run`, `orders`, `user`, \
             `cap`, `iommu`, `blk`, `runtime`, `mutate` and `panic`, every one of them in the \
             gate. *Reversal:* none that is only about testing — a frame with a host harness \
             would be a different frame.",
        ),
        bare: Some(
            "the frame is x86-64 today. `kernel/src/arch/` is one architecture — the GDT, the \
             IDT, the local APIC, `syscall`/`sysret` — and `KERNEL_TARGET` says so. \
             *Reversal:* an AArch64 frame, which is the same reversal `user/init/src/lib.rs` \
             states for `f_abi::door::call`: the day that function has a second \
             implementation this row loses both its reasons at once, and the boot suite \
             acquires a second runner rather than a second excuse.",
        ),
    },
    Portability { krate: "f-init", host: None, bare: None },
    Portability { krate: "f-store", host: None, bare: None },
    Portability { krate: "f-virtio-blk", host: None, bare: None },
    Portability { krate: "f-virtio-net", host: None, bare: None },
    Portability { krate: "f-virtio-gpu", host: None, bare: None },
    Portability {
        krate: "f-bench",
        host: None,
        bare: Some(
            "a host tool. It records distributions to a file and formats them for the claims \
             registry, so it is `std` from its first line, and `aarch64-unknown-none` has no \
             `std`. Nothing in it is compiled into the system, so nothing in it can be wrong \
             on a machine the system runs on — and its tests do run on the arm runner, which \
             is the half of the question that is about this crate. *Reversal:* a harness that \
             runs on the machine under test rather than beside it, which is what E0-P18's \
             hardware boot would need.",
        ),
    },
    Portability {
        krate: "f-sim",
        host: None,
        bare: Some(
            "a host tool, and `sim/Cargo.toml` states this in the crate itself: the simulator \
             reads a command line and writes a trace, under `std`. Its absence here is not a \
             portability gap for the same reason `f-bench`'s is not — it is not compiled into \
             the system. *Reversal:* a simulator that runs beside the frame on the machine, \
             which RFC 0032 explicitly did not choose.",
        ),
    },
    Portability {
        krate: "xtask",
        host: None,
        bare: Some(
            "the build orchestrator. It runs on the machine that *drives* a build and never \
             on the machine the build is for, it spawns processes and reads the filesystem, \
             and both of those are `std`. Its tests run on both runners like every other \
             host crate's. *Reversal:* none foreseeable; a bare-metal build driver is not a \
             thing this tree wants.",
        ),
    },
];

/// The member *paths* in a workspace manifest.
///
/// # Why the key is matched on a line and not on a substring
///
/// The first version of this read from the first occurrence of the substring
/// `members`, and cargo has a key whose name ends in it: `default-members`. A
/// manifest that grew one *above* `members` would have had its default set read
/// as the whole set — which is a shorter list, silently, and a shorter list here
/// is fewer crates checked on AArch64 with nothing going red. That is the exact
/// failure this whole change exists to remove, reintroduced by the reader for
/// it, so it is matched as a key at the start of a line and there is a test
/// below feeding it both keys in the wrong order.
///
/// The same caveat every other reader in this file carries: this is not a TOML
/// parser. It reads the shape `Cargo.toml`'s `members` is written in — a
/// bracketed list of quoted paths — and the day the workspace needs more than
/// that it needs a parser rather than a longer version of this.
fn member_paths(text: &str) -> Result<Vec<String>, String> {
    let mut rest = None;
    let mut offset = 0usize;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if let Some(after) = trimmed.strip_prefix("members")
            && after.trim_start().starts_with('=')
        {
            rest = Some(offset + (line.len() - trimmed.len()));
            break;
        }
        offset += line.len() + 1;
    }
    let at = rest.ok_or("the workspace manifest has no `members = [...]` array in it")?;
    let list = text[at..]
        .split_once('[')
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list)
        .ok_or("the workspace manifest's `members` is not a `[...]` list")?;

    // The quoted halves of a `"a", "b"` list: odd indices once split on the
    // quote character.
    let paths: Vec<String> = list
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::trim)
        .filter(|path| !path.is_empty())
        .map(str::to_string)
        .collect();
    if paths.is_empty() {
        return Err("the workspace manifest's `members` list is empty".into());
    }
    Ok(paths)
}

/// Every `members` entry in the workspace manifest, as package names.
///
/// Read from the workspace rather than restated beside it, because restating it
/// is the defect [`PORTABILITY`] exists to remove. The path in `members` is not
/// the package name — `user/init` is `f-init` — so each member's own manifest
/// is what supplies the name, which also means a crate renamed in one place and
/// not the other fails here rather than becoming an unclassified member with a
/// plausible-looking row.
fn workspace_members() -> Result<Vec<String>, String> {
    let manifest = root().join("Cargo.toml");
    let text = std::fs::read_to_string(&manifest)
        .map_err(|e| format!("reading the workspace manifest: {e}"))?;

    let mut names = Vec::new();
    for path in member_paths(&text)? {
        let member = root().join(&path).join("Cargo.toml");
        let text = std::fs::read_to_string(&member)
            .map_err(|e| format!("the workspace names `{path}` and reading its manifest: {e}"))?;
        let name = toml_table_field(&text, "package", "name")
            .ok_or_else(|| format!("{path}/Cargo.toml has no `name` under `[package]`"))?;
        names.push(name);
    }
    Ok(names)
}

/// The workspace and [`PORTABILITY`] checked against each other, in both
/// directions.
///
/// # Errors
///
/// A member with no row, or a row naming no member. Both are the same failure
/// seen from two sides: the table and the workspace have stopped describing one
/// set of crates, and every architecture check downstream is then reporting on
/// a set nobody chose.
fn classify<'a>(
    members: &[String],
    table: &'a [Portability],
) -> Result<Vec<&'a Portability>, String> {
    if members.is_empty() {
        return Err("the workspace manifest yielded no members, so there is nothing to check. \
                    An empty list here would make every architecture check trivially green, \
                    which is why it is refused rather than tolerated."
            .into());
    }

    let mut rows: Vec<&Portability> = Vec::new();
    let mut unclassified: Vec<&str> = Vec::new();
    for name in members {
        match table.iter().find(|row| row.krate == name.as_str()) {
            Some(row) => rows.push(row),
            None => unclassified.push(name.as_str()),
        }
    }
    let stale: Vec<&str> = table
        .iter()
        .map(|row| row.krate)
        .filter(|krate| !members.iter().any(|name| name == krate))
        .collect();

    if unclassified.is_empty() && stale.is_empty() {
        return Ok(rows);
    }
    Err(format!(
        "the workspace and the portability table are not about the same set of crates.\n\n\
         in the workspace and not in the table: {}\n\
         in the table and not in the workspace: {}\n\n\
         `PORTABILITY` in xtask/src/main.rs is what decides which crates are tested on both\n\
         architectures and which are compiled for {AARCH64_TARGET}. A member with no row is\n\
         refused rather than skipped, and that is the whole point of the table: a crate added\n\
         to this workspace is checked on AArch64 by default, and leaving it out costs a\n\
         sentence saying why and what would reverse it. A row naming no crate is the same\n\
         failure from the other side — an exclusion the next crate could inherit a reason\n\
         from. RFC 0045.",
        if unclassified.is_empty() { "none".to_string() } else { unclassified.join(", ") },
        if stale.is_empty() { "none".to_string() } else { stale.join(", ") },
    ))
}

/// Print what the table says, so a reader of a CI log sees the exclusions
/// rather than only the crates that ran.
///
/// An exclusion nobody reads is an exclusion nobody argues with, and the reason
/// this prints on every run — green as well as red — is that the reasons are
/// the deliverable. A list of what ran says nothing about what did not.
fn portability_report(rows: &[&Portability], which: fn(&Portability) -> Option<&'static str>) {
    let excluded: Vec<&&Portability> = rows.iter().filter(|row| which(row).is_some()).collect();
    if excluded.is_empty() {
        println!("  every workspace crate is included; nothing is excluded");
        return;
    }
    for row in excluded {
        let Some(reason) = which(row) else { continue };
        println!("  excluded: {}\n    {reason}", row.krate);
    }
}

/// The host suite, on whatever architecture this is running on.
///
/// # Why this is a command rather than a line of YAML
///
/// Because it is run on two machines and they have to run the same thing. The
/// gate named four crates on both runners while `cargo xtask test` ran the
/// whole workspace, so `f-store`, `f-virtio-blk`, `f-sim`, `f-bench` and
/// `xtask` had tests that ran on a laptop and on no runner at all — and on the
/// arm runner two of those crates were never *compiled*, which is the failure
/// this file already records happening once with `f-bench` and `f-init`. One
/// command, derived from the workspace, is what stops the two lists from
/// drifting: there is only one list now.
///
/// It prints the architecture it ran on, because that is the fact a reader of
/// the log wants and the one thing the command cannot assert about itself.
fn test_host() -> Result<(), String> {
    let rows = classify(&workspace_members()?, PORTABILITY)?;

    println!("host tests on {} — the whole workspace except:", std::env::consts::ARCH);
    portability_report(&rows, |row| row.host);
    println!();

    let mut args: Vec<&str> = vec!["test", "--workspace"];
    let mut running = 0usize;
    for row in &rows {
        if row.host.is_some() {
            args.push("--exclude");
            args.push(row.krate);
        } else {
            running += 1;
        }
    }
    // The one way this command could be green while nothing was checked: every
    // crate excluded is a `cargo test --workspace` with nothing left in it,
    // which runs no tests and exits zero. Fail closed rather than report a pass
    // over an empty set. R04.
    if running == 0 {
        return Err("every crate in the workspace is excluded from the host suite, so this \
                    would run no tests and exit zero. A pass over an empty set is the one \
                    result this command must not be able to produce."
            .into());
    }
    sh("cargo", &args)
}

/// Compile every crate that reaches the machine for AArch64.
///
/// # What this is and is not
///
/// It is the half of the AArch64 job that does not need an AArch64 machine. CI
/// runs the tests on an arm runner, which is where the ordering means anything
/// and which nothing local substitutes for. But most of what that job has ever
/// caught is not an ordering bug at all: it is code that does not *compile* off
/// x86-64, and a compile is a compile on any host. This is what would have
/// caught the one that got through — a component calling through a door whose
/// one instruction is `#[cfg(target_arch = "x86_64")]` — and it costs seconds.
///
/// A component crate belongs in it for exactly that reason: its
/// architecture-specific half is behind a `cfg`, and a `cfg` that stopped
/// covering everything is a compile error on the other target and nothing at
/// all on this one.
fn cross_check() -> Result<(), String> {
    let rows = classify(&workspace_members()?, PORTABILITY)?;

    println!("compiling for {AARCH64_TARGET} — the whole workspace except:");
    portability_report(&rows, |row| row.bare);
    println!();

    let mut args: Vec<&str> = vec!["check"];
    for row in &rows {
        if row.bare.is_none() {
            args.push("-p");
            args.push(row.krate);
        }
    }
    if args.len() == 1 {
        return Err("every crate in the workspace is excluded from the AArch64 build, so this \
                    check would pass by having nothing to do. `cargo check` with no `-p` would \
                    then build the whole workspace for a bare-metal target and fail for an \
                    unrelated reason, which is worse than refusing here."
            .into());
    }
    args.push("--target");
    args.push(AARCH64_TARGET);
    sh("cargo", &args)
}

/// What would make the architecture checks green while the property was false.
///
/// Three inputs, and each is a way this could have been a habit rather than a
/// check: a crate added to the workspace and to neither list, an exclusion left
/// behind by a crate that is gone, and a manifest reader that quietly returns
/// less than the workspace holds. All three are the same failure — the table
/// and the workspace stop being about one set of crates — and all three are red
/// here rather than green.
#[cfg(test)]
mod portability_tests {
    use super::{PORTABILITY, Portability, classify, member_paths, workspace_members};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_string()).collect()
    }

    #[test]
    fn default_members_is_not_read_as_members() {
        // The reader's own version of the bug it exists to remove. `cargo` has
        // a key ending in `members`, and a substring match takes the first one
        // — a shorter list, quietly, and a shorter list here is fewer crates
        // checked on AArch64 with nothing going red.
        let manifest = "\
[workspace]
resolver = \"3\"
default-members = [\"abi\"]
members = [\"abi\", \"env\", \"user/init\"]
";
        assert_eq!(member_paths(manifest).unwrap(), names(&["abi", "env", "user/init"]));
    }

    #[test]
    fn a_members_list_spanning_lines_is_read_whole() {
        // The shape a workspace takes the moment it outgrows one line, which is
        // the next edit anyone makes to this file.
        let manifest = "\
[workspace]
members = [
    \"abi\",
    \"user/init\",
]
";
        assert_eq!(member_paths(manifest).unwrap(), names(&["abi", "user/init"]));
    }

    #[test]
    fn a_manifest_with_no_members_is_refused_rather_than_read_as_empty() {
        // Fail closed, R04: an empty answer here makes every architecture check
        // trivially green, which is the one outcome that must not be reachable
        // by a parser going wrong.
        assert!(member_paths("[workspace]\nresolver = \"3\"\n").is_err());
        assert!(member_paths("[workspace]\nmembers = []\n").is_err());
    }

    #[test]
    fn the_workspace_this_tree_has_is_the_one_the_table_describes() {
        // The green case, and it is here so the red ones below are known to be
        // red for their own reason rather than because the shape always fails.
        let members = workspace_members().expect("the workspace manifest is readable");
        classify(&members, PORTABILITY).expect("every member has a row and every row a member");
    }

    #[test]
    fn a_member_is_named_by_its_package_and_not_by_its_path() {
        // `user/init` is `f-init`. A reader that took the path would produce a
        // member no row matches and a row no member matches — one edit, two
        // failures, neither of them the real one.
        let members = workspace_members().expect("the workspace manifest is readable");
        assert!(members.iter().any(|name| name == "f-init"), "{members:?}");
        assert!(members.iter().any(|name| name == "f-kernel"), "{members:?}");
        assert!(members.iter().any(|name| name == "xtask"), "{members:?}");
        assert!(!members.iter().any(|name| name.contains('/')), "a path reached the list");
    }

    #[test]
    fn a_crate_added_to_the_workspace_is_refused_rather_than_skipped() {
        // The whole point of the table. Before this, a crate added to the
        // workspace joined neither the host list nor the AArch64 one and
        // nothing said so.
        let mut members = workspace_members().expect("the workspace manifest is readable");
        members.push("f-newcomer".to_string());
        let refusal = classify(&members, PORTABILITY).expect_err("an unclassified member passed");
        assert!(refusal.contains("f-newcomer"), "the refusal must name the crate: {refusal}");
    }

    #[test]
    fn an_exclusion_that_names_no_crate_is_refused() {
        // The other direction, which is the one a deletion breaks: a reason
        // written about a crate that is gone is a reason the next crate with
        // that name inherits without anyone reading it.
        let table = &[
            Portability { krate: "f-abi", host: None, bare: None },
            Portability { krate: "f-departed", host: None, bare: Some("gone") },
        ];
        let refusal = classify(&names(&["f-abi"]), table).expect_err("a stale exclusion passed");
        assert!(refusal.contains("f-departed"), "the refusal must name the row: {refusal}");
    }

    #[test]
    fn a_manifest_reader_that_returns_less_than_the_workspace_is_refused() {
        // The failure mode that would otherwise be invisible: parse fewer
        // members, check fewer crates, stay green. It arrives as a table full
        // of rows naming nothing, which the direction above already refuses.
        let refusal =
            classify(&names(&["f-abi"]), PORTABILITY).expect_err("a truncated member list passed");
        assert!(refusal.contains("f-kernel"), "{refusal}");
        let empty = classify(&[], PORTABILITY).expect_err("an empty member list passed");
        assert!(empty.contains("nothing to check"), "{empty}");
    }

    #[test]
    fn every_exclusion_states_what_would_reverse_it() {
        // A reason with no reversal is a preference wearing a decision's
        // clothes — the same sentence RFC 0000's last section exists for. An
        // exclusion is a small RFC, so it owes the same thing.
        for row in PORTABILITY {
            for reason in [row.host, row.bare].into_iter().flatten() {
                assert!(
                    reason.contains("Reversal:"),
                    "{} is excluded without saying what would reverse it: {reason}",
                    row.krate
                );
            }
        }
    }
}

/// A test that runs on one architecture and not the other, with the reason
/// recorded.
///
/// Path prefix, then why. Empty today, and that is a statement rather than an
/// oversight: no test in this workspace is architecture-gated. An entry here
/// owes what an exclusion in [`PORTABILITY`] owes — a reason and a *Reversal*,
/// checked below — because a test that runs on half the runners is a claim
/// about half the machines, and E1-P11's exit is that nothing is skipped on
/// AArch64 without a recorded reason.
const ARCH_TEST_ALLOW: &[(&str, &str)] = &[];

/// Every `mod NAME;` a file declares, and whether an architecture gate stands
/// in front of it.
///
/// The declaration is what carries the gate — `#[cfg(target_arch = "x86_64")]
/// pub mod component;` in `user/init/src/lib.rs` is the shape — so the file it
/// names is compiled on one architecture and not on the other, and every
/// `#[test]` inside it is skipped on the other one without the test itself
/// saying anything at all. The declaration is the only place that gate can be
/// read, because the file it names does not carry it.
///
/// A file-scope `#![cfg(target_arch = …)]` gates the declarations below it the
/// same way, and it is tracked separately because it is not a property of the
/// next item: it does not lift when that item is passed. Reading it as an item
/// gate loses it at the first line of code after it, which is the shape review
/// found green here once.
fn file_modules(text: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut gated = false;
    let mut file_gated = false;
    let mut carry = Carry::default();
    for line in text.lines() {
        let code = strip_to_code(line, &mut carry);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("#![") {
            // An inner attribute belongs to the enclosing scope, not to the
            // item after it, so it is never cleared below. A `#![cfg]` written
            // inside an inline `mod` therefore over-reaches to the rest of the
            // file; that direction is deliberate. Over-reporting is argued with
            // in review and under-reporting is not noticed at all, and this
            // check's whole purpose is that a skipped test be said out loud.
            file_gated |= trimmed.contains("target_arch");
            continue;
        }
        if trimmed.starts_with("#[") {
            // An attribute keeps whatever gate is already pending rather than
            // replacing it: an item wearing both `#[cfg(target_arch)]` and
            // `#[cfg(feature)]` is gated by both, and reading only the one
            // nearest the item would drop the gate that matters here.
            gated |= trimmed.contains("target_arch");
            continue;
        }
        if let Some(rest) = trimmed
            .strip_prefix("pub mod ")
            .or_else(|| trimmed.strip_prefix("mod "))
            .or_else(|| trimmed.strip_prefix("pub(crate) mod "))
            && let Some(name) = rest.strip_suffix(';')
        {
            out.push((name.trim().to_string(), gated || file_gated));
        }
        gated = false;
    }
    out
}

/// A `#[test]` compiled on one architecture and not the other, found in the
/// file it is written in.
///
/// # What this reads, and what it cannot
///
/// It reads the gate as a *lexical* property: an architecture `cfg` on a test
/// function, on any block the test is written inside, or on the file itself as
/// an inner `#![cfg(target_arch = …)]`. Three shapes, and the third is the one
/// that matters most rather than least: an integration test file under `tests/`
/// is named by no `mod` declaration anywhere, so the module half of
/// [`lint_arch_tests`] can never reach it, and a file-scope attribute is the
/// only way such a file can be gated at all. `ring/tests/litmus.rs` and its
/// neighbours are exactly those files, and they are the ones CLAUDE.md's scar
/// is about. Review found this check green over that input once; the fix is
/// that a file gate is held apart from the item gate and is never cleared by
/// the line of code that follows it.
///
/// It does not see a test a macro generates, a module reached through
/// `#[path]`, or a test gated on a *feature* that is itself only ever enabled
/// on one architecture. The last of those is worth naming rather than leaving
/// implied: `user/store` and `user/virtio-blk` both write
/// `all(target_arch = "x86_64", feature = "image")`, and a crate that wrote the
/// feature alone would be gated by an architecture this reader cannot see. That
/// is this check's declared limit rather than a defect in it, and it is the
/// same kind of limit `JOIN_GAP` states: the honest move is to write the gap
/// down, because a check that claims more than it reads is worse than one that
/// says where it stops.
fn arch_gated_tests(rel: &str, text: &str) -> Vec<String> {
    let mut findings = Vec::new();
    // The brace depths at which an architecture-gated block was opened. A stack
    // rather than a flag, because a gated `mod` can contain an ungated one and
    // the gate lifts when its own block closes rather than when the first inner
    // block does. A flag here would clear the gate at the first `}` and every
    // test after it in the same module would read as ungated.
    let mut gates: Vec<usize> = Vec::new();
    let mut depth = 0usize;
    let mut gated = false;
    // The file-scope gate, held apart from the item gate and never cleared. An
    // inner attribute is a property of the scope, not of the next item, so the
    // line of code after it must not consume it — and an integration test file
    // has no other way to be gated, because nothing declares it with a `mod`.
    let mut file_gated = false;
    let mut is_test = false;
    let mut carry = Carry::default();

    for (n, line) in text.lines().enumerate() {
        let code = strip_to_code(line, &mut carry);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("#![") {
            file_gated |= trimmed.contains("target_arch");
            continue;
        }
        if trimmed.starts_with("#[") {
            gated |= trimmed.contains("target_arch");
            // `#[test]` and anything ending in `test]` alike. What matters is
            // that the item below is a test, not which harness runs it —
            // and `#[cfg(test)]`, which introduces a module rather than a test,
            // is excluded by name.
            is_test |= trimmed.ends_with("test]") && !trimmed.contains("cfg(test)");
            continue;
        }
        if is_test && (gated || file_gated || !gates.is_empty()) {
            findings.push(format!("  {rel}:{}  {trimmed}", n + 1));
        }

        let opens = trimmed.matches('{').count();
        let closes = trimmed.matches('}').count();
        if gated && opens > closes {
            gates.push(depth);
        }
        depth = (depth + opens).saturating_sub(closes);
        gates.retain(|at| *at < depth);
        gated = false;
        is_test = false;
    }
    findings
}

/// The file a `mod NAME;` in `parent` names, if there is one.
///
/// `dir/foo.rs` owns `dir/foo/NAME.rs`; a crate root or a `mod.rs` owns
/// `dir/NAME.rs`. Both spellings of a directory module are tried, because both
/// are in this tree. A declaration naming no file on disk is not an error here
/// — it is an inline module, already handled by [`arch_gated_tests`], or a
/// `#[path]` one, which this check declares it cannot see.
fn module_file(parent: &Path, name: &str) -> Option<PathBuf> {
    let dir = parent.parent()?;
    let stem = parent.file_stem().and_then(|s| s.to_str())?;
    let base =
        if matches!(stem, "lib" | "main" | "mod") { dir.to_path_buf() } else { dir.join(stem) };
    [base.join(format!("{name}.rs")), base.join(name).join("mod.rs")]
        .into_iter()
        .find(|candidate| candidate.is_file())
}

/// No test is skipped on AArch64 without a recorded reason.
///
/// # Why a source check rather than a count of what ran
///
/// The obvious check is to compare the tests the two runners collected. It
/// cannot be written here. Nothing local can run an AArch64 test binary: the
/// container carries `qemu-system-aarch64`, which would need a frame to boot
/// and the frame is x86-64, and it carries no `qemu-user`, nothing in
/// `binfmt_misc` and no `aarch64-unknown-linux-gnu` to build a hosted binary
/// for. And a comparison of two CI logs is a check that lives in neither
/// runner's job and fails in a third place. This reads the one thing that
/// decides the answer — the gate in the source — and it runs everywhere,
/// including on the laptop where the test is being written, which is the
/// moment the gate is cheap to argue with.
///
/// It is the level below [`PORTABILITY`], and it exists because that table
/// cannot see this. `cargo xtask test-host` runs the whole workspace on both
/// runners and would keep saying so while a test inside an included crate
/// quietly compiled on one of them: the job stays green, the test count on one
/// runner is smaller, and nobody reads a test count.
///
/// # What would make this green while tests were being skipped
///
/// Four inputs, three of them refused and one declared. A test under an
/// architecture-gated *item*, a test under an architecture-gated *block*, and a
/// test in a file carrying `#![cfg(target_arch = …)]` are all refused rather
/// than tolerated, and each has a case below. The third of those is here
/// because review found this check green over it: an inner attribute is a
/// property of the scope and not of the next item, and a reader that treats it
/// as an item gate drops it at the first `use` below it. It is also the only
/// gate an integration test file can carry, which made it the worst of the four
/// to be blind to.
///
/// What is left is the limit [`arch_gated_tests`] declares and cannot close by
/// reading text: a gate spelled as a *feature* that only one architecture ever
/// enables is invisible to a reader of the source, and this is a reader of the
/// source. Closing it means asking cargo to resolve features per target rather
/// than scanning, and that is a different check.
fn lint_arch_tests() -> Result<(), String> {
    let sources = rust_sources()?;
    let mut findings = Vec::new();

    // The files a gated `mod NAME;` pulls in, and everything those pull in
    // after that. A gate on a declaration reaches the whole subtree below it,
    // so stopping at the first file would miss a test one module deeper — and
    // one module deeper is where a test worth having usually is.
    let mut gated_files: Vec<PathBuf> = Vec::new();
    let mut queue: Vec<PathBuf> = Vec::new();
    for path in &sources {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("reading {}: {e}", relative(path)))?;
        for (name, gated) in file_modules(&text) {
            if gated && let Some(child) = module_file(path, &name) {
                queue.push(child);
            }
        }
    }
    while let Some(path) = queue.pop() {
        if gated_files.contains(&path) {
            continue;
        }
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", relative(&path)))?;
        for (name, _) in file_modules(&text) {
            if let Some(child) = module_file(&path, &name) {
                queue.push(child);
            }
        }
        gated_files.push(path);
    }

    for path in &gated_files {
        let rel = relative(path);
        if ARCH_TEST_ALLOW.iter().any(|(allowed, _)| rel.starts_with(allowed)) {
            continue;
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
        let mut carry = Carry::default();
        for (n, line) in text.lines().enumerate() {
            let code = strip_to_code(line, &mut carry);
            if code.trim().ends_with("test]") && !code.contains("cfg(test)") {
                findings.push(format!(
                    "  {rel}:{}  a test in a module an architecture `cfg` gates",
                    n + 1
                ));
            }
        }
    }

    for path in &sources {
        let rel = relative(path);
        if ARCH_TEST_ALLOW.iter().any(|(allowed, _)| rel.starts_with(allowed)) {
            continue;
        }
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
        findings.extend(arch_gated_tests(&rel, &text));
    }

    if findings.is_empty() {
        println!(
            "lint-arch-tests: ok  ({} file(s) behind an architecture gate, none with a test \
             in it; {} recorded exception(s))",
            gated_files.len(),
            ARCH_TEST_ALLOW.len()
        );
        return Ok(());
    }
    Err(format!(
        "{} test(s) compiled on one architecture and not on the other:\n{}\n\n\
         E1-P11's exit is `green, and no test is skipped on AArch64 without a recorded\n\
         reason`. A test behind an architecture `cfg` is skipped on the other runner and\n\
         says nothing when it is: the job stays green, the count on one runner is smaller,\n\
         and nobody reads a count. That is the same silence `PORTABILITY` removes one level\n\
         up, arriving one level down — a crate can be on both runners while a test inside\n\
         it is on one.\n\n\
         Either move the test to the part of the crate that is not architecture-specific —\n\
         `user/init` is the precedent, and states it: the door is gated, the protocol\n\
         arithmetic is not, and the arithmetic is what its tests are about — or add a\n\
         prefix to `ARCH_TEST_ALLOW` in xtask/src/main.rs with a reason and a *Reversal:*.\n\
         RFC 0045.",
        findings.len(),
        findings.join("\n")
    ))
}

/// What would make [`lint_arch_tests`] green while a test was being skipped.
#[cfg(test)]
mod arch_test_lint_tests {
    use super::{ARCH_TEST_ALLOW, arch_gated_tests, file_modules, lint_arch_tests};

    #[test]
    fn a_gated_module_declaration_is_seen_and_an_ungated_one_is_not() {
        let src = "\
#[cfg(target_arch = \"x86_64\")]
pub mod component;

pub mod protocol;
";
        assert_eq!(
            file_modules(src),
            vec![("component".to_string(), true), ("protocol".to_string(), false)]
        );
    }

    #[test]
    fn a_second_attribute_does_not_drop_the_gate() {
        // `user/store` and `user/virtio-blk` both write the gate this way, and
        // a reader that took only the attribute nearest the item would lose it
        // — which would make the two crates with the most architecture-specific
        // code in them the two this check could not see.
        let src =
            "#[cfg(all(target_arch = \"x86_64\", feature = \"image\"))]\npub mod component;\n";
        assert_eq!(file_modules(src), vec![("component".to_string(), true)]);
    }

    #[test]
    fn a_test_under_a_gated_item_is_found() {
        let src = "\
#[cfg(target_arch = \"x86_64\")]
#[test]
fn only_on_one_machine() {}
";
        assert_eq!(arch_gated_tests("x.rs", src).len(), 1);
    }

    #[test]
    fn a_test_inside_a_gated_block_is_found_and_the_gate_lifts_at_its_brace() {
        // The shape a person reaches for second: gate the module rather than
        // the test. The second `mod` is outside the first and must not inherit
        // its gate — if it did, every ungated test after one gated module would
        // read as a finding, and a check that cries wolf is a check somebody
        // deletes.
        let src = "\
#[cfg(target_arch = \"x86_64\")]
mod only_here {
    #[test]
    fn t() {}
}

mod everywhere {
    #[test]
    fn u() {}
}
";
        let findings = arch_gated_tests("x.rs", src);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("x.rs:4"), "{findings:?}");
    }

    #[test]
    fn a_file_scope_gate_survives_the_line_of_code_after_it() {
        // The input review found this check green over, written out exactly.
        // `#![cfg(target_arch)]` opens no brace, so a reader that treats it as
        // a gate on the *next item* discards it the moment any code follows —
        // a `use`, a `const`, anything — and every test below reads as
        // ungated. Two tests here, one of them inside a `#[cfg(test)] mod`,
        // because both were green before.
        let src = "\
//! A file compiled on one machine.
#![cfg(target_arch = \"x86_64\")]

use core::mem::size_of;

#[test]
fn first() {}

#[cfg(test)]
mod inner {
    #[test]
    fn second() {}
}
";
        let findings = arch_gated_tests("x.rs", src);
        assert_eq!(findings.len(), 2, "{findings:?}");
        assert!(findings[0].contains("x.rs:7"), "{findings:?}");
        assert!(findings[1].contains("x.rs:12"), "{findings:?}");
    }

    #[test]
    fn a_file_scope_gate_reaches_the_modules_the_file_declares() {
        // The other half of the same hole: `lint_arch_tests` walks the subtree
        // under a gated `mod`, and a file gate has to reach those declarations
        // or the subtree is walked as if it were on both machines.
        let src = "#![cfg(target_arch = \"x86_64\")]\n\nuse core::mem::size_of;\n\nmod sub;\n";
        assert_eq!(file_modules(src), vec![("sub".to_string(), true)]);
    }

    #[test]
    fn an_inner_attribute_that_is_not_a_gate_is_not_a_finding() {
        // The control for the two above: `#![no_std]` is an inner attribute in
        // almost every file in this workspace. If a bare inner attribute were
        // read as a gate, the check would report the entire tree and be
        // deleted within the week.
        let src = "#![no_std]\n#![forbid(unsafe_code)]\n\n#[test]\nfn t() {}\n";
        assert!(arch_gated_tests("x.rs", src).is_empty(), "{:?}", arch_gated_tests("x.rs", src));
        let with_mod = "#![no_std]\n\nmod sub;\n";
        assert_eq!(file_modules(with_mod), vec![("sub".to_string(), false)]);
    }

    #[test]
    fn an_ordinary_test_module_is_not_a_finding() {
        // The green case, so that the red ones above are known to be red for
        // their own reason rather than because the scanner flags everything.
        let src = "\
#[cfg(test)]
mod tests {
    #[test]
    fn t() {}
}
";
        assert!(arch_gated_tests("x.rs", src).is_empty(), "{:?}", arch_gated_tests("x.rs", src));
    }

    #[test]
    fn the_tree_as_it_stands_has_no_architecture_gated_test() {
        // The property itself, over the real workspace, and a test rather than
        // only a lint so that `cargo xtask test-host` asserts it on the arm
        // runner too — on the machine whose tests are the ones at stake.
        lint_arch_tests().expect("a test in this tree is compiled on one architecture only");
    }

    #[test]
    fn the_gap_the_local_loop_cannot_close_is_declared_and_still_open() {
        // Both directions, the way `JOIN_GAP` is checked. An empty
        // `ARCH_RUN_GAP` would say nothing is unobserved here, which is false
        // and is the shape of a gap quietly deleted rather than closed; and a
        // run path on this machine would mean the gap has closed while the
        // declaration went on describing it, which is the failure mode the
        // declaration exists to be caught by.
        assert!(
            !super::ARCH_RUN_GAP.is_empty(),
            "the local loop cannot observe AArch64 behaviour, so it owes a list of what it \
             therefore does not know"
        );
        if std::env::consts::ARCH != "aarch64" {
            assert_eq!(
                super::aarch64_run_path(),
                None,
                "a hosted AArch64 binary can be run here after all, so ARCH_RUN_GAP is stale"
            );
        }
    }

    #[test]
    fn every_recorded_exception_states_what_would_reverse_it() {
        // An exclusion is a small RFC and owes the same last section, which is
        // the rule `PORTABILITY` is already held to one level up.
        for (path, reason) in ARCH_TEST_ALLOW {
            assert!(
                reason.contains("Reversal:"),
                "{path} skips a test on one architecture without saying what would reverse it"
            );
        }
    }
}

/// What the local loop cannot observe about AArch64, as a set rather than a
/// sentence.
///
/// The exit E1-P11 is measured against has two halves. *No test is skipped
/// without a recorded reason* is decided by reading source, so it is decided
/// here, on this machine, by `lint-arch-tests`. *Green* is decided by running
/// the suite on an AArch64 machine, and this machine is not one — so the second
/// half is unobservable locally, and the honest thing is to name exactly what is
/// unobserved rather than to say "CI covers it" and move on.
///
/// Each entry is one property the arm runner establishes and nothing here can.
/// The list is short on purpose: a long one would mean the local loop had
/// stopped being worth running.
const ARCH_RUN_GAP: &[&str] = &[
    "the ring's Release/Acquire pair holding under a weak memory model — total store \
     order reorders store-then-load and nothing else, and AArch64 reorders freely, so a \
     litmus test passing here is evidence about the ordering's *shape*, not the machine",
    "every host test's behaviour on a target that is not total-store-order — the \
     whole suite runs there, and `test-host` here says nothing about what it does",
];

/// Whether this machine can execute a hosted AArch64 binary after all.
///
/// # Why this is a check and not a comment
///
/// [`ARCH_RUN_GAP`] is a declaration that something cannot be observed here, and
/// a declaration of that shape rots in one direction only: the day the container
/// gains a way to run AArch64 code, the sentence saying it cannot is still in the
/// file, still read as true, and the local loop goes on not running a suite it
/// could now run. That is `JOIN_GAP`'s discipline exactly — the failure worth
/// guarding is not that a gap is never closed but that it closes and the
/// documents go on describing it — so the gap is required to still be a gap.
///
/// Two run paths are probed, because they are the two that exist: an interpreter
/// registered in `binfmt_misc`, which makes an AArch64 binary directly
/// executable, and a `qemu-aarch64` user-mode emulator on `PATH`, which makes it
/// executable when named. A *system* emulator is not probed and is not a run
/// path: `qemu-system-aarch64` is in this image and needs a frame to boot, and
/// the frame is x86-64 — the row `f-kernel` already holds in [`PORTABILITY`].
/// Nor is an installed `aarch64-unknown-linux-gnu` target, which would let a
/// binary be built and still not run.
fn aarch64_run_path() -> Option<String> {
    for name in ["qemu-aarch64", "qemu-aarch64-static"] {
        let registered = Path::new("/proc/sys/fs/binfmt_misc").join(name);
        if registered.is_file() {
            return Some(format!("{} is registered in binfmt_misc", registered.display()));
        }
        // `PATH` is split rather than walked: a directory listing would put a
        // read_dir order into a decision, and RFC 0004 is about exactly that.
        for dir in std::env::var("PATH").unwrap_or_default().split(':') {
            if dir.is_empty() {
                continue;
            }
            let candidate = Path::new(dir).join(name);
            if candidate.is_file() {
                return Some(format!("{} is on PATH", candidate.display()));
            }
        }
    }
    None
}

fn test() -> Result<(), String> {
    // Before anything is compiled, because this one is about what the whole
    // verb is allowed to claim afterwards and it costs two `stat` calls. The
    // unit test in `arch_test_lint_tests` asserts the same thing and would
    // catch it too — from inside `test_host`, several minutes later and worded
    // as an assertion rather than as what to do about it.
    if std::env::consts::ARCH != "aarch64"
        && let Some(found) = aarch64_run_path()
    {
        return Err(format!(
            "ARCH_RUN_GAP says this machine cannot run a hosted AArch64 binary, and it can:\n  \
             {found}\n\n\
             The declaration has outlived the thing it declared. The {} propert(ies) it lists \
             as unobservable here are observable now, so the local loop should run the suite \
             for that architecture rather than describe why it cannot: build the host tests \
             for AArch64 and run them through that path, and shorten `ARCH_RUN_GAP` to \
             whatever is left. E1-P11, RFC 0045.",
            ARCH_RUN_GAP.len()
        ));
    }

    // Host tests exercise the ring and the substrate under the host memory
    // model. That is necessary and not sufficient — see the note below.
    test_host()?;
    cross_check()?;

    // Read at run time rather than through a `cfg`, so that one binary says the
    // true thing on both runners. On the arm job this whole section is about a
    // gap that machine does not have, and printing the x86-64 note there would
    // be the same species of stale sentence `ARCH_RUN_GAP` is checked against.
    if std::env::consts::ARCH == "aarch64" {
        println!(
            "\nnote: this machine is AArch64, so the {} propert(ies) ARCH_RUN_GAP declares\n      \
             unobservable on an x86-64 host were observed by the run above. That is what\n      \
             this job is for. E1-P11, RFC 0045.",
            ARCH_RUN_GAP.len()
        );
        return Ok(());
    }

    println!(
        "\nnote: x86-64 total-store-order hides weak-memory ordering bugs.\n      \
         The AArch64 crates compile here; whether the ring's ordering holds on\n      \
         one is the arm job's to say, and nothing local substitutes for it.\n      \
         This container is x86-64. It has `qemu-system-aarch64`, which needs a\n      \
         frame to boot and the frame is x86-64; what it has no way to run is a\n      \
         *hosted* AArch64 binary — no qemu-user, nothing in binfmt_misc, and no\n      \
         aarch64-unknown-linux-gnu installed to build one for. So it compiles\n      \
         for that architecture and cannot run for it, and `cargo xtask\n      \
         test-host` on the arm runner is what runs. E1-P11, RFC 0045."
    );
    println!(
        "\n{} propert{} the arm runner establishes and this machine cannot \
         (ARCH_RUN_GAP, checked above rather than asserted):",
        ARCH_RUN_GAP.len(),
        if ARCH_RUN_GAP.len() == 1 { "y" } else { "ies" }
    );
    for gap in ARCH_RUN_GAP {
        println!("  - {gap}");
    }
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
    // The half of the allocator the fixture is too small to reach. A second
    // boot rather than a bigger one, and `orders` says why.
    orders()?;
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
    trace_check()?;
    // The same claim, one layer up, and a separate command because it is a
    // separate claim: `trace` says the frame's boot reproduces and this says a
    // simulated workload does. RFC 0032 argues why the seam between them is
    // where it is, and why a tree that answered only one of the two questions
    // would not be able to say which half a failure belonged to.
    sim_check()?;
    // And the seam between the two, which is the only check that says the boot
    // and the workload are about one component set. It boots once more, which is
    // the cost of the claim being about the kernel's own behaviour rather than
    // about a directory listing.
    sim_join()?;
    // And gate G1's own sentence, which is here rather than in CI alone because
    // `claims/0005` says `status = "gating"` and a gating claim that nothing in
    // the local loop runs is a claim that gates nothing. It costs a few seconds:
    // every metric it produces is a count, so there is no machine to wait for
    // and no reason to defer it — which is the whole argument for splitting the
    // latency half into `claims/0006` rather than making both wait.
    chaos()?;
    // Gate G1's other sentence, and the half of it that says a sweep can fail.
    // `sim_check` above proves that a scenario reproduces; this proves that a
    // simulator with a defect in it is *found*, minimised and reported as a
    // command — which is a different claim, and the one E1-P03's exit is about.
    // Eleven seconds, because it sweeps sixteen seeds rather than the default
    // sixty-four: the number here is chosen to reach the defect and no further,
    // and the overnight grid is `.github/workflows/nightly.yml`. RFC 0040.
    sweep_mutate()?;
    // E1-P08's exit, as a command, and in the loop for the reason `chaos` is:
    // `claims/0007` records a ratio and a claim whose reproduction nothing local
    // runs is a claim that reproduces on somebody else's machine only. It is
    // twelve seconds warm, most of which is two release builds of `f-sim` — one
    // with the deliberate defect that gives the run something to fail at, one
    // without, because the pair is also what shows a snapshot from another build
    // being refused. RFC 0043.
    snapshot()?;
    // E1-P04, and in the loop for `chaos`'s reason: `claims/0008` is `gating`
    // and a gating claim that nothing local runs is a claim that gates nothing.
    // A hundred million operations, 4.4-7.3 s in release here over four runs —
    // the exit's billion is 44-60 s and runs in CI, and the nightly runs it
    // again at a moving base. Both counts are thresholds in the claim.
    // Every number it produces is a count, so there is no machine to wait for.
    // The Miri half is not here: it costs six orders of magnitude and has its
    // own job. RFC 0046.
    hostile_gate()?;
    // And the half that says a clean fuzzer means something. Two defects, one
    // per property this half can see, each required to be found by the property
    // it breaks — and the third required to be *invisible* here, which is the
    // argument for the Miri job existing at all.
    hostile_mutate()?;
    // E1-P05, and in the loop for `hostile_gate`'s reason: `claims/0009` is
    // `gating` and a gating claim that nothing local runs is a claim that gates
    // nothing. A quarter of a million cases, and the number it produces is a
    // *percentage of lines*, which is the same figure on a fast host and a slow
    // one. The coverage measurement itself is not here — it needs an
    // instrumented build with link-time optimisation off, which is a second
    // compile of the crate — and has its own step in CI. RFC 0048.
    entries_gate()?;
    // And the half that says a clean entry fuzzer means something: three
    // deliberate defects, one per oracle, each required to be found by the
    // oracle it breaks and by no other.
    entries_mutate()?;
    // E1-B07, and in the loop for `chaos`'s reason: `claims/0010` is `gating`
    // and a gating claim that nothing local runs is a claim that gates nothing.
    // It is a model with a virtual clock and one more boot, and every number it
    // produces is a count — periods met, slots taken, placements refused — so
    // there is no machine to wait for. The half that needs one is `claims/0011`,
    // which is `pending` and is not run here. RFC 0050.
    admission_gate()?;
    // E1-B06, and in the loop for the same reason every line above it is:
    // `claims/0012` is `gating` and a gating claim that nothing local runs is a
    // claim that gates nothing. It was left out once, on the argument that
    // `blk`, `iommu`, `cap`, `user` and `fault` are device-requiring boots kept
    // out of this loop — but those five feed no gating claim, and the
    // difference is the whole of why this one is here. Three boots, and it is
    // the only member of this loop that needs a disk image and a remapping
    // unit: a host with neither fails here rather than reporting an ordering it
    // never ran, which is R04 pointing the safe way. Every number it produces
    // is a count, so there is no machine to wait for; the half that needs one is
    // `claims/0013`, which is `pending` and is not run here. RFC 0049.
    deadline(None)?;
    // E1-B14, and in the loop for the reason every line above it is:
    // `claims/0014` is `gating` and a gating claim that nothing local runs is a
    // claim that gates nothing. One boot and one host workload, and every number
    // either produces is a count — invalidations per unmap request, round trips
    // saved per set, shootdowns issued — so there is no machine to wait for. It
    // needs a remapping unit and no disk, which puts it between `deadline` and
    // the rest. The half that needs a machine is `claims/0015`, which is
    // `pending` and whose *time* is not run here: the workload runs, and
    // `f_bench::Environment` declines to publish what it saw. RFC 0052.
    churn()?;
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
    println!(
        "         One gating claim's own metric is not in this: `claims/0009`'s\n         \
         path_line_coverage needs a second, instrumented compile with link-time\n         \
         optimisation off, so it is `cargo xtask entries --coverage` and the CI\n         \
         `entries` job rather than part of the local loop. Green here means the\n         \
         262 144-case gate and the three oracles passed, not that the percentage\n         \
         was measured."
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
    // The topology check RFC 0005 promised in the R02 row: every component
    // manifest fits the schema, declares a domain, and does not put an
    // imported image in `shared`. It runs here so a boot is not the first
    // place a missing field is found.
    lint_manifests()?;
    // And the list that decides which of those manifests is built. It runs
    // beside the schema check because the two answer halves of one question —
    // *is this a component* and *does anything build it* — and the second was
    // for a while answered only by a hand-written list nothing compared against
    // anything. RFC 0041.
    lint_components()?;
    // The mechanism behind `blk/copies`. The counter is published at zero on
    // every datapath boot and would be published at zero by a crate that had
    // grown a second way to move a client's bytes, so the property is checked
    // where it actually lives — in the source — rather than inferred from a
    // number that cannot move. E1-B02.
    lint_datapath()?;
    // And the reversal conditions that have fallen due and are not paid,
    // declared as a set for `CHAOS_GAP`'s reason: a deviation in prose is one
    // nobody re-checks, and the failure that matters is not that it is never
    // closed but that it is closed and the documents go on describing it.
    lint_owed()?;
    // One level below `PORTABILITY`, and the level that table cannot see: a crate
    // can be on both runners while a test inside it compiles on one. `test-host`
    // would stay green through that, because a smaller test count is not a failure
    // and nobody reads a count. E1-P11, RFC 0045.
    lint_arch_tests()?;
    // A generated file that is committed is a claim about the generator, and
    // the only moment it can be checked cheaply is before anything regenerates
    // it. `xtask claims` rewrites the snapshot by design, so this has to come
    // first or it grades its own homework.
    lint_snapshot()?;
    lint_reproduce()?;
    // The build RFC 0053 promised an ordinary `cargo build` could find a
    // broken stand-in with, and which nothing was running. `kernel/proofs` is
    // outside the workspace, so the two invocations below reach it and neither
    // does the `fmt --all` above. Under fifteen seconds, and no checker.
    lint_proofs()?;
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

/// Every component manifest in the permissive tree fits `docs/manifest.md`.
///
/// The schema itself is `manifest::check`; this is the walk, the cross-file
/// rule, and the report. The cross-file rule is that two manifests do not share
/// a `name`: the topology and every `sibling:` reference name a component by
/// it, and two files with one name are two answers to "which component".
///
/// An image that is not in the tree yet is reported on the ok line rather than
/// refused. The manifest is written before the driver on purpose — E1-D04
/// precedes E1-B02 as a claim precedes its number — and the moment existence
/// matters is assembly, which refuses there. Reporting it every run is what
/// keeps "not yet" from quietly becoming "never".
fn lint_manifests() -> Result<(), String> {
    let files = manifest::files(&root(), &target_dir())?;
    let mut findings = Vec::new();
    let mut pending = Vec::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();

    for path in &files {
        let rel = relative(path);
        let text = std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
        let checked = match manifest::check(&rel, &text) {
            Ok(checked) => checked,
            Err(mut refusals) => {
                findings.append(&mut refusals);
                continue;
            }
        };
        if let Some(other) = names.insert(checked.name.clone(), rel.clone()) {
            findings.push(format!(
                "  {rel}  `name = \"{}\"` is also the name in {other}; a component has one manifest",
                checked.name
            ));
        }
        match manifest::image_state(&root(), &checked) {
            manifest::Image::Present | manifest::Image::ByHash => {}
            manifest::Image::NotYet => pending.push(format!("{} ({rel})", checked.image)),
            manifest::Image::Wrong(why) => findings.push(format!("  {rel}  {why}")),
        }
    }

    if findings.is_empty() {
        let not_yet = if pending.is_empty() {
            String::new()
        } else {
            format!("; not yet built: {}", pending.join(", "))
        };
        println!("lint-manifests: ok  ({} manifest(s) fit the schema{not_yet})", files.len());
        return Ok(());
    }
    Err(format!(
        "{} manifest problem(s):
{}

         A manifest is a parser's input and a parser here refuses what it does not
         know — R04. The schema is docs/manifest.md, field by field, with the reason
         for each field and what is refused; the domain field is RFC 0005's, the
         shape is RFC 0008's. A component the lint refuses is one the supervisor
         would refuse to spawn, found here instead of at boot.",
        findings.len(),
        findings.join(
            "
"
        )
    ))
}

/// The name a line declares, if the line declares a function.
///
/// Prefixes rather than a parser, and the list is what this workspace actually
/// writes: `pub`, `pub(crate)`, `const`, `unsafe`, in the order rustfmt puts
/// them. A form nobody here writes reports no name, which fails *closed* — the
/// enclosing function stays whatever it was, and a call in an unrecognised
/// function is attributed to the previous one and therefore refused.
fn declared_fn(code: &str) -> Option<&str> {
    let mut rest = code.trim_start();
    for prefix in ["pub(crate) ", "pub(super) ", "pub ", "default ", "const ", "async ", "unsafe "]
    {
        if let Some(stripped) = rest.strip_prefix(prefix) {
            rest = stripped;
        }
    }
    let rest = rest.strip_prefix("fn ")?;
    let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))?;
    rest.get(..end)
}

/// One crate's shipped source, against one row of [`DATAPATH`].
///
/// `text` is a whole file. The scan stops at the first `#[cfg(test)]`, which is
/// the seam between what ships and what the host tests build for themselves;
/// [`MINTS`] says why that seam has to exist.
fn datapath_findings(
    rel: &str,
    text: &str,
    mover: &str,
    allowed: &str,
    mints: &[&str],
) -> (Vec<String>, usize) {
    let mut findings = Vec::new();
    let mut calls = 0;
    let mut carry = Carry::default();
    let mut current = "";
    let call = format!("{mover}(");

    for (n, raw) in text.lines().enumerate() {
        let code = strip_to_code(raw, &mut carry);
        let trimmed = code.trim();
        if trimmed.starts_with("#[cfg(test)]") {
            break;
        }
        if declared_fn(&code).is_some() {
            // Taken from the raw line rather than from the stripped copy, whose
            // lifetime ends here. A declaration line has nothing stripped out of
            // it; if the two ever disagreed, `current` keeps its previous value
            // and the next call is attributed to the wrong function and refused,
            // which is the direction to be wrong in. The definition of the mover
            // is not a call to it, so the line ends here either way.
            if let Some(name) = declared_fn(raw) {
                current = name;
            }
            continue;
        }
        for mint in mints {
            if code.contains(mint) {
                findings.push(format!(
                    "  {rel}:{}  {mint}) — a component receives a granted window and does \
                     not mint one",
                    n + 1
                ));
            }
        }
        if code.contains(&call) {
            calls += 1;
            if current != allowed {
                findings.push(format!(
                    "  {rel}:{}  `{mover}` called from `{current}`, not from `{allowed}`",
                    n + 1
                ));
            }
        }
    }
    (findings, calls)
}

/// The zero on the data path, checked as a property of the source.
///
/// # What this makes true that a counter alone does not
///
/// `blk/copies` is published at zero on every boot, and it would be published
/// at zero by a crate that had grown a second way to move a client's bytes — so
/// on its own it is an assertion with a `u64` around it. This is the mechanism
/// behind it: for each row of [`DATAPATH`], the crate defines exactly one
/// function that moves bytes, calls it from exactly one place, and that place is
/// the boot's own self-check rather than the data path. A build where the mover
/// had been deleted fails here, and so does one where the data path had started
/// calling it — which are the two ways the published zero could stop meaning
/// what it says.
///
/// It is a source check and it is limited the way every source check is: it
/// reads names, so a mover spelled differently is a mover it does not know
/// about. That limit is why [`MINTS`] used to be in it — the one *general* way
/// a safe component could reach memory it was not handed is to build an
/// accessor over an address it invented, which is a shape rather than a name —
/// and why it no longer needs to be: the component runs at ring 3 and an
/// address it invents is a page fault.
///
/// The second half is [`NOT_THE_FRAME`], and it points the other way. The claim
/// above is worth nothing if the frame is the one running the crate's code, so
/// this refuses a frame that names an associated item of a component's driver
/// type — RFC 0033's own reversal condition, run as a check rather than left as
/// an instruction to a reader.
fn lint_datapath() -> Result<(), String> {
    let sources = rust_sources()?;
    let mut findings = Vec::new();

    for (prefix, mover, allowed) in DATAPATH {
        let mut defined = 0;
        let mut calls = 0;
        let mut seen = false;

        for path in &sources {
            let rel = relative(path);
            if !rel.starts_with(prefix) {
                continue;
            }
            seen = true;
            let text = std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
            let mut carry = Carry::default();
            for line in text.lines() {
                let code = strip_to_code(line, &mut carry);
                if code.trim().starts_with("#[cfg(test)]") {
                    break;
                }
                if declared_fn(&code) == Some(*mover) {
                    defined += 1;
                }
            }
            let (found, called) = datapath_findings(&rel, &text, mover, allowed, MINTS);
            findings.extend(found);
            calls += called;
        }

        if !seen {
            findings.push(format!("  {prefix}  no source under this prefix — the row is stale"));
            continue;
        }
        // Both directions, and the second is the one that matters: a crate with
        // no mover in it publishes the same zero as a crate whose mover the data
        // path never calls, and only one of those is the property holding.
        if defined != 1 {
            findings.push(format!(
                "  {prefix}  `{mover}` is defined {defined} time(s); the claim is that it is \
                 defined once and is the only thing that moves bytes"
            ));
        }
        if calls != 1 {
            findings.push(format!(
                "  {prefix}  `{mover}` is called {calls} time(s); it must be called exactly \
                 once, from `{allowed}`, so that the counter it moves has been moved"
            ));
        }
    }

    // And the half that says who is running the code above. A crate that moves
    // bytes in one place is a claim about a component; it is worth nothing if
    // the frame is the component's caller, because then the direct map is under
    // every address in it. RFC 0033, RFC 0047.
    //
    // Both directions, for `NOT_THE_FRAME`'s own reason: the absence half is a
    // search for a name, and a name nothing defines is absent from everywhere.
    for (prefix, needle, defines) in NOT_THE_FRAME {
        let mut named = 0usize;
        for path in &sources {
            let rel = relative(path);
            if rel.starts_with(prefix) {
                let text =
                    std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
                findings.extend(frame_findings(&rel, &text, needle));
            }
            if rel.starts_with(defines) {
                let text =
                    std::fs::read_to_string(path).map_err(|e| format!("reading {rel}: {e}"))?;
                named += code_mentions(&text, needle);
            }
        }
        if named == 0 {
            findings.push(format!(
                "  {defines}  nothing under this prefix names `{needle}` in shipped code, so \
                 the rule that `{prefix}` must not name it cannot fail — rename the \
                 type and the check goes green over a frame calling a component"
            ));
        }
    }

    if findings.is_empty() {
        println!(
            "lint-datapath: ok  ({} crate(s) move bytes in one place, not on the data \
             path, and none of them called by the frame)",
            DATAPATH.len()
        );
        return Ok(());
    }
    Err(format!(
        "{} datapath finding(s):\n{}\n\n\
         E1-B02's exit is `zero copies on the data path, verified by counter`, and the\n\
         counter is structurally zero: a request resolves to a `Reach`, which is an\n\
         address and a length and not a slice, so the address goes into a descriptor and\n\
         the bytes never reach the component. This is what keeps that true. A zero\n\
         published by a crate with a second way to move bytes reads exactly like a zero\n\
         published by one without — which is the reason `state::node::MEMORY_FORCED`\n\
         exists beside `MEMORY_REMOTE`, one subsystem over.\n\n\
         If a driver now legitimately needs to move bytes on a client's behalf, that is a\n\
         change to what E1-B02 claims and belongs in an RFC and in `DATAPATH`, not in a\n\
         second call site.",
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
        // The checker's harness is not the checked. `kernel/proofs` is under
        // `kernel/` — so `lint-unsafe` permits its one `unsafe impl`, which is
        // right — but it is not part of the trusted computing base this number
        // measures: it never ships, it is not linked into anything, and it is
        // not even built by this workspace. Counting it would move A-05 in
        // *both* directions for a reason that has nothing to do with the
        // frame, and the direction that matters is the flattering one: a
        // couple of hundred lines of proof harness would dilute the share and
        // make the metric improve because somebody wrote a proof.
        //
        // *Reversal:* the day something under `kernel/proofs` is linked into an
        // image. Then it is the frame and belongs in the denominator.
        //
        // `ring/proofs` is the same argument one crate over and it matters
        // more there, because that crate's fixture writes symbolic bytes into
        // a region and is therefore several `unsafe` blocks of harness. RFC
        // 0057.
        if rel.starts_with(PROOFS) || rel.starts_with(RING_PROOFS) {
            continue;
        }
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

/// One scalar from inside one named table.
///
/// [`toml_field`] answers with the first matching key *anywhere* in the file,
/// which is right for `status` and `milestone` and wrong the moment two tables
/// share a key name. `command` lives in `[reproduce]` and `path` lives in
/// `[workload]` and also in `[owner]` under the name `document`; asking for a
/// key without saying which table is asking for whichever table happens to come
/// first, which is a correct answer by accident.
fn toml_table_field(text: &str, table: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            inside = trimmed.trim_end().trim_end_matches('\r') == format!("[{table}]");
            continue;
        }
        if !inside {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix(key) else { continue };
        if rest.trim_start().starts_with('=') {
            return toml_scalar(trimmed);
        }
    }
    None
}

/// One line of the release manifest.
struct Content {
    /// What the release contract calls it.
    name: &'static str,
    /// Where it comes from, when it exists.
    source: ContentSource,
    /// The task that produces it, when it does not exist yet.
    ///
    /// `None` for every row today, and that is the state RFC 0021's *what would
    /// reverse this* named as its own end: the field stays because a future
    /// deferral will need it and a row that owes nothing should say so, and the
    /// **conditional** requirement that used to sit beside it is gone.
    owed_to: Option<&'static str>,
}

// The `Requirement` enum used to live here, and it is gone rather than left
// empty. It let a content be *not owed while a claim is `pending`*, which was
// `E0-R01`'s honest answer to a contract listing two things that could not exist
// yet — `A-07` is the finding that produced it and RFC 0021 is the argument.
//
// RFC 0021's *what would reverse this* named the condition precisely: the
// variant should be deleted rather than left as a shape for the next deferral to
// grow into, and it named `E1-P01` and `E1-P03` as the owners. `E1-D06` closed
// the baseline row; `E1-P03` closes the seed corpus, which was the last content
// carrying the conditional. So the eight contents are unconditional again, which
// is what `RELEASING.md` says without a footnote.
//
// If a ninth content is ever deferred, this comment is the record of how it was
// done last time — and of the rule that a deferral is a variant somebody deletes
// rather than a shape that stays.

/// Where a manifest entry's content comes from.
enum ContentSource {
    /// A file in the tree.
    File(&'static str),
    /// Several named files, which are one content between them.
    ///
    /// Not a directory and not an extension filter, because the files a content
    /// is made of do not have to live together: the seed corpora are one per
    /// fuzzer and each sits beside the fuzzer it belongs to — `sim/corpus.txt`
    /// beside the simulator, `ring/corpus.txt` and `ring/entries-corpus.txt`
    /// beside the two that drive the ring. Moving them into one directory to
    /// suit the packager would put each of them further from the thing that
    /// writes it, which is the wrong trade: the contract cares that a release
    /// carries every corpus, not where they are.
    ///
    /// Every named file must exist. A content that is present by the manifest's
    /// count and short of a corpus is the same failure as an absent row, arriving
    /// through a door the fix opened — which `ContentSource::Dir` already had to
    /// say once, one content up.
    Files(&'static [&'static str]),
    /// Every file directly under a directory with this extension.
    Tree(&'static str, &'static str),
    /// Every file under a directory and everything below it, whatever the
    /// extension.
    ///
    /// [`Tree`](ContentSource::Tree) filters by extension and looks one level
    /// deep, which is right for `docs/rfc` — one flat directory of `.md`.
    ///
    /// It is wrong for `claims/baselines`, which is a directory *per baseline
    /// version*, each holding a README, three scripts and three data files
    /// with four extensions between them and none in common. A filter
    /// naming one of those would package a baseline without the script that
    /// applies it — a content that is present by the manifest's count and
    /// useless to the stranger the contract is written for, which is a worse
    /// failure than the absent row it replaced.
    Dir(&'static str),
    /// Produced by the build rather than read from the tree.
    Built(&'static str),
    /// Nothing produces it yet.
    ///
    /// **No content is in this state today**, which is why the compiler is told
    /// to expect it to be unused rather than told to ignore it. `E1-P03` landed
    /// the seed corpus, the last row that was ever `Absent`, and the `[!!]` arm
    /// in `release` and the refusal in `build_package` are the two places that
    /// read this variant.
    ///
    /// Kept rather than deleted, unlike the `Requirement` enum this file used to
    /// carry beside it, and the difference is worth stating because the two look
    /// alike. `Requirement` encoded *a content may be missing*, which is a
    /// policy, and RFC 0021 asked for it to go so that the next deferral had to
    /// argue for itself rather than fill in a shape. This encodes *a content
    /// does not exist*, which is a fact a manifest has to be able to state — and
    /// the day one does not, the row says so and the packager refuses.
    ///
    /// The expectation is self-correcting in both directions: an unused variant
    /// is a warning, and an expectation that stops being met is also a warning,
    /// so adding an `Absent` row deletes this line rather than being silently
    /// tolerated by it.
    #[expect(
        dead_code,
        reason = "the shape a content with no producer takes; every row has one today"
    )]
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
    // This row was `Absent`, owed to `E1-D06`, and exempt while
    // `ring-submit-latency` stayed `pending` — because `claims/0001` named
    // `linux-6.x-tuned` in one sentence of prose, and prose ages into a stock
    // comparison without anybody deciding it should, since prose cannot be
    // re-run. `E1-D06` delivered the directory it names: `cmdline.txt`,
    // `sysctl.conf` and `baseline.conf` as data, `apply.sh` to put a machine
    // into that configuration and `verify.sh` to say when it has drifted out
    // of one, and a README carrying the concessions and the rule that
    // re-tuning produces a new versioned directory rather than an edit.
    //
    // So the exemption is gone rather than satisfied, and the row is `Always`:
    // the deferral was never about this content being optional, it was about
    // the content not existing. A directory rather than a file because there
    // will be more than one baseline — that is the append-only rule the README
    // states — and a new version of it appears in the package by existing.
    Content {
        name: "the baseline configuration",
        source: ContentSource::Dir("claims/baselines"),
        owed_to: None,
    },
    // This row was `Absent`, owed to `E1-P01` and `E1-P03`, and exempt while
    // `ring-submit-latency` stayed `pending` — because the corpus exists so a
    // third party can re-run the sweeps a number came out of, and there was no
    // sweep and no number.
    //
    // `E1-P03` built the sweep, so the exemption is gone rather than satisfied.
    // One file carries both halves the contract names: the entries are the seed
    // corpus — every trial that has ever found something, replayed by
    // `cargo xtask sweep --corpus` and required to be clean — and the header is
    // the scenario set, regenerated from `SCENARIOS` on every write so that a
    // list of scenarios in a comment cannot stop matching the table.
    //
    // What the row does **not** claim is that these seeds were found in the
    // wild. Every entry today was found under `mutate-crossed-completion`, the
    // deliberate defect, and each says so on its own `# under` line — because a
    // corpus whose entries are all green and do not say why they are green would
    // read as a corpus that never found anything. RFC 0040.
    // And this row grew a second and a third file rather than a second row,
    // which is the decision E1-P05 had to take and is worth stating. The
    // contract names *the seed corpus* — one content — and there are three
    // fuzzers in this tree that accumulate one: the simulator's sweeps, the
    // hostile peer, and the structure-aware entry generator. A row each would
    // have made `release --dry-run` report ten of ten and the contract's list of
    // eight stop matching `docs/the-long-plan.html` section 08; a row that named
    // only the first would have shipped a release missing two of the three
    // corpora it has. So the row is one content made of three files, and the
    // count stays eight.
    //
    // What each is, because they are not the same kind of thing.
    // `sim/corpus.txt` and `ring/corpus.txt` are **regression suites**: every
    // entry found something once and says what, at which commit, under which
    // deliberate defect. `ring/entries-corpus.txt` is a **cover**: every entry
    // reaches a region of the entry-validation path no earlier entry reaches,
    // and it is the artefact `claims/0009`'s number is measured from — which is
    // why that claim can say a stranger reproduces the figure without a seed and
    // without a fuzzing run. RFC 0040, RFC 0046 and RFC 0048 respectively.
    Content {
        name: "the seed corpus and scenario set",
        source: ContentSource::Files(&[
            "sim/corpus.txt",
            "ring/corpus.txt",
            "ring/entries-corpus.txt",
        ]),
        owed_to: None,
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

/// Every file one content contributes, as `(name in the archive, bytes)`.
///
/// Sorted by name at the end, and that sort is load-bearing rather than tidy:
/// `read_dir` order is the filesystem's business and differs between two
/// machines holding the same files. It is the one same-machine-invisible
/// difference a build-it-twice check cannot see, which is why it is also the
/// one this file tests directly.
fn content_files(content: &Content) -> Result<Vec<(String, Vec<u8>)>, String> {
    let read = |rel: &str| -> Result<Vec<u8>, String> {
        std::fs::read(root().join(rel)).map_err(|e| format!("reading {rel}: {e}"))
    };

    let mut files = match &content.source {
        ContentSource::File(path) => vec![((*path).to_string(), read(path)?)],
        ContentSource::Files(paths) => {
            let mut out = Vec::new();
            for path in *paths {
                out.push(((*path).to_string(), read(path)?));
            }
            out
        }
        ContentSource::Tree(dir, extension) => {
            let mut out = Vec::new();
            let entries =
                std::fs::read_dir(root().join(dir)).map_err(|e| format!("reading {dir}/: {e}"))?;
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if !path.extension().is_some_and(|x| x == *extension) {
                    continue;
                }
                let rel = relative(&path);
                let bytes = std::fs::read(&path).map_err(|e| format!("reading {rel}: {e}"))?;
                out.push((rel, bytes));
            }
            out
        }
        ContentSource::Dir(dir) => {
            // A stack rather than recursion, and no ordering assumed while
            // walking: the sort below is what makes the result the same on two
            // machines, so the walk is free to visit in whatever order the
            // filesystem hands back.
            let mut out = Vec::new();
            let mut pending = vec![root().join(dir)];
            while let Some(directory) = pending.pop() {
                let entries = std::fs::read_dir(&directory)
                    .map_err(|e| format!("reading {}/: {e}", relative(&directory)))?;
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.is_dir() {
                        pending.push(path);
                        continue;
                    }
                    let rel = relative(&path);
                    let bytes = std::fs::read(&path).map_err(|e| format!("reading {rel}: {e}"))?;
                    out.push((rel, bytes));
                }
            }
            out
        }
        ContentSource::Built(_) => Vec::new(),
        ContentSource::Absent => Vec::new(),
    };
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Build the release package, or print the manifest it would produce.
///
/// # What the package is
///
/// One `.tar`, and the contract's eight contents inside it, plus a `MANIFEST`
/// naming every file and its SHA-256. The archive is built by `pack::Tar`,
/// which has no clock and no user in it; the hashes are `pack::sha256`, which
/// has no dependency. Both of those are the same requirement stated twice: a
/// content address that depends on which machine computed it is not one.
///
/// Not compressed, deliberately. A deflate stream carries its encoder's version
/// and level in the output, so compressing here would put a dependency's
/// *version* inside the content address. Whoever ships the file may compress
/// it; that is an envelope, and the address is of the content.
///
/// # Errors
///
/// A missing content the registry says is required, a tree git cannot
/// identify, or a name the archive format cannot hold.
fn release(mode: Option<&str>) -> Result<(), String> {
    let dry_run = mode == Some("--dry-run");
    let twice = mode == Some("--twice");
    let address_only = mode == Some("--address");
    if let Some(other) = mode
        && !dry_run
        && !twice
        && !address_only
    {
        return Err(format!("unknown option for release: {other}"));
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

    if twice {
        let first = build_package(&describe, &commit)?;
        let second = build_package(&describe, &commit)?;
        if first.0 != second.0 {
            return Err(format!(
                "two builds of the same tree produced two packages:\n  \
                 {}\n  {}\n\n\
                 Something with a clock, a user or a directory order in it has reached\n\
                 the archive. `pack::Tar` has none of the three, so look at what was\n\
                 put into it — the kernel image is the likeliest, because a debug build\n\
                 carries its absolute build path in DWARF and in cargo's -Cmetadata.",
                first.0, second.0
            ));
        }
        println!("release: the same tree packages identically — {}", first.0);
        println!(
            "\n  This is the weaker half of E0-R01's exit and it is worth saying so.\n\
             \x20 Directory order, uid, path and clock are all constant within one\n\
             \x20 machine, so two runs here agree for reasons that say little about two\n\
             \x20 machines agreeing. `--address` on two runners is the other half."
        );
        return Ok(());
    }

    // The address and nothing else, so that two runners can each write one line
    // and a third job can compare them. `trace --hash` is the same shape for the
    // same reason: what crosses a machine boundary is an artefact, and an
    // artefact a job has to parse is one a job can parse wrong.
    //
    // Deliberately no path, no version and no timestamp in this output, though
    // all three would be useful in a failure. The build path is a property of
    // the runner rather than of the release, and a release record naming where
    // it happened invites the reading that a different path is an acceptable
    // difference. It is not: it changes the address, which is the whole finding
    // this verb exists to expose. The workflow records the path beside this
    // line, where it belongs.
    if address_only {
        println!("{}", build_package(&describe, &commit)?.0);
        return Ok(());
    }

    println!("release manifest{}\n", if dry_run { " (dry run)" } else { "" });
    println!("  version   {describe}");
    println!("  commit    {commit}");
    println!("  contract  RELEASING.md");
    let route = packaged_sweep_route(&commit)?;
    println!("  sweep     {route}\n");
    if route != SWEEP_FROM_MANIFEST {
        // Loud rather than a row, because this is the one line here that says a
        // route `RELEASING.md` publishes will not work for the package about to
        // be built. It is checked instead of asserted for a reason that had
        // already bitten once: the fallback lives in `xtask/src/main.rs`, the
        // package reaches a stranger only through `source.tar`, and `git
        // archive` takes that from the commit — so a working tree that carries
        // the fallback and a package that does not are the ordinary state of an
        // uncommitted change, and the document cannot tell them apart.
        println!(
            "  The source this package would carry cannot read a commit out of MANIFEST,\n\
             \x20 so `cargo xtask sweep` from the unpacked package refuses with `cannot read\n\
             \x20 the commit from git`. RELEASING.md's *From the package alone* holds for\n\
             \x20 packages built at or after the commit that lands that fallback; this is\n\
             \x20 not one. Commit it and package again. RFC 0056.\n"
        );
    }

    // The manifest head, printed here because `RELEASING.md` sends a human to
    // `--dry-run` to read the claims block before authorising a tag, and until
    // now this command did not print one — prose drifting from a generated file,
    // which is the failure RFC 0056 was written against, in RFC 0056's own diff.
    // Verbatim rather than re-rendered: this is the text the package carries,
    // and a second rendering is a second thing that can disagree with `claims/`.
    print!("{}", claims_block()?);

    let mut missing = 0usize;
    for content in CONTENTS {
        match &content.source {
            ContentSource::File(path) => {
                let full = root().join(path);
                if full.exists() {
                    let bytes = std::fs::read(&full).map_err(|e| e.to_string())?;
                    let hash = pack::hex(&pack::sha256(&bytes));
                    println!("  [ok]  {:<36} {path}", content.name);
                    println!("        {} bytes  sha256 {}", bytes.len(), &hash[..16]);
                } else {
                    missing += 1;
                    println!("  [--]  {:<36} {path} does not exist", content.name);
                }
            }
            ContentSource::Files(paths) => {
                let absent: Vec<&&str> =
                    paths.iter().filter(|path| !root().join(path).exists()).collect();
                if absent.is_empty() {
                    let mut bytes = 0usize;
                    for path in *paths {
                        bytes += std::fs::read(root().join(path)).map_err(|e| e.to_string())?.len();
                    }
                    println!("  [ok]  {:<36} {} file(s)", content.name, paths.len());
                    for path in *paths {
                        println!("        {path}");
                    }
                    println!("        {bytes} bytes in all");
                } else {
                    missing += 1;
                    println!(
                        "  [--]  {:<36} {} of {} file(s) do not exist: {}",
                        content.name,
                        absent.len(),
                        paths.len(),
                        absent.iter().map(|path| **path).collect::<Vec<_>>().join(", ")
                    );
                }
            }
            ContentSource::Tree(dir, _) | ContentSource::Dir(dir) => {
                let count = content_files(content)?.len();
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
                // `!!` and never `--`: every content the contract names is owed,
                // now that the last conditional row has landed. A row reaching
                // this arm at all is a release that cannot go out. RFC 0021.
                println!("  [!!]  {:<36} nothing produces this yet", content.name);
                if let Some(task) = content.owed_to {
                    println!("        owed to {task}");
                }
            }
        }
    }

    println!("\n  {} of {} contents present", CONTENTS.len() - missing, CONTENTS.len());

    if dry_run {
        // The gates, listed rather than run. Running `verify` from inside a dry
        // run would make a manifest print cost a full boot, and the point of
        // this command is that it is cheap enough to run while thinking.
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
                "\n`cargo xtask release` builds the package; the rows above say what would\n\
                 be in it. `--twice` checks that the same tree packages identically."
            );
        }
        return Ok(());
    }

    let (address, path, count, bytes) = build_package(&describe, &commit)?;
    println!("\n  {count} file(s), {bytes} bytes");
    println!("  {}", relative(&path));
    println!("\nrelease: {address}");
    println!(
        "\n  The address is over the content, not the envelope: no clock, no user,\n\
         \x20 no directory order and no compressor version is in it. Two machines at\n\
         \x20 this commit must produce this string, and `cargo xtask release --twice`\n\
         \x20 is the local half of asking."
    );
    Ok(())
}

/// Assemble the archive and return `(address, path, files, bytes)`.
fn build_package(describe: &str, commit: &str) -> Result<(String, PathBuf, usize, usize), String> {
    // Every file that will be in the package, gathered before anything is
    // written, so a refusal leaves no half-built archive behind.
    let mut files: Vec<(String, bool, Vec<u8>)> = Vec::new();
    let mut manifest = String::new();
    manifest.push_str("# The release package, as a list of what is in it.\n#\n");
    manifest.push_str("# Every line is a file and its SHA-256. The package's own address is\n");
    manifest
        .push_str("# the SHA-256 of the archive these are in. RELEASING.md is the contract.\n\n");
    manifest.push_str(&format!("version {describe}\ncommit  {commit}\n"));
    manifest.push_str(
        "# Whether `cargo xtask sweep` runs inside this package with no repository:\n\
         # the source below either reads the commit line above when git cannot answer,\n\
         # or predates that fallback and refuses. Derived from the packaged source, not\n\
         # asserted. RELEASING.md, *From the package alone*; RFC 0056.\n",
    );
    manifest.push_str(&format!("sweep   {}\n\n", packaged_sweep_route(commit)?));
    manifest.push_str(&claims_block()?);

    for content in CONTENTS {
        let gathered = content_files(content)?;

        if gathered.is_empty() && !matches!(content.source, ContentSource::Built(_)) {
            return Err(format!(
                "the release contract requires `{}` and nothing produces it.\n\n\
                 {}\n\n\
                 A package missing a content the contract names is not a smaller\n\
                 release, it is a release nobody can check. RFC 0021.",
                content.name,
                content.owed_to.unwrap_or("No task owes it, which is worse.")
            ));
        }

        for (name, bytes) in gathered {
            let hash = pack::hex(&pack::sha256(&bytes));
            manifest.push_str(&format!("{hash}  {name}\n"));
            files.push((name, false, bytes));
        }
    }

    // The source, as git sees it at this commit. Not a walk of the working
    // tree: `git archive` takes its file list from the tree object and its
    // mtimes from the commit, so it cannot pick up an untracked file and cannot
    // vary with when the checkout happened.
    let source = capture_bytes("git", &["archive", "--format=tar", commit])?;
    let source_hash = pack::hex(&pack::sha256(&source));
    manifest.push_str(&format!("{source_hash}  source.tar\n"));
    files.push(("source.tar".to_string(), false, source));

    // The kernel image, which is built rather than read. Last, so that a build
    // failure does not happen after an archive has been assembled.
    build()?;
    let image = kernel_elf32();
    let bytes = std::fs::read(&image).map_err(|e| format!("reading {}: {e}", relative(&image)))?;
    let hash = pack::hex(&pack::sha256(&bytes));
    let name = "image/f-kernel.elf32".to_string();
    manifest.push_str(&format!("{hash}  {name}\n"));
    files.push((name, true, bytes));

    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut tar = pack::Tar::new();
    tar.file("MANIFEST", false, manifest.as_bytes())?;
    let count = files.len();
    let mut total = manifest.len();
    for (name, executable, bytes) in files {
        total += bytes.len();
        tar.file(&name, executable, &bytes)?;
    }
    let archive = tar.finish();

    let address = pack::hex(&pack::sha256(&archive));
    // `target/package/` and not `target/release/`: that second one is cargo's
    // release *profile* directory, and putting an artefact of ours in it means
    // one `cargo build --release` away from a collision nobody expected.
    let out = target_dir().join("package");
    std::fs::create_dir_all(&out).map_err(|e| format!("creating {}: {e}", relative(&out)))?;
    let path = out.join(format!("f-{describe}.tar"));
    std::fs::write(&path, &archive).map_err(|e| format!("writing {}: {e}", relative(&path)))?;

    Ok((address, path, count + 1, total))
}

/// What `MANIFEST` says when the packaged source can sweep without a repository.
const SWEEP_FROM_MANIFEST: &str = "from MANIFEST";

/// The same field when it cannot.
const SWEEP_NEEDS_REPOSITORY: &str = "needs a repository";

/// Whether the source this package will carry can identify itself without git.
///
/// # Why this is derived rather than written down
///
/// `RELEASING.md` publishes a route — unpack the package and its `source.tar`
/// into one directory, then sweep — and the fallback that makes the route work
/// lives in this file. A stranger reaches this file only through `source.tar`,
/// which is `git archive` of the commit, so a working tree that has the fallback
/// and a package that does not is not an exotic state: it is every moment
/// between writing the fallback and committing it. That is exactly how the route
/// was published false once, and a sentence in a document could not have caught
/// it because the document is right about the tree and wrong about the artefact.
///
/// So the packager reads the source it is about to ship and states what it
/// found, in the file that is about the package. A stranger holding a package
/// learns from one line whether the route holds for *that* package, instead of
/// from a document describing some other one.
///
/// The test is a search of the shipped bytes for the fallback's name, which is
/// weaker than running it and is chosen for which way it fails. Renaming
/// `manifest_commit` makes this report `needs a repository` for a package that
/// can in fact sweep — a pessimistic manifest and a nuisance — where anything
/// that inferred the route from the packager's own binary would report the route
/// as present because *this* build has it, which is the false direction and the
/// one that already happened. If the rename ever occurs, the honest repair is to
/// unpack a package and sweep it in the release job rather than to loosen this.
fn packaged_sweep_route(commit: &str) -> Result<&'static str, String> {
    let shipped =
        capture("git", &["show", &format!("{commit}:xtask/src/main.rs")]).map_err(|e| {
            format!(
                "cannot read xtask/src/main.rs at {commit}: {e}\n\n\
             The manifest states whether a sweep runs from this package without a\n\
             repository, and it reads the source the package will carry to say so.\n\
             A commit whose xtask is unreadable is one nothing can state that about."
            )
        })?;
    Ok(if shipped.contains("fn manifest_commit") {
        SWEEP_FROM_MANIFEST
    } else {
        SWEEP_NEEDS_REPOSITORY
    })
}

/// What the release asserts, as the manifest states it.
///
/// # Why the manifest says this at all, when `claims/snapshot.json` is in the
/// package one row down
///
/// Because they answer different questions and only one of them is about the
/// release. The snapshot is the registry, serialised: it is what a document
/// renders a number from, and `lint-snapshot` requires it to be exactly what
/// `claims/` implies. This is the packager saying, in the file that is *about*
/// the package, which of those entries the release it just built is asserting
/// as gating — which is `RELEASING.md`'s second stopping condition written
/// where somebody opening the archive will read it rather than left as a thing
/// they have to go and look up.
///
/// Derived, never restated. Both this and the snapshot come from `claim_files`
/// and `claim_value`, so a claim whose status changes moves in both or in
/// neither, and a release note that repeated these counts in prose would be the
/// third copy and the first one able to drift. It is why `RELEASING.md` does not
/// repeat them.
///
/// A status this function does not know about is printed under its own name
/// rather than dropped. The registry's vocabulary is three words today; a
/// fourth added without this file noticing would otherwise produce a manifest
/// whose counts do not add up to its rows, which is the one failure a summary
/// must not have. E1-R02, RFC 0056.
fn claims_block() -> Result<String, String> {
    // Insertion order is the order these are printed in, and `claim_files` is
    // sorted, so the block is a function of the registry and of nothing else.
    // A `HashMap` here would be non-deterministic output in a content address.
    let mut by_status: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for path in claim_files()? {
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("reading {}: {e}", relative(&path)))?;
        let status = claim_value(&text, "status").unwrap_or_else(|| "unknown".into());
        by_status.entry(status).or_default().push(claim_name(&text, &path));
    }

    // Named rather than sorted alphabetically, because the order is what the
    // reader wants: what gates, then what is recorded, then what is owed.
    // Anything else follows, so a new status word appears rather than vanishes.
    let mut order: Vec<String> = ["gating", "tracked", "pending"]
        .iter()
        .map(|s| (*s).to_string())
        .filter(|s| by_status.contains_key(s))
        .collect();
    for status in by_status.keys() {
        if !order.contains(status) {
            order.push(status.clone());
        }
    }

    let mut out = String::new();
    out.push_str("# What this release asserts, derived from claims/ at the moment the\n");
    out.push_str("# package was built. `gating` fails the build on a regression, `tracked`\n");
    out.push_str("# records without gating, `pending` has a threshold and no number. The\n");
    out.push_str("# baseline, the workload and the one-command reproduction of each are in\n");
    out.push_str("# claims/<file>, inside source.tar; claims/snapshot.json is this same\n");
    out.push_str("# statement machine-readably. RELEASING.md is the contract.\n\n");

    let tally: Vec<String> =
        order.iter().map(|status| format!("{} {status}", by_status[status].len())).collect();
    out.push_str(&format!("claims  {}\n", tally.join(", ")));
    for status in &order {
        for name in &by_status[status] {
            out.push_str(&format!("  {status:<8} {name}\n"));
        }
    }
    out.push('\n');
    Ok(out)
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

    // Checked here rather than as an eighteenth verb, because it is the same
    // question this check already asks — whether a document still agrees with
    // the registry — asked about the four entries the registry does *not* have.
    // A number a document claims and the registry lacks is caught above; a
    // number a document says is absent, and the reason it is absent, is caught
    // by this. `E1-R02`, RFC 0056.
    gap_holds("DATAPATH_GAP", DATAPATH_GAP)?;

    if stale.is_empty() {
        println!("lint-claims: ok  ({} citation(s) match the registry)", references.len());
        println!(
            "lint-claims: the four datapath numbers are absent for {} declared reason(s), \
             each still true",
            DATAPATH_GAP.len()
        );
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

/// How one claim's workload is actually run.
///
/// One table, because the alternative is what was here before: a chain of `if
/// name ==` inside the runner and a separate belief, held nowhere in
/// particular, about which claims have a workload at all. A registry entry
/// whose published reproduction command dispatches to nothing is a claim only
/// its author can re-derive, which is the opposite of what the registry is for.
///
/// `lint-reproduce` reads this table and the registry together, so the two
/// cannot drift.
enum Route {
    /// A benchmark binary under `bench/src/bin/`, named here because the name
    /// is not derivable: `ring-submit-latency` runs `ring_submit`, and a
    /// derivation that happened to work for one claim was already carrying a
    /// `strip_prefix` special case for it.
    Bench(&'static str),
    /// Ten boots of the kernel. Not a program: the measurement is taken inside
    /// the kernel because nothing outside it can see where boot begins.
    Boots,
    /// The timer, for as many seconds as the claim's `[workload]` asks for.
    Timer,
    /// Every component killed under load, twice over. Not a program and not a
    /// boot: the workload is `cargo xtask chaos`, and the number it produces is
    /// a *count* — which is why it is the one claim in this epoch that may gate
    /// on a machine `f_bench::Environment` refuses to time on.
    Chaos,
    /// A long run marked as it goes and re-entered one minute before it fails.
    /// Not a program and not a boot: the workload is `cargo xtask snapshot`, and
    /// what it produces is a *ratio* between two wall-clock numbers taken in the
    /// same command on the same machine — which is why `claims/0007` is pending
    /// on the runner rather than gating on this one. E1-P08, RFC 0043.
    Snapshot,
    /// A hundred million hostile operations against a real channel region. Not
    /// a program and not a boot: the workload is `cargo xtask hostile`, and
    /// every number it produces is a **count** — operations performed, paths
    /// reached — which is why `claims/0008` may gate on this machine the way
    /// `claims/0005` does and for exactly that reason. E1-P04, RFC 0046.
    Hostile,
    /// A reservation refused, one granted and put under adversarial load, and
    /// the frame asked what this machine can hold. Not a program and not a
    /// boot: the workload is `cargo xtask admission`, and what it produces is a
    /// set of **counts** — refusals, periods met, slots taken, placements
    /// refused — which is why `claims/0010` may gate on this machine the way
    /// `claims/0005` does and for exactly that reason. `claims/0011` is the
    /// margin, which is a time, and it runs the same command while
    /// `f_bench::Environment` declines to record — the same shape
    /// `driver-restart-latency` has two routes up. E1-B07, RFC 0050.
    Admission,
    /// A quarter of a million generated submission entries, and the share of the
    /// entry-validation path the committed corpus covers. Not a program and not
    /// a boot: the workload is `cargo xtask entries`, and what it produces is a
    /// **percentage of lines** — the same figure on a fast host and a slow one,
    /// which is why `claims/0009` may gate on this machine for `claims/0005`'s
    /// reason. E1-P05, RFC 0048.
    Entries,
    /// Three boots of one client script against one block driver, differing in
    /// the ordinals the frame writes into its routing page. Not a program and
    /// not a benchmark: the workload is `cargo xtask deadline`, and what it
    /// produces is a **count** — how many queued batch operations a hard-class
    /// read was handed to the device ahead of — which is why `claims/0012` may
    /// gate on this machine the way `claims/0005` does and for exactly that
    /// reason. `claims/0013` is the time half and waits on a machine.
    /// E1-B06, RFC 0049.
    Deadline,
    /// One boot that retires forty buffer sets under each invalidation policy,
    /// and the same registry churn on the host beside it. Not a program alone
    /// and not a boot alone: the workload is `cargo xtask churn`, and what the
    /// boot produces is a **count** — invalidations per unmap request, round
    /// trips saved per set, shootdowns issued — which is why `claims/0014` may
    /// gate on this machine the way `claims/0005` does and for exactly that
    /// reason. `claims/0015` is the time half and waits on a machine.
    /// E1-B14, RFC 0052.
    Churn,
}

const ROUTES: &[(&str, Route)] = &[
    ("ring-submit-latency", Route::Bench("ring_submit")),
    ("timer-jitter", Route::Timer),
    ("boot-to-m0", Route::Boots),
    ("buffer-registration-cost", Route::Bench("buffer_register")),
    ("driver-restart-blast-radius", Route::Chaos),
    // The latency half of the same sentence, and the same workload: what
    // separates the two claims is that one is a count this machine may take and
    // the other is a time it may not. `claims/0006` says so in its own words,
    // and the command running the workload while the harness declines to record
    // is `bench/src/lib.rs` working rather than failing — the same shape
    // `buffer-registration-cost` has one claim over.
    ("driver-restart-latency", Route::Chaos),
    ("snapshot-re-entry-saving", Route::Snapshot),
    ("hostile-peer-operations", Route::Hostile),
    ("entry-validation-coverage", Route::Entries),
    ("admission-refusals", Route::Admission),
    // The margin half of the same sentence and the same workload: what separates
    // the two claims is that one is a count this machine may take and the other
    // is a time it may not. `claims/0011` says so in its own words, and the
    // command running the workload while the harness declines to record is
    // `bench/src/lib.rs` working rather than failing.
    ("reservation-margin", Route::Admission),
    ("deadline-overtake", Route::Deadline),
    // The time half of the same sentence and the same three boots: what
    // separates the two claims is that one is a count this machine may take and
    // the other is a latency it may not. `claims/0013` says so in its own words,
    // and the command running the workload while nothing records a time is
    // `bench/src/lib.rs`'s rule holding rather than failing — the same shape
    // `driver-restart-latency` has, seven claims up.
    ("deadline-overtake-latency", Route::Deadline),
    ("unmap-churn", Route::Churn),
    // The time half of the same sentence and the same command: what separates
    // the two claims is that one is a count this machine may take and the other
    // is a duration it may not. `claims/0015` says so in its own words, and the
    // command running the workload while nothing records a time is
    // `bench/src/lib.rs`'s rule holding rather than failing — the fourth pair
    // in this table with that shape.
    ("unmap-churn-cost", Route::Churn),
];

/// The registry file one claim name resolves to.
///
/// Exact match first, then a unique suffix, and an ambiguous suffix is an error
/// naming the candidates. What this replaces read the directory unsorted and
/// unfiltered and took the first `ends_with` hit — so it could match
/// `claims/README.md` or `claims/runner-class-A.md`, and among two real
/// candidates it picked whichever the filesystem happened to hand back first.
/// Nothing had two candidates yet, which is exactly when a resolution bug is
/// cheap to fix.
fn find_claim(name: &str) -> Result<PathBuf, String> {
    let files = claim_files()?;
    let stem = |p: &PathBuf| p.file_stem().map(|s| s.to_string_lossy().into_owned());

    if let Some(exact) = files.iter().find(|p| stem(p).as_deref() == Some(name)) {
        return Ok(exact.clone());
    }

    let matches: Vec<&PathBuf> = files
        .iter()
        .filter(|p| stem(p).is_some_and(|s| s.ends_with(name) && !name.is_empty()))
        .collect();

    match matches.as_slice() {
        [] => Err(format!("no claim named {name} in claims/. `cargo xtask claims` lists them.")),
        [one] => Ok((*one).clone()),
        many => Err(format!(
            "{name} names {} claims: {}. Say which.",
            many.len(),
            many.iter().filter_map(|p| stem(p)).collect::<Vec<_>>().join(", ")
        )),
    }
}

fn claim_run(name: Option<&str>) -> Result<(), String> {
    let Some(name) = name else {
        return Err("usage: cargo xtask claim <name>   (see `cargo xtask claims`)".into());
    };

    let file = find_claim(name)?;
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

    // Not every claim's workload is a benchmark binary, which is why there is a
    // table rather than a naming convention. `boot-to-m0` is ten boots and
    // `timer-jitter` is the timer; only one of the three is a program under
    // `bench/src/bin/`. Dispatching through `ROUTES` keeps `cargo xtask claim
    // <name>` as the one reproduction command the registry publishes, whatever
    // the workload turns out to be.
    //
    // The convention this replaced was `name.replace('-', "_")` with a
    // `strip_prefix` fixup for the one claim it did not fit, and it silently
    // did not fit a second: `cargo xtask claim timer-jitter` asked cargo for a
    // binary called `timer_jitter`, which has never existed.
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let route =
        ROUTES.iter().find(|(claim, _)| stem.ends_with(claim)).map(|(_, route)| route).ok_or_else(
            || {
                format!(
                    "claim {name} has no entry in ROUTES in xtask/src/main.rs, so its\n\
                 published reproduction command runs nothing. Add one, or the claim\n\
                 is a number only its author can re-derive."
                )
            },
        )?;

    match route {
        Route::Boots => claim_boot_to_m0()?,
        Route::Bench(bin) => sh("cargo", &["run", "--release", "-p", "f-bench", "--bin", bin])?,
        Route::Timer => {
            let seconds = toml_table_field(&text, "workload", "seconds");
            timer(seconds.as_deref())?;
        }
        Route::Chaos => chaos()?,
        Route::Snapshot => snapshot()?,
        Route::Hostile => hostile_gate()?,
        Route::Entries => {
            entries_gate()?;
            entries_coverage()?;
        }
        Route::Admission => admission_gate()?,
        Route::Deadline => deadline(None)?,
        Route::Churn => churn()?,
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
    use super::{
        JOIN_GAP, MINTS, code_mentions, datapath_findings, declared_fn, frame_findings,
        gap_holds_under, hold_the_gap, toml_field, toml_multiline, trace_hash, unspawned,
    };

    /// A component set shaped like the one this tree builds: two records, the
    /// hashes standing in for content ids.
    fn modelled(names: &[(&str, u64)]) -> Vec<(String, u64)> {
        names.iter().map(|(name, id)| ((*name).to_string(), *id)).collect()
    }

    const STORE: u64 = 0x55ff_b07c_0dfc_5864;
    const VIRTIO_BLK: u64 = 0xbf22_756d_7b8c_7b9c;

    #[test]
    fn the_gap_this_tree_declares_is_the_gap_this_tree_has() {
        // The state as of RFC 0044: the boot builds a place per component file,
        // so both are spawned and the difference is empty. This is the green
        // case, and it is here so that the two red cases below are known to be
        // red for their own reason rather than because the shape is always
        // refused.
        let set = modelled(&[("store", STORE), ("virtio-blk", VIRTIO_BLK)]);
        let gap = unspawned(&set, &[STORE, VIRTIO_BLK]);
        assert!(gap.is_empty(), "the boot spawns every component file this tree builds");
        assert_eq!(hold_the_gap(&gap, JOIN_GAP), Ok(()));
    }

    #[test]
    fn a_component_the_boot_never_spawns_is_refused() {
        // **The input the review named, run.** Drop a third component file with
        // a modelled protocol into `target/component/`: the simulator runs
        // three, the boot spawns the two it was handed, and before RFC 0036 this
        // printed `join: ok` while the workload half covered a component the
        // kernel never instantiated.
        //
        // This is the direction an empty `JOIN_GAP` still exercises against the
        // real constant, and it is the one a growing tree meets: adding a
        // component file is red until a boot spawns it.
        let set =
            modelled(&[("store", STORE), ("virtio-blk", VIRTIO_BLK), ("virtio-net", 0xABCD_1234)]);
        let gap = unspawned(&set, &[STORE, VIRTIO_BLK]);
        let refused = hold_the_gap(&gap, JOIN_GAP).expect_err("a third component is not declared");
        assert!(refused.contains("virtio-net"), "the refusal does not name what appeared");
        assert!(refused.contains("JOIN_GAP"), "the refusal does not say where to declare it");
    }

    #[test]
    fn a_declared_component_the_boot_has_started_spawning_is_refused() {
        // The other direction, and it is stated against a list this test
        // supplies rather than against `JOIN_GAP`, because `JOIN_GAP` is empty
        // now and an empty list has no entry that could go stale. Saying so is
        // the point: the check still refuses a stale exception, and the *tree*
        // no longer has one to refuse. A test that quietly stopped covering this
        // direction would leave the next person who adds an entry with a
        // half-checked mechanism.
        let set = modelled(&[("store", STORE), ("virtio-blk", VIRTIO_BLK)]);
        let gap = unspawned(&set, &[STORE, VIRTIO_BLK]);
        assert!(gap.is_empty());
        assert!(
            hold_the_gap(&gap, &["virtio-blk"]).is_err(),
            "a stale exception was accepted: the boot spawns it and the list still names it"
        );
        // And with the entry gone it is green again, which is what says the
        // refusal above is about the list and not about the boot.
        assert_eq!(hold_the_gap(&gap, &[]), Ok(()));
    }

    #[test]
    fn the_difference_is_computed_by_identity_and_not_by_name() {
        // A component whose image changed has a different content hash and is
        // therefore a different component here, which is the whole reason the
        // join reads hashes out of the log rather than names: two builds of one
        // name are two components, and the half that ran the other one is not
        // the half that ran this one.
        let set = modelled(&[("store", STORE), ("virtio-blk", VIRTIO_BLK)]);
        assert_eq!(unspawned(&set, &[STORE ^ 1]), ["store", "virtio-blk"]);
    }

    /// A driver crate shaped like `user/virtio-blk`: one function that moves
    /// bytes, called once, from the boot's own self-check.
    const DATAPATH_HELD: &str = concat!(
        "impl Driver {\n",
        "    /// Resolve a name and hand the address to the device.\n",
        "    fn transfer(&mut self, entry: &Sqe) -> Result<Cqe, Refusal> {\n",
        "        let reach = path.resolve(name, entry.len)?;\n",
        "        self.round_trip(reach.address, entry.len)\n",
        "    }\n",
        "\n",
        "    pub fn provoke_copy(&mut self) -> Result<(), Trouble> {\n",
        "        stage(&self.control, FROM, TO, BYTES, &mut self.counters.provoked)\n",
        "    }\n",
        "}\n",
        "\n",
        "fn stage(region: &Region, from: u32, to: u32, len: u32, tally: &mut u64) {\n",
        "    // The only function in this crate that moves bytes.\n",
        "}\n",
        "\n",
        "#[cfg(test)]\n",
        "mod tests {\n",
        "    fn region() -> Region { Region::at(base, device, LEN).expect(\"aligned\") }\n",
        "}\n",
    );

    #[test]
    fn a_datapath_that_moves_no_bytes_is_the_shape_the_lint_passes() {
        let (findings, calls) =
            datapath_findings("x.rs", DATAPATH_HELD, "stage", "provoke_copy", MINTS);
        assert_eq!(findings, Vec::<String>::new(), "the held shape reports nothing");
        assert_eq!(calls, 1, "and the one call is counted");
    }

    #[test]
    fn a_copy_on_the_data_path_is_a_finding_the_counter_would_not_show() {
        // The fixture that breaks the lint, and the reason the lint exists: this
        // crate publishes `copies = 0` — `transfer` passes `provoked`, not
        // `copies` — while copying a client's bytes on every request. The
        // counter is deaf to it and the source is not.
        let broken = DATAPATH_HELD.replace(
            "        self.round_trip(reach.address, entry.len)\n",
            "        stage(&self.scratch, 0, 512, entry.len, &mut self.counters.provoked)?;\n\
             \x20       self.round_trip(reach.address, entry.len)\n",
        );
        let (findings, calls) = datapath_findings("x.rs", &broken, "stage", "provoke_copy", MINTS);
        assert_eq!(calls, 2, "two call sites now");
        assert_eq!(findings.len(), 1, "and one of them is not `provoke_copy`");
        assert!(findings[0].contains("`stage` called from `transfer`"), "{}", findings[0]);
    }

    #[test]
    fn a_window_minted_out_of_an_invented_address_is_a_finding() {
        // The scan, driven against a list this test supplies rather than
        // against `MINTS` — which RFC 0047 emptied, because the driver runs at
        // ring 3 now and an address it invents is a page fault rather than a
        // client's bytes.
        //
        // Kept, and kept *working*, for the reason this file applies to every
        // other check in it: a mechanism with nothing to find is
        // indistinguishable from one that cannot find anything, and the day a
        // component's code is linked into the frame again is the day the direct
        // map is under it again and this is what was holding. The second half
        // of this test is the retirement itself, stated as a check rather than
        // as a comment.
        let broken = DATAPATH_HELD.replace(
            "        let reach = path.resolve(name, entry.len)?;\n",
            "        let reach = path.resolve(name, entry.len)?;\n\
             \x20       let peek = Region::at(DIRECT_MAP + reach.address, 0, entry.len)?;\n",
        );
        let held = ["Region::at(", "Window::at("];
        let (findings, _) = datapath_findings("x.rs", &broken, "stage", "provoke_copy", &held);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("Region::at"), "{}", findings[0]);

        let (retired, _) = datapath_findings("x.rs", &broken, "stage", "provoke_copy", MINTS);
        assert!(
            retired.is_empty(),
            "`MINTS` is empty on purpose: the address space refuses what this used to look for"
        );
    }

    #[test]
    fn a_needle_no_crate_defines_is_a_finding_rather_than_a_green_rule() {
        // `NOT_THE_FRAME`'s third field, and the fixture that breaks it.
        //
        // The absence half is a search for a *name*. Rename `Driver` in
        // `user/virtio-blk` and `kernel/` stops naming it for a reason that has
        // nothing to do with who runs the code, which is a green lint over a
        // frame calling a component. So the needle has to still refer to
        // something, and that is what this counts.
        let owns = "pub struct Driver;\nimpl Driver { pub fn start() {} }\n";
        assert_eq!(code_mentions(owns, "Driver::"), 0, "a declaration is not a use of the path");
        let uses = "fn boot() { let d = Driver::start(); }\n";
        assert_eq!(code_mentions(uses, "Driver::"), 1);

        // The rename, modelled: the crate that owned the name no longer spells
        // it, so nothing under the owning prefix names it and the count is zero
        // — which is the input `lint_datapath` turns into a finding.
        let renamed = uses.replace("Driver::", "Engine::");
        assert_eq!(code_mentions(&renamed, "Driver::"), 0);

        // And the two exclusions this shares with `frame_findings`, stated so
        // that neither can be what keeps the rule alive: prose about the
        // reversal, and a fixture below `#[cfg(test)]`.
        let recorded = "// `Driver::execute` used to be called here. RFC 0047.\nfn turn() {}\n";
        assert_eq!(code_mentions(recorded, "Driver::"), 0);
        let fixture = "fn turn() {}\n#[cfg(test)]\nmod t { fn f() { Driver::start(); } }\n";
        assert_eq!(code_mentions(fixture, "Driver::"), 0);
    }

    #[test]
    fn a_declared_gap_that_has_closed_is_a_red_build() {
        // The mechanism under `OWED_REVERSALS` and `CHAOS_GAP`, with its own
        // fixture — four declared quantities rest on it, and until now the only
        // evidence it could fail was that it had not.
        let base = std::env::temp_dir().join("f-xtask-gap-fixture");
        let dir = base.join("kernel").join("src");
        std::fs::create_dir_all(&dir).expect("a temporary directory");
        let file = dir.join("component.rs");
        let gap: &[super::Gap] = &[(
            "kernel/src/component.rs",
            "policy::decide(",
            "the reason it is still owed",
            "TODO.md E1-B05; docs/rfc/0008",
        )];

        // Held: the text the row names is there, so the deviation is still open
        // and the build is green.
        std::fs::write(&file, "fn restart() { policy::decide(&record, &tally); }\n").unwrap();
        assert_eq!(gap_holds_under(&base, "FIXTURE", gap), Ok(()));

        // Paid: the text is gone, which is the good news this refuses on
        // purpose, and the refusal has to name both the needle and the constant
        // so that whoever closed it knows which documents now describe a tree
        // that does not exist.
        std::fs::write(&file, "fn restart() { supervisor.tell(notice::PEER_GONE); }\n").unwrap();
        let refused = gap_holds_under(&base, "FIXTURE", gap)
            .expect_err("a gap whose needle is gone must not stay green");
        assert!(refused.contains("policy::decide("), "{refused}");
        assert!(refused.contains("kernel/src/component.rs"), "{refused}");
        assert!(refused.contains("FIXTURE"), "the refusal does not say which constant to edit");
        // The fourth field, and the reason it is there: the last time a gap
        // closed, the constants were updated and five other live documents were
        // not. A refusal that says *update the documents* without naming them
        // is an instruction that assumes the reader knows the answer.
        assert!(refused.contains("TODO.md E1-B05"), "the refusal names no document to update");
        assert!(refused.contains("docs/rfc/0008"), "{refused}");

        // And a row naming a file that is not there is a declaration nobody can
        // check, which is the other way this stops meaning anything.
        std::fs::remove_file(&file).unwrap();
        let refused = gap_holds_under(&base, "FIXTURE", gap).expect_err("a missing file is not ok");
        assert!(refused.contains("nobody can check"), "{refused}");
        std::fs::remove_dir_all(&base).ok();
    }

    #[test]
    fn a_frame_that_calls_a_components_driver_is_a_finding() {
        // RFC 0033's reversal condition, and the fixture that breaks it.
        //
        // The first assertion is the hole, stated rather than left to be found:
        // a *method call on a value* does not name the type, so this needle
        // does not see one. It does not have to. The value cannot exist without
        // `Driver::start`, which is a constructor and does name the type, so a
        // frame that had gone back to running the driver is caught at the line
        // that brought the device up rather than at the line that used it.
        //
        // The last assertion is the direction that matters more day to day: the
        // prose in this tree talks about `Driver::execute` at length, because
        // that is how a reversal is recorded, and a check that failed on a
        // comment would make recording it impossible.
        let called = "    let answer = driver.execute(&entry, &mut asking, 0);\n";
        let calling = format!("fn turn() {{\n{called}}}\n");
        let findings = frame_findings("kernel/src/blk.rs", &calling, "Driver::");
        assert!(findings.is_empty(), "a method call on a value is not what the needle names");

        let naming = "use f_virtio_blk::driver::Driver;\nfn turn() { Driver::start(); }\n";
        let findings = frame_findings("kernel/src/blk.rs", naming, "Driver::");
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("Driver::"), "{}", findings[0]);

        let recorded =
            "// The frame used to call `Driver::execute` here. RFC 0047.\nfn turn() {}\n";
        assert!(
            frame_findings("kernel/src/blk.rs", recorded, "Driver::").is_empty(),
            "a reversal this tree records in prose must not fail the build that records it"
        );
    }

    #[test]
    fn a_mover_the_fixtures_alone_call_is_not_a_mover_the_boot_moved() {
        // The `MEMORY_FORCED` half, at the level of the lint rather than of the
        // boot: delete the one call and both counters publish zero, which is a
        // green board and a counter nobody has ever seen move.
        let deaf = DATAPATH_HELD.replace(
            "        stage(&self.control, FROM, TO, BYTES, &mut self.counters.provoked)\n",
            "        Ok(())\n",
        );
        let (findings, calls) = datapath_findings("x.rs", &deaf, "stage", "provoke_copy", MINTS);
        assert_eq!(calls, 0);
        assert!(findings.is_empty(), "the per-line scan sees nothing wrong — the count does");
    }

    #[test]
    fn a_declaration_names_its_function_through_every_prefix_this_tree_writes() {
        assert_eq!(declared_fn("fn stage(region: &Region)"), Some("stage"));
        assert_eq!(declared_fn("    pub fn provoke_copy(&mut self)"), Some("provoke_copy"));
        assert_eq!(declared_fn("    pub const fn counters(&self) -> Counters"), Some("counters"));
        assert_eq!(declared_fn("    pub(crate) unsafe fn read8(base: u64)"), Some("read8"));
        assert_eq!(declared_fn("        let x = stage(&r, 0, 1, 2, &mut t);"), None);
        // A form nobody here writes reports nothing, which leaves the enclosing
        // function as it was and attributes the next call to it — refused.
        assert_eq!(declared_fn("extern \"C\" fn trampoline()"), None);
    }

    /// The string `f_sim::trace::digest` is pinned against, and the value both
    /// functions must produce.
    const DIGEST_FIXTURE: &str = "F reproduction fixture
0123456789";

    /// FNV-1a of [`DIGEST_FIXTURE`], with the carriage return skipped.
    const DIGEST_FIXTURE_VALUE: u64 = 0xea6c_1d51_99fa_61cd;

    #[test]
    fn the_sim_digest_is_the_one_this_file_hashes_boot_logs_with() {
        // Two reproduction checks, two copies of one hash function, and this is
        // what keeps them one function. `sim/src/trace.rs` carries the twin of
        // this test over the same string against the same constant; if either
        // has to change, both change, or the boot's check and the simulator's
        // have quietly stopped speaking one language. RFC 0032.
        assert_eq!(trace_hash(DIGEST_FIXTURE), DIGEST_FIXTURE_VALUE);
    }

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
    fn a_baseline_named_in_prose_is_caught() {
        // The state every named baseline was in before `E1-D06`: a claim that
        // says which machine it was compared against and gives nobody a way to
        // be that machine.
        let findings = baseline_findings("claims/0001-x.toml", "linux-6.x-tuned", "");
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].contains("prose"), "{findings:?}");
    }

    #[test]
    fn a_baseline_directory_with_no_verifier_is_caught() {
        // `claims/baselines/linux-6.x-tuned` exists and has all three, so the
        // fixture points one level down at a name that does not: the finding
        // has to be about the missing files rather than about the missing
        // directory, because a directory of notes with no `verify.sh` is the
        // failure that reads as a success.
        let findings =
            baseline_findings("claims/0001-x.toml", "l", "claims/baselines/no-such-baseline");
        assert_eq!(findings.len(), 3, "{findings:?}");
        assert!(findings.iter().any(|f| f.contains("verify.sh")), "{findings:?}");
    }

    #[test]
    fn a_baseline_outside_what_the_release_packages_is_caught() {
        // Green lint, empty package. `ContentSource::Dir("claims/baselines")`
        // is the whole of what a release carries, so a baseline elsewhere is
        // one no stranger receives — and `..` is the same sentence with the
        // tree's edge in it.
        for path in ["docs/baselines/linux", "claims/baselines/../../etc"] {
            let findings = baseline_findings("claims/0001-x.toml", "linux-6.x-tuned", path);
            assert_eq!(findings.len(), 1, "{path}: {findings:?}");
            assert!(findings[0].contains("no release contains"), "{path}: {findings:?}");
        }
    }

    #[test]
    fn a_directory_content_reaches_below_its_top_level_and_is_sorted() {
        // The two properties `ContentSource::Dir` exists for. The first is why
        // it is not `Tree`: `claims/baselines` holds a directory per baseline
        // version, so a walk that stopped at the top would package nothing. The
        // second is invisible on one machine and is the whole of what makes a
        // package byte-identical on two, since `read_dir` order is the
        // filesystem's business.
        let content = CONTENTS
            .iter()
            .find(|content| matches!(content.source, ContentSource::Dir(_)))
            .expect("the baseline configuration is the one Dir content");
        let files = content_files(content).expect("claims/baselines is readable");
        let names: Vec<&str> = files.iter().map(|(name, _)| name.as_str()).collect();
        assert!(
            names.iter().any(|name| name.ends_with("/apply.sh")),
            "the walk must descend into the per-version directory: {names:?}"
        );
        let mut sorted = names.clone();
        sorted.sort_unstable();
        assert_eq!(names, sorted, "unsorted: two machines would pack two archives");
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
        assert!(lint_manifests().is_ok(), "a component manifest no longer fits the schema");
    }
}

/// One registry entry, reduced to what a stranger needs to re-run it.
struct Reproduction {
    name: String,
    file: String,
    command: String,
    runner: String,
    status: String,
    /// `[baseline] system`. `none` is a real answer and two claims give it.
    baseline: String,
    /// `[baseline] path`, when the baseline is a directory somebody can apply
    /// rather than a sentence somebody has to interpret.
    baseline_path: String,
}

fn reproductions() -> Result<Vec<Reproduction>, String> {
    let mut out = Vec::new();
    for file in claim_files()? {
        let rel = relative(&file);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("reading {rel}: {e}"))?;
        out.push(Reproduction {
            name: toml_field(&text, "name").unwrap_or_default(),
            file: rel,
            command: toml_table_field(&text, "reproduce", "command").unwrap_or_default(),
            runner: toml_table_field(&text, "hardware", "runner").unwrap_or_default(),
            status: toml_field(&text, "status").unwrap_or_else(|| "unknown".into()),
            baseline: toml_table_field(&text, "baseline", "system").unwrap_or_default(),
            baseline_path: toml_table_field(&text, "baseline", "path").unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Re-run one published claim from this checkout, or list what there is to run.
///
/// # What this is for
///
/// A stranger with the repository and nothing else. `RELEASING.md` promises
/// that every published number can be re-derived by somebody who was not
/// there, and until this verb existed that promise was three different commands
/// in three registry files, one of which did not run.
///
/// It is a **dispatcher over the registry**, deliberately, and not a
/// reimplementation of each claim. The claim file says how it is reproduced;
/// this reads that and does it. Anything else and there are two accounts of how
/// a number was taken, which is the decay the whole registry exists to prevent.
///
/// # Why an honest refusal exits zero
///
/// Because on every machine this project can currently reach, refusal is the
/// path that runs. `F_ENVIRONMENT` is `container` in the development image and
/// unset everywhere else, and `f_bench::Environment` fails closed on both — so
/// the workload runs, a distribution is drawn, and recording is refused. That
/// is correct behaviour and not a failure. Painting it red would make every
/// local run of this command a red run, which is precisely how a check gets
/// muted.
///
/// The three endings are therefore distinguished in words rather than in the
/// exit code, and there is no shared "ok": *the number was recorded*, *the
/// route ran and the number was not recorded*, and an error, which means the
/// route itself broke.
///
/// # Errors
///
/// No such claim, an ambiguous name, or a workload that did not complete.
fn reproduce(name: Option<&str>) -> Result<(), String> {
    let Some(name) = name else {
        return reproduce_list();
    };

    let file = find_claim(name)?;
    let rel = relative(&file);
    let text = std::fs::read_to_string(&file).map_err(|e| format!("reading {rel}: {e}"))?;

    let claim = toml_field(&text, "name").unwrap_or_else(|| name.to_string());
    let status = toml_field(&text, "status").unwrap_or_else(|| "unknown".into());
    let runner = toml_table_field(&text, "hardware", "runner").unwrap_or_else(|| "unset".into());
    let command = toml_table_field(&text, "reproduce", "command").unwrap_or_else(|| "unset".into());

    // The tree being reproduced from, named. A number quoted from a tree nobody
    // can identify is the failure `release --dry-run` was carrying until
    // E0-P01; here it is a warning rather than fatal, because reproducing on a
    // branch is a legitimate thing to be doing — what matters is that the
    // printed record says so.
    let commit = capture("git", &["rev-parse", "--short", "HEAD"]).unwrap_or_default();
    let dirty = capture("git", &["status", "--porcelain"]).is_ok_and(|s| !s.trim().is_empty());

    println!("claim     {claim}");
    println!("file      {rel}");
    println!("status    {status}");
    println!(
        "commit    {}{}",
        commit.trim(),
        if dirty { "  (dirty — not a quotable tree)" } else { "" }
    );
    println!("needs     {runner}");
    println!("command   {command}");

    let environment = f_bench::Environment::detect();
    println!("machine   {} — {}", environment.name(), environment.why());
    println!();

    claim_run(Some(name))?;

    println!();
    if environment.records() {
        println!("reproduce: route ok — the number was recorded.");
        return Ok(());
    }

    println!(
        "reproduce: route ok, and the number was not recorded.\n\n\
         This machine is `{}`, and {}.\n\n\
         What would record it: a machine meeting `claims/{runner}.md` with all four\n\
         of RFC 0007's reservation components obtained by partition, and\n\
         `F_ENVIRONMENT={runner}` set on it. That file's own warning applies —\n\
         the variable is an assertion, not a measurement, so read the checklist\n\
         in it before setting one.",
        environment.name(),
        environment.why()
    );
    Ok(())
}

/// Every claim, its command, the machine it needs, and whether this is one.
fn reproduce_list() -> Result<(), String> {
    let environment = f_bench::Environment::detect();
    println!("this machine is `{}` — {}", environment.name(), environment.why());
    println!();

    for entry in reproductions()? {
        let mine = if entry.runner == environment.name() { "yes" } else { "no" };
        println!(
            "  {:<22} {:<8} needs {:<16} here? {mine}",
            entry.name, entry.status, entry.runner
        );
        println!("  {:<22} {}", "", entry.command);
        println!();
    }

    println!("run one with: cargo xtask reproduce <claim>");
    Ok(())
}

/// What one claim's `[baseline]` block owes, given its `system` and its `path`.
///
/// A named baseline is either `none` — which `claims/0002` and `0003` both are,
/// and argue for in their own notes — or a directory a stranger can apply to a
/// machine. There is no third option, and before `E1-D06` every named baseline
/// was one: a sentence describing a configuration, which is
/// `claims/README.md` rule 1's decay with nothing to stop it, because a
/// sentence cannot be re-run and so cannot be found to have stopped being true.
///
/// The path is checked for where it points as well as for what is in it. The
/// release packages `claims/baselines` and nothing else, so a baseline
/// directory outside it is complete on disk, green in this lint, and absent
/// from every package — which is the same failure as the `Absent` row this
/// check replaced, arriving through the door the fix opened. `..` is refused
/// for the same sentence: a path that leaves the tree names a baseline no
/// clone contains.
///
/// A free function rather than a branch inside the lint because a lint that has
/// never failed is indistinguishable from a lint that cannot, and this is the
/// shape `mechanised_rules` can hand a fixture to.
fn baseline_findings(file: &str, baseline: &str, path: &str) -> Vec<String> {
    let mut findings = Vec::new();
    if baseline.is_empty() || baseline == "none" {
        return findings;
    }
    if path.is_empty() {
        findings.push(format!(
            "  {file}: names the baseline `{baseline}` and gives no `[baseline] path`, so the \
             baseline is prose"
        ));
        return findings;
    }
    if !path.starts_with("claims/baselines/") || path.split('/').any(|part| part == "..") {
        findings.push(format!(
            "  {file}: baseline `{path}` is not under `claims/baselines/`, which is the whole \
             of what a release packages — so it is a baseline no release contains"
        ));
        return findings;
    }
    // The three files that make a baseline directory a baseline rather than a
    // folder of notes: what it is, how it is applied, and how a machine says it
    // has drifted out of it.
    for needed in ["README.md", "apply.sh", "verify.sh"] {
        if !root().join(path).join(needed).exists() {
            findings.push(format!("  {file}: baseline `{path}` has no {needed}"));
        }
    }
    findings
}

/// Every claim's published reproduction command resolves inside this tree.
///
/// `RELEASING.md`'s second gate says a claim in the snapshot with no
/// reproduction command that runs from a clean checkout is a release that does
/// not go out. That was prose. This is the executable half, and it asserts four
/// things a stranger's afternoon depends on:
///
/// - the command exists at all;
/// - it is `cargo xtask claim <this claim's own name>`, so no claim can publish
///   a command that names somebody else's, and none can name a step outside the
///   tree — the long plan's rule, applied to the one place it is easiest to
///   break;
/// - the name resolves through `ROUTES` to something that runs;
/// - and the runner class it requires has a specification file beside it, so
///   `[hardware] runner` cannot name a machine nobody has described.
///
/// The last of those is what `E0-D10` made checkable. Before
/// `claims/runner-class-A.md` existed there was nothing for this to point at.
///
/// # Errors
///
/// Any claim failing any of the four, with the file named.
fn lint_reproduce() -> Result<(), String> {
    let mut findings = Vec::new();

    for entry in reproductions()? {
        if entry.name.is_empty() {
            findings.push(format!("  {}: no `name`", entry.file));
            continue;
        }
        let expected = format!("cargo xtask claim {}", entry.name);
        if entry.command.is_empty() {
            findings.push(format!("  {}: no `[reproduce] command`", entry.file));
        } else if entry.command != expected {
            findings.push(format!(
                "  {}: reproduces with `{}`, and the registry's one command is `{expected}`",
                entry.file, entry.command
            ));
        }
        if !ROUTES.iter().any(|(claim, _)| *claim == entry.name) {
            findings.push(format!(
                "  {}: `{}` has no ROUTES entry, so its command runs nothing",
                entry.file, entry.name
            ));
        }
        findings.extend(baseline_findings(&entry.file, &entry.baseline, &entry.baseline_path));
        if entry.runner.is_empty() {
            findings.push(format!("  {}: no `[hardware] runner`", entry.file));
        } else {
            let spec = root().join("claims").join(format!("{}.md", entry.runner));
            if !spec.exists() {
                findings.push(format!(
                    "  {}: needs `{}`, and claims/{}.md does not exist",
                    entry.file, entry.runner, entry.runner
                ));
            }
        }
    }

    if findings.is_empty() {
        println!(
            "lint-reproduce: ok  ({} claim(s) reproduce from this tree)",
            reproductions()?.len()
        );
        return Ok(());
    }
    Err(format!(
        "{} claim(s) cannot be reproduced from this tree:\n{}\n\n\
         RELEASING.md gate 2: a claim in the snapshot whose reproduction does not\n\
         run from a clean checkout is a number a stranger cannot check, which is\n\
         the one thing the registry exists to prevent.",
        findings.len(),
        findings.join("\n")
    ))
}

// ---------------------------------------------------------------------------
// E1-P04 — the hostile-peer fuzzer, its Miri half, its harness and its corpus.
//
// The division of labour is `sweep`'s, restated because it is the reason a
// number out of here is worth anything:
//
//   the fuzzer  draws the peer's behaviour from a seed, drives the honest end,
//               counts every path it reached, and reports a finding as a seed
//               and an episode. It reads no clock — `lint-determinism` scans
//               `ring/` with no allow-list entry, so it could not.
//   xtask       supplies the count, the wall clock, and the verdict about
//               whether a run that printed nothing printed nothing for a good
//               reason. RFC 0046.
// ---------------------------------------------------------------------------

/// Operations `cargo xtask hostile` runs when it is not told a count.
/// Unit: operations.
///
/// A hundred million: **4.4 s to 7.3 s** in release on the four-core
/// development container, over four runs, which is what makes it affordable in
/// `verify`. A range and not the best of the four, for the reason the exit's
/// own figure is a range: the host is shared, and a cost quoted at its minimum
/// is one somebody later cannot reproduce and quietly stops running.
///
/// It is **not** the exit's number. `claims/0008-hostile-peer-operations.toml`
/// carries both as thresholds — `operations` is this constant and
/// `exit_operations` is [`HOSTILE_EXIT`] — and `hostile_thresholds_match`
/// requires the registry and these constants to agree on every run, so neither
/// number can move without the other noticing.
const HOSTILE_GATE: u64 = 100_000_000;

/// `E1-P04`'s own number. Unit: operations.
///
/// Measured at 44.3 s to 60.3 s in release here, over three runs on a shared
/// host. It is a constant because a number in a workflow file and a number in a
/// claim drift, and this is the one the exit is about — and it is a
/// **threshold** in `claims/0008` (`exit_operations`) rather than only a line in
/// its prose, checked against this constant by `hostile_thresholds_match` on
/// every ordinary run. A registry that published a billion while gating a
/// hundred million with nothing tying the two together is the shape this
/// arrangement exists to refuse.
const HOSTILE_EXIT: u64 = 1_000_000_000;

/// Operations one Miri run performs when it is not told a count.
/// Unit: operations.
///
/// Four thousand and ninety-six — four episodes, about 45 s of interpretation
/// after a minute of sysroot. Miri costs roughly six orders of magnitude, so
/// this is the count at which the unsafety property is checked per commit, and
/// the nightly's is sixteen times larger. Reporting both is the whole of RFC
/// 0046's first decision.
///
/// The distance between this and [`HOSTILE_EXIT`] is the exit's one unmet
/// conjunct, and it is a **number in the registry** rather than a paragraph:
/// `claims/0008`'s `unsafety_gap` is `exit_operations / miri_operations`, and
/// `hostile_thresholds_match` recomputes it from these constants on every run.
/// Raising the exit's count without raising Miri's widens the gap and goes red,
/// which is the only mechanism available for a property no tool can check at the
/// exit's own scale. `JOIN_GAP` and `CHAOS_GAP` are the same discipline.
const HOSTILE_MIRI: u64 = 4096;

/// Operations the mutation harness runs against one armed defect.
/// Unit: operations.
///
/// A hundred thousand. Each of the three defects is reached inside the first
/// handful of episodes — the harness fails loudly if that stops being true,
/// which is the reversal condition rather than a comment.
const HOSTILE_MUTATE_OPS: u64 = 100_000;

/// The defect that breaks *no panic*: `Mapping::adopt` believes the layout the
/// peer described.
const HOSTILE_DEFECT_PANIC: &str = "mutate-believed-header";

/// The defect that breaks *no memory unsafety*: `Consumer::pop` reads through
/// the slot number a peer wrote.
const HOSTILE_DEFECT_UNSAFE: &str = "mutate-trusted-slot";

/// The defect that breaks *no hang*: `Service::drain` ignores its budget.
const HOSTILE_DEFECT_STUCK: &str = "mutate-unbounded-drain";

/// Where `claims/0008` lives, relative to the workspace root.
///
/// Read rather than restated: every count and every reach minimum this file
/// enforces comes out of that file, so a threshold that lives here and a
/// threshold that lives there cannot be two numbers.
const HOSTILE_CLAIM: &str = "claims/0008-hostile-peer-operations.toml";

/// Episodes replayed as a **control** when the corpus is recorded and again when
/// `--mutate` checks the corpus can go red. Unit: episode indices.
///
/// They are episodes of [`TRACE_SEED`] that are deliberately *not* corpus
/// entries, and what they measure is the one thing an entry's provenance does
/// not say: whether the entry is **rare**. A corpus of runs that found something
/// is a regression suite only if a run that did not find it exists — otherwise
/// every line in the file carries exactly the information an arbitrary episode
/// carries, which is none.
const HOSTILE_CONTROL: &[u64] = &[1, 3, 7, 11, 101];

/// How many of [`HOSTILE_CONTROL`] reproduce each defect. Unit: episodes.
///
/// # What this is for
///
/// A corpus entry says *this run found something once*. It cannot say whether a
/// run that did **not** find it exists, and if none does then the entry carries
/// exactly the information an arbitrary episode carries, which is none. This is
/// that measurement, taken when an entry is recorded and checked on every
/// `--mutate` run.
///
/// # The two answers differ, and that is the useful part
///
/// `mutate-believed-header` is reached by **every** control episode: a corpus
/// entry recorded under it is provenance — a seed, a commit, a defect and an
/// evidence line that outlive the run — and not rarity, and `ring/corpus.txt`
/// says so rather than implying otherwise.
///
/// `mutate-unbounded-drain` is reached by **one of five**: an entry recorded
/// under it does carry something an arbitrary episode does not, which is what a
/// regression suite is supposed to be. The corpus earns its keep on one of the
/// two defects and not on the other, and that is a more useful thing to have
/// written down than an average.
///
/// Checked for equality the way `JOIN_GAP` is, because **both** directions are
/// information: a defect that became easier to reach and one that became harder
/// are each a fact about the generator, and neither should be discovered as a
/// silence next to a green run.
const HOSTILE_SELECTIVITY: &[(&str, usize)] =
    &[(HOSTILE_DEFECT_PANIC, 5), (HOSTILE_DEFECT_STUCK, 1)];

/// The declared control count for one defect, or `None` if it has none.
fn hostile_selectivity(defect: &str) -> Option<usize> {
    HOSTILE_SELECTIVITY.iter().find(|(name, _)| *name == defect).map(|(_, hits)| *hits)
}

/// Where the hostile corpus lives, relative to the workspace root.
///
/// In the tree rather than under `target/`, for `sim/corpus.txt`'s reason: a
/// corpus is the one artefact of a fuzzing run that is supposed to outlive the
/// run, and a build directory is where things go to be deleted.
const HOSTILE_CORPUS: &str = "ring/corpus.txt";

/// Miri's flags for this suite.
///
/// `-Zmiri-permissive-provenance` and nothing else. It silences the
/// integer-to-pointer warning that `f_ring::adopt` earns by carrying a channel
/// base as a `u64` — which is RFC 0037's design and not a defect — at the cost
/// of weaker aliasing checking on that one path. RFC 0046 declares that as
/// `MIRI_GAP` and `claims/0008` names it beside the number, rather than leaving
/// a reader to find it out from a warning that scrolled past.
const HOSTILE_MIRIFLAGS: &str = "-Zmiri-permissive-provenance";

/// Run the fuzzer once, answering `(clean, output)`.
///
/// A non-zero exit is *a finding* rather than an error — the binary uses the
/// status that way deliberately — so this cannot use [`capture`], which treats
/// one as a failure. The output is printed as well as returned, because the
/// report is the thing a person came for and a harness that swallowed it would
/// make its own summary the only evidence.
fn hostile_run(features: &[&str], args: &[&str], miri: bool) -> Result<(bool, String), String> {
    hostile_run_reported(features, args, miri, true)
}

/// The same, with the fuzzer's report captured rather than printed.
///
/// For the one caller that runs the fuzzer several times to measure something
/// about the *runs* rather than to show one: [`hostile_control_hits`] replays
/// five episodes and what matters is how many of them found something, not five
/// reports of twenty-six counters each. Everything a person came for is still
/// printed by the step that calls it.
fn hostile_run_quietly(
    features: &[&str],
    args: &[&str],
    miri: bool,
) -> Result<(bool, String), String> {
    hostile_run_reported(features, args, miri, false)
}

fn hostile_run_reported(
    features: &[&str],
    args: &[&str],
    miri: bool,
    loud: bool,
) -> Result<(bool, String), String> {
    let mut argv: Vec<String> = if miri {
        ["miri", "test", "-q", "-p", "f-ring", "--test", "hostile"]
    } else {
        ["test", "-q", "--release", "-p", "f-ring", "--test", "hostile"]
    }
    .iter()
    .map(|s| (*s).to_string())
    .collect();
    if !features.is_empty() {
        argv.push("--features".into());
        argv.push(features.join(","));
    }
    argv.push("--".into());
    argv.extend(args.iter().map(|s| (*s).to_string()));

    let mut command = Command::new("cargo");
    command.args(&argv).current_dir(root());
    if miri {
        command.env("MIRIFLAGS", HOSTILE_MIRIFLAGS);
    }
    let out = command.output().map_err(|e| format!("could not run cargo: {e}"))?;

    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if loud {
        print!("{text}");
    }
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if loud && !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    // Miri reports undefined behaviour on standard error and aborts, so the
    // verdict this function returns has to see both streams. The fuzzer's own
    // findings are on standard output; a tool's are not.
    text.push_str(&stderr);
    // The one output a caller must never read as a result: a run that printed
    // no report at all did not run. `f-sim` fails closed on the same signal.
    if !text.contains("hostile ") {
        // The one failure worth naming rather than passing through, because its
        // own message names a component and not a cause: an image built before
        // `docker/Dockerfile` grew `rustup component add miri` has a `cargo
        // miri` proxy with nothing behind it, and rustup renders that as a
        // stack backtrace.
        if miri && (stderr.contains("is not installed") || stderr.contains("no such command")) {
            return Err("Miri is not installed in this environment.\n\n\
                 `docker/Dockerfile`'s `dev` stage installs it and builds its sysroot; an\n\
                 image built before that does not have it. Rebuild:\n\n  \
                 docker compose -f docker/compose.yaml build dev\n\n\
                 It is deliberately not in `rust-toolchain.toml` — that file is the pin\n\
                 every laptop reads and bumping it invalidates every claim. RFC 0046."
                .to_string());
        }
        return Err(format!("the fuzzer printed no report, so it did not run:\n{}", stderr.trim()));
    }
    Ok((out.status.success(), text))
}

/// The `hostile` verb.
///
/// `--base <seed>` is pulled out here rather than taken positionally, for
/// `sweep --base`'s reason: it is the argument a nightly varies and the
/// positional one is the argument a person varies. Everything downstream takes
/// the base explicitly, because a report whose base was implicit is a report
/// nobody can reproduce from its own header.
fn hostile_verb(args: &[String]) -> Result<(), String> {
    let mut miri = false;
    let mut base: Option<String> = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        match arg.as_str() {
            "--miri" => miri = true,
            "--base" => {
                let value =
                    walk.next().ok_or("--base needs a seed: 0x-prefixed hex, or decimal")?;
                base = Some(value.clone());
            }
            other => rest.push(other),
        }
    }
    let base = base.as_deref().unwrap_or(TRACE_SEED);

    match rest.first().copied() {
        Some("--mutate") if miri => hostile_miri_mutate(),
        Some("--mutate") => hostile_mutate(),
        Some("--corpus") => hostile_corpus(miri),
        Some("--record") => hostile_record(base),
        // `E1-P04`'s own number, by name rather than as a literal. A workflow
        // file that spelled `1000000000` would be a second copy of the exit
        // criterion, and the two would drift the first time one of them moved.
        Some("--exit") => hostile(HOSTILE_EXIT, miri, base, true),
        Some(other) if other.starts_with('-') => {
            Err(format!("unknown option for hostile: {other}"))
        }
        count => {
            let default = if miri { HOSTILE_MIRI } else { HOSTILE_GATE };
            let ops = match count {
                None => default,
                Some(text) => text
                    .parse()
                    .map_err(|_| format!("hostile takes an operation count, not `{text}`"))?,
            };
            if ops == 0 {
                return Err("hostile 0 asks for a run with no operations in it, which is a \
                            result that is green because it asserted nothing. R04."
                    .to_string());
            }
            hostile(ops, miri, base, false)
        }
    }
}

/// The gate: [`HOSTILE_GATE`] operations, in `verify` and in CI.
///
/// A named function rather than a call with a constant in it, because `verify`
/// should read as a list of claims and not as a list of numbers — and because
/// `claims/0008`'s route dispatches here too.
fn hostile_gate() -> Result<(), String> {
    hostile(HOSTILE_GATE, false, TRACE_SEED, false)
}

/// One run of the fuzzer, with the wall clock around it.
///
/// The clock is here and not in the binary, for `sweep`'s reason: a cost that
/// could reach a verdict would make two machines disagree about what a commit
/// does. What it buys is the number `claims/0008` reports — a run nobody can
/// afford is a run nobody performs.
fn hostile(ops: u64, miri: bool, base: &str, exit: bool) -> Result<(), String> {
    let ops_text = ops.to_string();
    let started = std::time::Instant::now();
    let (clean, text) = hostile_run(&[], &["--seed", base, "--ops", &ops_text], miri)?;
    let elapsed = started.elapsed();

    let seconds = elapsed.as_secs_f64();
    #[allow(clippy::cast_precision_loss, clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let rate = if seconds > 0.0 { (ops as f64 / seconds) as u64 } else { 0 };
    println!(
        "\nelapsed    {seconds:.1} s of wall clock under {}, about {rate} operation(s) a\n\
         \x20          second, and it is in no verdict above. Two machines that disagree\n\
         \x20          about this number still agree about every line of the report.",
        if miri { "Miri" } else { "the ordinary build" }
    );

    if !clean {
        return Err(hostile_failure(&text));
    }

    // The registry, before any verdict about the run. A run judged against a
    // list that has drifted from the claim is the failure one level down from
    // the one this whole file is about.
    hostile_thresholds_match()?;

    // The reach requirement is the ordinary run's and not Miri's, and the reason
    // is structural rather than empirical. **Reach is a property of the
    // generator**, and it is asserted where asserting it at scale is affordable:
    // the ordinary run drives a hundred million operations or more and can be
    // asked whether every path moved. The Miri run asserts one property, at a
    // count six orders of magnitude smaller, and loading a coverage requirement
    // onto it would make the *unsafety* check fail for a coverage reason — which
    // is the one thing a per-property split exists to prevent.
    //
    // The earlier note here said 4 096 operations "cannot drive twenty-six
    // paths". That was an empirical claim and it is false: a Miri run at this
    // count reaches all twenty-six. A reason that is measurably wrong is worse
    // than none, so it is the structural argument that stands.
    if miri {
        hostile_miri_count(ops)?;
        println!(
            "hostile: clean under Miri, over {ops} operation(s). Reach belongs to the\n\
             \x20        ordinary run: it is a property of the generator, asserted where a\n\
             \x20        large sample is affordable, and this run asserts one property at a\n\
             \x20        count six orders of magnitude smaller. RFC 0046."
        );
        return Ok(());
    }
    hostile_counts(ops, exit)?;
    hostile_reached(&text, ops)
}

/// The Miri run against the count `claims/0008` states for it.
///
/// A separate function from [`hostile_counts`] because it is a separate claim:
/// this is the count at which the *unsafety* property is checked, and the whole
/// of RFC 0046's first decision is that it is written down beside the other two
/// rather than folded into them.
fn hostile_miri_count(ops: u64) -> Result<(), String> {
    let rows = hostile_thresholds()?;
    let stated = rows.get("miri_operations").and_then(|b| b.min).unwrap_or(0);
    if ops < stated {
        return Err(format!(
            "this Miri run performed {ops} operation(s) and {HOSTILE_CLAIM} states\n\
             `miri_operations = {{ min = {stated} }}`.\n\n\
             A shorter run is a smaller sample of a property that is already checked at\n\
             a fraction of the exit's count, and lowering it silently is how the gap\n\
             `unsafety_gap` measures grows without anybody deciding to grow it."
        ));
    }
    Ok(())
}

/// The two counts `claims/0008` states, checked against the run that just
/// happened, and both named whichever one this run was.
///
/// # Why this exists
///
/// Because the registry has to carry the number it publishes. `operations` is
/// the gate and `exit_operations` is `E1-P04`'s own billion; before this, only
/// the first was a threshold and the second lived in a CI job and in prose,
/// which meant the claim's own reproduction command reproduced a tenth of the
/// exit and said nothing about the rest.
fn hostile_counts(ops: u64, exit: bool) -> Result<(), String> {
    let rows = hostile_thresholds()?;
    let min = |key: &str| rows.get(key).and_then(|b| b.min).unwrap_or(0);
    let gate = min("operations");
    let want = min("exit_operations");

    if exit && ops < want {
        return Err(format!(
            "`--exit` performed {ops} operation(s) and {HOSTILE_CLAIM} states\n\
             `exit_operations = {{ min = {want} }}`, which is E1-P04's own number.\n\n\
             The exit sentence is a conjunction over one billion operations. A run that\n\
             claims it and performs fewer is the shape this registry exists to refuse."
        ));
    }

    let stood = if ops >= want {
        "this run performed the exit's own count"
    } else if ops >= gate {
        "this run performed the gate's count"
    } else {
        "this run is below the gate and is neither"
    };
    println!(
        "registry   {stood}: {ops} operation(s) against `operations >= {gate}` and\n\
         \x20          `exit_operations >= {want}` in {HOSTILE_CLAIM}. The published\n\
         \x20          reproduction — `cargo xtask claim hostile-peer-operations` — runs the\n\
         \x20          gate; `cargo xtask hostile --exit` runs the exit's own number, and\n\
         \x20          the claim's [reproduce] names both."
    );
    Ok(())
}

/// What a finding looks like when it is reported to a person.
fn hostile_failure(text: &str) -> String {
    let finding = text
        .lines()
        .find(|line| line.starts_with("finding 1  "))
        .unwrap_or("finding 1  (the report's shape moved)");
    format!(
        "the fuzzer found something.\n\n  {finding}\n\n\
         The `repro` line above stands alone: an episode is derived from (seed, index)\n\
         by identity, so it reproduces in a millisecond rather than in the whole run.\n\
         RFC 0046.\n\n\
         If there is no finding line at all, the process died without reporting — which\n\
         is what memory unsafety looks like without a tool, and is `--miri`'s job."
    )
}

/// Every counter the report is required to have moved.
///
/// # Why a clean run is not enough
///
/// Because a fuzzer that reached nothing prints the same two words as one that
/// reached everything. These are the paths a hostile peer has to have driven
/// for *no panic, no hang* to be a statement about the ring rather than about a
/// region that spent the whole run refused — and four of them were zero at some
/// point while this was being written, which is the argument for having the
/// list at all. `claims/0008` carries the same rows as thresholds.
const HOSTILE_REACHED: &[(&str, &str)] = &[
    ("header bytes", "peer_header_bytes"),
    ("header fields", "peer_header_fields"),
    ("cursors", "peer_cursors"),
    ("index slots", "peer_index_slots"),
    ("entry slots", "peer_entry_slots"),
    ("arena bytes", "peer_arena_bytes"),
    ("flag words", "peer_flag_words"),
    ("restarts", "peer_restarts"),
    ("restarts mid-batch", "peer_restarts_mid_batch"),
    ("adopted", "channel_adopted"),
    ("refused malformed", "channel_refused_malformed"),
    ("refused address", "channel_refused_address"),
    ("refused version", "channel_refused_version"),
    ("refused feature", "channel_refused_feature"),
    ("epoch changes seen", "channel_epoch_changes_seen"),
    ("submitted", "ring_submitted"),
    ("ring full", "ring_full"),
    ("corrupt, reported", "ring_corrupt_reported"),
    ("popped", "ring_popped"),
    ("reaped", "ring_reaped"),
    ("executed", "entries_executed"),
    ("refused reserved", "entries_refused_reserved"),
    ("refused flag", "entries_refused_flag"),
    ("refused opcode", "entries_refused_opcode"),
    ("refused bad address", "entries_refused_bad_address"),
    ("arena bytes copied", "entries_arena_bytes_copied"),
];

/// The rows of `claims/0008`'s `[threshold]` table that are *not* reach counts.
///
/// Named as an exclusion rather than deriving the reach rows by a prefix,
/// because a prefix is a convention and this is a short list. They are the three
/// counts, the gap between two of them, and the three properties; everything
/// else in that table is a minimum on a path, and a new one added there without
/// a line in [`HOSTILE_REACHED`] is a threshold nothing reads.
const HOSTILE_NOT_REACH: &[&str] = &[
    "operations",
    "exit_operations",
    "miri_operations",
    "unsafety_gap",
    "panics",
    "stuck",
    "miri_undefined_behaviour",
];

/// One row of `claims/0008`'s `[threshold]` table.
#[derive(Clone, Copy, Default)]
struct Bound {
    /// The `min`, where the row states one.
    min: Option<u64>,
    /// The `max`, where the row states one.
    max: Option<u64>,
}

/// `claims/0008`'s `[threshold]` table, read.
///
/// # Why it is read rather than restated
///
/// Because a minimum that lives in `xtask` and a minimum that lives in the claim
/// are two copies of one number, and the copy nobody reads is the one that rots.
/// `hostile_thresholds_match` makes that argument about the *key set*; this makes
/// it about the values, so `cargo xtask hostile` enforces exactly what the
/// registry publishes and a reader can change a threshold by editing the claim.
fn hostile_thresholds() -> Result<std::collections::BTreeMap<String, Bound>, String> {
    let path = root().join(HOSTILE_CLAIM);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;

    let value = |rest: &str, which: &str| -> Option<u64> {
        let (_, after) = rest.split_once(which)?;
        after
            .trim_start()
            .strip_prefix('=')?
            .split_whitespace()
            .next()?
            .trim_end_matches([',', '}'])
            .parse()
            .ok()
    };

    let mut rows = std::collections::BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('[') {
            inside = trimmed.trim_end().trim_end_matches('\r') == "[threshold]";
            continue;
        }
        if !inside || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once('=') else { continue };
        let key = key.trim();
        if key.is_empty() {
            continue;
        }
        rows.insert(key.to_string(), Bound { min: value(rest, "min"), max: value(rest, "max") });
    }
    Ok(rows)
}

/// Require the twenty-six reach rows in `claims/0008` and the twenty-six pairs
/// in [`HOSTILE_REACHED`] to be the same set.
///
/// # Why this exists
///
/// Because they are one list written twice — the report prints prose names and
/// the claim carries keys — and two copies of a list drift silently. Deleting a
/// row from `HOSTILE_REACHED` costs nothing and turns a published minimum into a
/// threshold nobody checks; deleting one from the claim leaves `xtask` enforcing
/// a number the registry no longer states. Either is exactly the shape this
/// epoch has already shipped three times: a check that is green while the thing
/// it stands for is not there.
///
/// It runs on every ordinary `hostile` run rather than as a lint, because the
/// verdict it protects is that run's, and a check that lives somewhere else is a
/// check somebody can pass without.
fn hostile_thresholds_match() -> Result<(), String> {
    let rows = hostile_thresholds()?;

    // The counts first, because they are the ones a reader would otherwise have
    // to take on trust: three constants in this file and three thresholds in the
    // registry, required to be the same three numbers.
    for (key, want, what) in [
        ("operations", HOSTILE_GATE, "the gate"),
        ("exit_operations", HOSTILE_EXIT, "E1-P04's own number"),
        ("miri_operations", HOSTILE_MIRI, "the count the unsafety property is checked at"),
    ] {
        match rows.get(key).and_then(|b| b.min) {
            Some(stated) if stated == want => {}
            Some(stated) => {
                return Err(format!(
                    "{HOSTILE_CLAIM} states `{key} = {{ min = {stated} }}` and this build \
                     runs {want}.\n\n\
                     {key} is {what}, and a registry number that is not the number the \
                     command performs is a published claim nobody is checking. Move both."
                ));
            }
            None => {
                return Err(format!(
                    "{HOSTILE_CLAIM} has no `{key}` in [threshold], and {key} is {what}.\n\n\
                     A count that lives only in prose is a count that quietly stops being \
                     the one that runs. RFC 0046."
                ));
            }
        }
    }

    // And the distance between the two — the exit's one conjunct no tool can
    // check at the exit's own scale, as a number the registry carries rather
    // than as a paragraph somebody has to find.
    let gap = HOSTILE_EXIT / HOSTILE_MIRI;
    match rows.get("unsafety_gap").and_then(|b| b.max) {
        Some(stated) if stated == gap => {}
        Some(stated) => {
            return Err(format!(
                "{HOSTILE_CLAIM} states `unsafety_gap = {{ max = {stated} }}` and this build's \
                 constants give {gap}.\n\n\
                 It is `exit_operations / miri_operations`: how many times larger the sample \
                 the panic and hang properties are checked over is than the sample the memory \
                 unsafety property is checked over. Raising the exit's count without raising \
                 Miri's widens it, which is a decision and belongs in a diff. RFC 0046."
            ));
        }
        None => {
            return Err(format!(
                "{HOSTILE_CLAIM} has no `unsafety_gap` in [threshold].\n\n\
                 The exit is a conjunction over a billion operations and one of its three \
                 conjuncts is checked over {HOSTILE_MIRI}. That shortfall is declared as a \
                 quantity, the way JOIN_GAP and CHAOS_GAP are, rather than as a sentence \
                 beside a green result. RFC 0046."
            ));
        }
    }

    let claimed: std::collections::BTreeSet<String> =
        rows.keys().filter(|key| !HOSTILE_NOT_REACH.contains(&key.as_str())).cloned().collect();

    let listed: std::collections::BTreeSet<String> =
        HOSTILE_REACHED.iter().map(|(_, key)| (*key).to_string()).collect();

    let only_claim: Vec<&String> = claimed.difference(&listed).collect();
    let only_xtask: Vec<&String> = listed.difference(&claimed).collect();
    if only_claim.is_empty() && only_xtask.is_empty() {
        return Ok(());
    }

    let say = |what: &str, rows: &[&String]| {
        if rows.is_empty() {
            String::new()
        } else {
            format!(
                "\n  {what}:\n    {}",
                rows.iter().map(|r| r.as_str()).collect::<Vec<_>>().join("\n    ")
            )
        }
    };
    Err(format!(
        "claims/0008's reach thresholds and HOSTILE_REACHED have drifted.{}{}\n\n\
         They are one list written twice, and the copy nobody reads is the one that\n\
         rots: a row only the claim has is a published minimum this run does not check,\n\
         and a row only xtask has is a requirement the registry does not state. Fix\n\
         whichever side is wrong — and if a path really has gone, say so in the claim\n\
         rather than deleting the row from here. RFC 0046.",
        say("in claims/0008 and not in HOSTILE_REACHED", &only_claim),
        say("in HOSTILE_REACHED and not in claims/0008", &only_xtask),
    ))
}

/// One counter out of a report, by the name the report prints it under.
fn hostile_counter(text: &str, name: &str) -> Option<u64> {
    text.lines()
        .map(str::trim)
        .find(|line| line.starts_with(name))
        .and_then(|line| line.split_whitespace().next_back())
        .and_then(|value| value.parse().ok())
}

/// Require every path in [`HOSTILE_REACHED`] to have been reached as often as
/// `claims/0008` says it must be.
///
/// # Why the minimums are not all one
///
/// Because a minimum of one catches a path that has *stopped*, and nothing else.
/// The observed values at the gate's count span 60 068 to 298 507 655, so a
/// regression that collapsed a path by five orders of magnitude — a generator
/// drawing an unknown opcode once a run instead of sixty-seven thousand times —
/// would pass a `min = 1` unchanged while the claim's prose says those rows exist
/// so that exactly that fails. Reach is the only thing standing between a clean
/// billion and a vacuous one, so the bound has to be able to bind.
///
/// # Why they scale
///
/// Each row in the claim is stated **per [`HOSTILE_GATE`] operations**, and is
/// scaled here to the run that actually happened, floored at one. So the gate
/// enforces the stated number, the exit's billion enforces ten times it, and a
/// short diagnostic run still enforces *reached at all* rather than being held to
/// a number it cannot make. The numbers themselves are about two orders of
/// magnitude below what a healthy run produces: a floor that fires on drift, not
/// one that fires on noise.
fn hostile_reached(text: &str, ops: u64) -> Result<(), String> {
    let rows = hostile_thresholds()?;

    // Integer arithmetic, in `u128` so that a large claim and a large run cannot
    // multiply past `u64` — the numbers today are nowhere near it, and a bound
    // that overflowed would be a bound that silently became small.
    let scaled = |stated: u64| -> u64 {
        let want = u128::from(stated) * u128::from(ops) / u128::from(HOSTILE_GATE);
        u64::try_from(want).unwrap_or(u64::MAX).max(1)
    };

    let mut missing = Vec::new();
    for (name, key) in HOSTILE_REACHED {
        let need = scaled(rows.get(*key).and_then(|b| b.min).unwrap_or(1));
        match hostile_counter(text, name) {
            Some(count) if count >= need => {}
            Some(count) => missing
                .push(format!("{name} reached {count}, and {key} scaled to this run is {need}")),
            None => missing.push(format!("{name} (no such line in the report)")),
        }
    }
    if missing.is_empty() {
        println!(
            "hostile: clean, and every one of the {} paths the claim names was reached at\n\
             \x20        or above the minimum {HOSTILE_CLAIM} states for it.",
            HOSTILE_REACHED.len()
        );
        return Ok(());
    }
    Err(format!(
        "the run was clean and {} path(s) fell below the claim's minimum:\n  {}\n\n\
         A fuzzer that reached nothing reports exactly what one that reached everything\n\
         reports, and this epoch has already shipped three tests that were green while\n\
         the property they stood for did not hold. Either the generator stopped\n\
         producing that input, or the code stopped having that path. Both are findings.\n\
         RFC 0046; every minimum above is read from {HOSTILE_CLAIM} and stated there per\n\
         {HOSTILE_GATE} operations.",
        missing.len(),
        missing.join("\n  ")
    ))
}

/// The mutation harness: arm each defect, require the property it breaks to be
/// reported, disarm, require quiet.
///
/// Two of the three are here. The third is `--miri --mutate`, and the split is
/// the whole of what this harness demonstrates: a memory-unsafety defect is
/// **invisible to this half**. It does not produce a finding, it produces a
/// dead process with no seed attached — which is exactly why the third property
/// is checked by a tool and at a different count.
fn hostile_mutate() -> Result<(), String> {
    // The panic defect first, and everything that needs it armed before the
    // features change: switching a feature set rebuilds the crate, so the order
    // of the steps below is also the order that builds `f-ring` three times
    // rather than five.
    // Each visible defect gets three steps under one feature set, in this order
    // because switching a feature set rebuilds the crate: the defect is found by
    // the property it breaks, the control says how much a corpus entry recorded
    // under it is worth, and the corpus itself is required to go red.
    for (defect, signature, property) in
        [(HOSTILE_DEFECT_PANIC, "panic ", "no panic"), (HOSTILE_DEFECT_STUCK, "stuck ", "no hang")]
    {
        hostile_defect_found(defect, signature, property)?;
        hostile_selective(defect)?;
        hostile_corpus_red(defect)?;
    }

    let ops = HOSTILE_MUTATE_OPS.to_string();
    println!("\n--- {HOSTILE_DEFECT_UNSAFE}: this half must NOT be able to see it\n");
    match hostile_run(&[HOSTILE_DEFECT_UNSAFE], &["--ops", &ops], false) {
        // The ordinary build dies on the wild read, and it dies as a signal
        // rather than as a finding: no seed, no episode, nothing to paste. That
        // is the observation this step exists to make, and it is why the third
        // property has its own tool.
        Err(_) => println!("  the process died without reporting, as it must"),
        Ok((false, text)) if !text.contains("finding 1  ") => {
            println!("  the run went red without reporting a finding, as it must");
        }
        Ok((false, _)) => {
            return Err(format!(
                "`{HOSTILE_DEFECT_UNSAFE}` was reported as a finding by the ordinary build.\n\n\
                 That is not a failure of the ring — it means the defect has become\n\
                 visible to a check that is supposed to be blind to it, so the Miri half\n\
                 is no longer proving anything the cheap half does not. Re-read RFC\n\
                 0046's argument for three defects before changing this."
            ));
        }
        Ok((true, _)) => {
            return Err(format!(
                "`{HOSTILE_DEFECT_UNSAFE}` left the ordinary build clean and alive.\n\n\
                 Reading past the end of the entry array is undefined behaviour whether\n\
                 or not this machine noticed, so a green run here says only that the\n\
                 bytes happened to be mapped — which is the entire argument for\n\
                 `cargo xtask hostile --miri --mutate`, and that command is what has to\n\
                 pass. This step is a note about what the cheap half cannot do."
            ));
        }
    }

    println!("\n--- disarmed: the fuzzer must go quiet\n");
    let (clean, text) = hostile_run(&[], &["--ops", &ops], false)?;
    if !clean {
        return Err(format!(
            "the fuzzer found something with no defect armed:\n{}",
            hostile_failure(&text)
        ));
    }

    // And the other half of the corpus pair. A file that goes red armed and red
    // disarmed says nothing about the defect, which is the control every
    // mutation harness in this tree carries beside its red half.
    println!("\n--- disarmed: the corpus must go green\n");
    hostile_corpus(false)?;

    println!(
        "\nhostile --mutate: the two properties this half can check both fail on demand,\n\
         the corpus goes red on each of the two defects and green with neither, its\n\
         entries are worth what HOSTILE_SELECTIVITY says they are worth, and the third\n\
         defect is shown to be invisible here. `cargo xtask hostile --miri --mutate` is\n\
         the half that can see it."
    );
    Ok(())
}

/// Measure how much a corpus entry recorded under `defect` is worth, and require
/// the answer to be the one [`HOSTILE_SELECTIVITY`] declares.
///
/// Five episodes of the tree's own seed that are *not* corpus entries, replayed
/// with the defect armed: what comes back is how many arbitrary runs find what a
/// recorded run found. It is the one figure a corpus cannot carry by
/// construction, and without it a reader has no way to tell a rare seed from the
/// first one the recorder tried.
fn hostile_selective(defect: &str) -> Result<(), String> {
    let control = HOSTILE_CONTROL.len();
    let Some(declared) = hostile_selectivity(defect) else {
        return Err(format!(
            "`{defect}` has no row in HOSTILE_SELECTIVITY in xtask/src/main.rs.\n\n\
             A defect the corpus records entries under and nothing measures the\n\
             selectivity of is a defect whose entries could be worth nothing without\n\
             anybody finding out. RFC 0046, *The corpus*."
        ));
    };

    println!("\n--- {defect}: how much a corpus entry recorded under it is worth\n");
    let hits = hostile_control_hits(defect)?;
    if hits != declared {
        return Err(format!(
            "{hits} of {control} control episode(s) reproduce `{defect}`, and\n\
             HOSTILE_SELECTIVITY in xtask/src/main.rs says {declared}.\n\n\
             Both directions are information and neither is a failure of the ring. Fewer\n\
             means the defect has become harder to reach and the entries in\n\
             {HOSTILE_CORPUS} recorded under it have started carrying more than an\n\
             arbitrary episode does; more means the opposite. Either way the number moves\n\
             in a diff, `--record` rewrites the `# also` lines, and the corpus header\n\
             stops saying what it says today. RFC 0046, *The corpus*."
        ));
    }
    if hits == control {
        println!(
            "\n  selectivity  {hits} of {control}: every control episode reproduces it, so an\n\
            \x20              entry recorded under `{defect}` is provenance — a seed, a\n\
            \x20              commit and an evidence line that outlive the run — and not\n\
            \x20              rarity. {HOSTILE_CORPUS} says so rather than implying otherwise."
        );
    } else {
        println!(
            "\n  selectivity  {hits} of {control}: most control episodes do not reproduce it,\n\
            \x20              so an entry recorded under `{defect}` carries something an\n\
            \x20              arbitrary episode does not. This is the half of\n\
            \x20              {HOSTILE_CORPUS} that is a regression suite in the full sense."
        );
    }
    Ok(())
}

/// Arm one defect and require every corpus entry recorded under it to go red.
///
/// # Why this step exists
///
/// Because without it `cargo xtask hostile --corpus` is green for the reason an
/// empty file is green: nothing has ever shown it can be anything else.
/// `sweep_mutate` spends its `[3/5]` on exactly this and its comment is the
/// argument — *a regression suite whose entries have never been seen to fail is
/// a file of command lines nobody has tested*. The step was missing here, which
/// is the fourth instance in this epoch of a check that is green while the thing
/// it stands for is not there.
///
/// The requirement is **exact rather than statistical**: an entry's `# under`
/// line names the defect it was recorded against, and only those entries are
/// required to fail. Entries recorded under another defect are counted and no
/// more, because a panic entry going red under the drain defect says nothing
/// either way.
fn hostile_corpus_red(defect: &str) -> Result<(), String> {
    println!("\n--- the corpus must go red with `{defect}` armed\n");
    let played = hostile_corpus_replay(false, &[defect])?;
    let owned: Vec<&(CorpusEntry, bool)> =
        played.iter().filter(|(e, _)| e.under() == Some(defect)).collect();
    if owned.is_empty() {
        return Err(format!(
            "no entry in {HOSTILE_CORPUS} names `{defect}` in an `# under` line, so arming\n\
             it proves nothing about the file.\n\n\
             An entry's provenance is what makes this step exact rather than statistical.\n\
             `cargo xtask hostile --record` writes it; an entry that lost it was written by\n\
             a recorder that read only the argv, which is how the first version of this\n\
             file came to hold seven bare lines under a header claiming each said what it\n\
             was found under."
        ));
    }
    let survived: Vec<String> =
        owned.iter().filter(|(_, clean)| *clean).map(|(e, _)| e.argv.join(" ")).collect();
    if !survived.is_empty() {
        return Err(format!(
            "{} of {} corpus entr(y/ies) recorded under `{defect}` stayed clean with it\n\
             armed:\n  {}\n\n\
             The corpus is the runs that found this, kept so that they keep finding it. An\n\
             entry that no longer does is either a stale line or a generator that has\n\
             stopped producing the input — both are findings, and neither is a reason to\n\
             delete the entry.",
            survived.len(),
            owned.len(),
            survived.join("\n  ")
        ));
    }
    println!(
        "\n  the corpus catches it: all {} entr(y/ies) recorded under `{defect}` went red,\n\
        \x20                        and {} of {} entries did in total.",
        owned.len(),
        played.iter().filter(|(_, clean)| !clean).count(),
        played.len()
    );
    Ok(())
}

/// Arm one defect, require it to be found, and require the property that found
/// it to be the one it breaks.
///
/// Split out of [`hostile_mutate`] when the corpus steps went in beside it: each
/// defect now gets three steps under one feature set, and a step that needs the
/// defect armed has to sit inside that stretch rather than after it.
fn hostile_defect_found(defect: &str, signature: &str, property: &str) -> Result<(), String> {
    let ops = HOSTILE_MUTATE_OPS.to_string();
    println!("\n--- {defect}: `{property}` must be reported\n");
    let (clean, text) = hostile_run(&[defect], &["--ops", &ops], false)?;
    if clean {
        return Err(format!(
            "the fuzzer was clean with `{defect}` armed.\n\n\
             The defect is in the shipped source behind a feature that is off by\n\
             default — RFC 0017's argument, extended to this layer by RFC 0046 — and\n\
             a fuzzer that cannot find it is a fuzzer whose clean runs mean nothing.\n\
             Either {HOSTILE_MUTATE_OPS} operations no longer reach it, or the\n\
             generator stopped producing the input that does."
        ));
    }
    let Some(finding) = text.lines().find(|line| line.starts_with("finding 1  ")) else {
        return Err(format!("`{defect}` went red and printed no finding line"));
    };
    if !finding.contains(signature) {
        return Err(format!(
            "`{defect}` was found by the wrong property:\n  {finding}\n\n\
             It is supposed to break `{property}`, and one defect per property is the\n\
             whole reason there are three. A defect found by another property's check\n\
             leaves that property's check unproven. RFC 0042, RFC 0046."
        ));
    }
    if !text.contains("--episode ") {
        return Err(format!("`{defect}` was found and printed no reproduction"));
    }
    println!("\n  found by   {}", finding.trim());
    Ok(())
}

/// The Miri half of the mutation harness.
fn hostile_miri_mutate() -> Result<(), String> {
    let ops = HOSTILE_MIRI.to_string();

    println!("\n--- {HOSTILE_DEFECT_UNSAFE} under Miri: `no memory unsafety` must be reported\n");
    let (clean, text) = hostile_run(&[HOSTILE_DEFECT_UNSAFE], &["--ops", &ops], true)?;
    if clean {
        return Err(format!(
            "Miri was clean with `{HOSTILE_DEFECT_UNSAFE}` armed.\n\n\
             The defect reads past the end of the entry array through the slot number a\n\
             peer wrote, which is undefined behaviour Miri detects by construction. A\n\
             clean run here means the generator no longer produces an out-of-range slot\n\
             number, or {HOSTILE_MIRI} operations no longer reach `Consumer::pop`."
        ));
    }
    if !text.contains("Undefined Behavior") {
        return Err(format!(
            "Miri went red with `{HOSTILE_DEFECT_UNSAFE}` armed and did not report\n\
             undefined behaviour. Something else failed, and this harness asserts the\n\
             tool's own verdict rather than an exit status: read the output above."
        ));
    }
    println!("\n  Miri reported undefined behaviour, as it must");

    println!("\n--- disarmed: Miri must go quiet\n");
    let (clean, _) = hostile_run(&[], &["--ops", &ops], true)?;
    if !clean {
        return Err("Miri found something with no defect armed. That is a real finding about \
                    this tree, and its output above names the location."
            .to_string());
    }
    println!("\nhostile --miri --mutate: the unsafety check fails on demand and is quiet without.");
    Ok(())
}

/// One corpus entry: where it came from, and the run it is.
///
/// A corpus line **is** an argv, which is the whole of the format;
/// `sim/corpus.txt` states the same rule and this follows it rather than
/// inventing a second. What sits *above* a line is the entry's provenance —
/// what was found, at which commit, under which defect, with what evidence —
/// and it is carried here rather than discarded, because a recorder that reads
/// only the argv rewrites the file without it and the second `--record` run
/// silently deletes what the first one wrote. That is what happened, and it is
/// why this is a struct and not a `Vec<String>`.
struct CorpusEntry {
    /// The comment lines immediately above the argv, verbatim.
    block: Vec<String>,
    /// The entry: an argument list for the fuzzer.
    argv: Vec<String>,
}

impl CorpusEntry {
    /// The defect this entry's block names, where it names one.
    ///
    /// It is what makes `--mutate` able to say something exact rather than
    /// something statistical: an entry recorded under a defect must go red when
    /// that defect is armed, and an entry recorded under another one is not
    /// evidence either way.
    fn under(&self) -> Option<&str> {
        self.block.iter().find_map(|line| {
            let rest = line.trim_start_matches('#').trim();
            rest.strip_prefix("under").map(str::trim).filter(|d| !d.is_empty())
        })
    }
}

/// Every corpus entry, with the provenance block that belongs to it.
///
/// The only structure in the file beyond *a line is an argv*: a **blank line
/// ends a block**, so the file's own header does not become the first entry's
/// provenance and each entry owns the comments immediately above it.
fn hostile_corpus_entries() -> Result<Vec<CorpusEntry>, String> {
    let path = root().join(HOSTILE_CORPUS);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;

    let mut entries = Vec::new();
    let mut block: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            block.clear();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix('#') {
            block.push(format!("#{rest}"));
            continue;
        }
        entries.push(CorpusEntry {
            block: std::mem::take(&mut block),
            argv: trimmed.split_whitespace().map(str::to_string).collect(),
        });
    }
    Ok(entries)
}

/// Replay every corpus entry, answering what each one did.
///
/// `features` is what makes this more than a green-path replay: armed with a
/// defect, the entries recorded under that defect have to go red, and that is
/// the step that says the file can fail at all. `sweep_mutate`'s `[3/5]` is the
/// precedent and its comment is the argument — *a regression suite whose entries
/// have never been seen to fail is a file of command lines nobody has tested*.
fn hostile_corpus_replay(
    miri: bool,
    features: &[&str],
) -> Result<Vec<(CorpusEntry, bool)>, String> {
    let entries = hostile_corpus_entries()?;
    if entries.is_empty() {
        return Err(format!(
            "{HOSTILE_CORPUS} holds no entries.\n\n\
             An empty corpus is not a corpus: it is a file that passes because it asks\n\
             nothing. `cargo xtask hostile --record` is how the entries in it were\n\
             produced."
        ));
    }

    println!(
        "corpus — {} entr(y/ies) from {HOSTILE_CORPUS}{}",
        entries.len(),
        if features.is_empty() {
            String::new()
        } else {
            format!(", with `{}` armed", features.join(","))
        }
    );
    let mut played = Vec::new();
    for (i, entry) in entries.into_iter().enumerate() {
        println!("\n[{}] {}\n", i + 1, entry.argv.join(" "));
        let argv: Vec<&str> = entry.argv.iter().map(String::as_str).collect();
        let clean = matches!(hostile_run(features, &argv, miri), Ok((true, _)));
        played.push((entry, clean));
    }
    Ok(played)
}

/// The corpus, replayed. Every run that has ever found something, required to
/// be clean now.
fn hostile_corpus(miri: bool) -> Result<(), String> {
    let played = hostile_corpus_replay(miri, &[])?;
    let failed = played.iter().filter(|(_, clean)| !clean).count();

    if failed == 0 {
        println!(
            "\ncorpus: {} entr(y/ies), all clean under {}.",
            played.len(),
            if miri { "Miri" } else { "the ordinary build" }
        );
        return Ok(());
    }
    Err(format!(
        "{failed} of {} corpus entr(y/ies) that used to be clean are not.\n\n\
         Each line above is an argument list: paste it after\n\
         `cargo test -q --release -p f-ring --test hostile --` and read the report.",
        played.len()
    ))
}

/// How many of [`HOSTILE_CONTROL`] reproduce `defect`. Unit: episodes.
///
/// # What this measures, and why a corpus needs it
///
/// A corpus entry says *this run found something once*. What it does not say is
/// whether a run that did **not** find it exists — and if every episode of every
/// seed reproduces the defect, then the entry carries exactly the information an
/// arbitrary episode carries, which is none. Measuring it is the difference
/// between a regression suite and seven lines somebody happened to record first.
///
/// The answer is [`HOSTILE_SELECTIVITY`], which is a row per defect because the
/// two answer differently — five of five for the panic defect, one of five for
/// the drain — and it is written into each entry as it is recorded and checked
/// by `--mutate` on every run.
fn hostile_control_hits(defect: &str) -> Result<usize, String> {
    let mut hits = 0;
    for episode in HOSTILE_CONTROL {
        let episode = episode.to_string();
        let (clean, _) =
            hostile_run_quietly(&[defect], &["--seed", TRACE_SEED, "--episode", &episode], false)?;
        if !clean {
            hits += 1;
        }
    }
    Ok(hits)
}

/// Arm each defect that reports through the fuzzer, and merge what it finds
/// into the corpus.
///
/// This is how the entries already in `ring/corpus.txt` were produced, and it
/// is spelled rather than left as folklore: on a tree with nothing wrong with
/// it the only thing to find is a deliberate defect, so a corpus of a clean
/// tree's findings would be an empty file.
///
/// **It still fails when it finds something**, on `sweep --record`'s argument:
/// growing the corpus and going red are two things one pass has to do.
fn hostile_record(base: &str) -> Result<(), String> {
    let commit = sweep_commit()?;
    let ops = HOSTILE_MUTATE_OPS.to_string();
    let mut found = Vec::new();

    for (defect, property) in
        [(HOSTILE_DEFECT_PANIC, "no panic"), (HOSTILE_DEFECT_STUCK, "no hang")]
    {
        println!("\n--- {defect}\n");
        let (clean, text) = hostile_run(&[defect], &["--seed", base, "--ops", &ops], false)?;
        if clean {
            continue;
        }
        let Some(finding) = text.lines().find(|line| line.starts_with("finding 1  ")) else {
            continue;
        };
        let Some(repro) = text.lines().find(|line| line.trim_start().starts_with("repro ")) else {
            continue;
        };
        let Some((_, argv)) = repro.split_once(" -- ") else { continue };
        // Measured while the defect is still armed, and written into the entry:
        // how many arbitrary episodes find the same thing. It is the one figure
        // a corpus entry cannot carry by construction, and without it a reader
        // has no way to tell a rare seed from the first one the recorder tried.
        let hits = hostile_control_hits(defect)?;
        found.push((
            defect.to_string(),
            property.to_string(),
            finding.trim_start_matches("finding 1  ").trim().to_string(),
            argv.trim().to_string(),
            hits,
        ));
    }

    let path = root().join(HOSTILE_CORPUS);
    let existing = hostile_corpus_entries().unwrap_or_default();
    let mut text = hostile_corpus_header();
    let mut known: Vec<String> = existing.iter().map(|e| e.argv.join(" ")).collect();

    // Every existing entry, **with its block**. Reading only the argv here is
    // what deleted the provenance of every earlier entry on the second
    // `--record` run: the file that shipped was seven bare lines under a header
    // claiming each one said what it was found under.
    for entry in &existing {
        text.push('\n');
        for line in &entry.block {
            text.push_str(line);
            text.push('\n');
        }
        text.push_str(&entry.argv.join(" "));
        text.push('\n');
    }
    let mut added = 0;
    for (defect, property, evidence, argv, hits) in found {
        if known.contains(&argv) {
            continue;
        }
        known.push(argv.clone());
        added += 1;
        let control = HOSTILE_CONTROL.len();
        let also = if hits == control {
            format!(
                "{hits} of {control} control episodes reproduce it too, so this entry is \
                 provenance and not rarity"
            )
        } else {
            format!(
                "only {hits} of {control} control episodes reproduce it, so this entry \
                 reaches something an arbitrary episode does not"
            )
        };
        text.push_str(&format!(
            "\n# ----------------------------------------------------------------------\n\
             # broke     {property}\n\
             # commit    {commit}\n\
             # under     {defect}\n\
             # evidence  {evidence}\n\
             # also      {also}\n\
             {argv}\n"
        ));
    }
    std::fs::write(&path, text).map_err(|e| format!("writing {HOSTILE_CORPUS}: {e}"))?;
    println!("\ncorpus     {added} entr(y/ies) added to {HOSTILE_CORPUS}");

    if added == 0 {
        return Ok(());
    }
    Err(format!(
        "{added} entr(y/ies) were recorded, which means the run found something.\n\n\
         That is the ordinary outcome of `--record` and it is still a failure: a pass\n\
         that both grew the corpus and reported clean would be a pass nobody could\n\
         read. Disarm the defects, and the corpus replay is what keeps them."
    ))
}

/// The header `ring/corpus.txt` is regenerated with.
///
/// Kept beside the writer rather than in the file, so a corpus regenerated by a
/// later commit carries that commit's explanation rather than the first one's.
/// `sim/src/main.rs` does the same for the seed corpus and this follows it.
fn hostile_corpus_header() -> String {
    let control = HOSTILE_CONTROL.len();
    let panic_defect = HOSTILE_DEFECT_PANIC;
    let stuck_defect = HOSTILE_DEFECT_STUCK;
    let panic_hits = hostile_selectivity(HOSTILE_DEFECT_PANIC).unwrap_or(0);
    let stuck_hits = hostile_selectivity(HOSTILE_DEFECT_STUCK).unwrap_or(0);
    format!(
        "# The hostile-peer corpus.\n\
         #\n\
         # Every line below that is not a comment is an argument list for the fuzzer,\n\
         # and every one of them is a run that found something once. `cargo xtask\n\
         # hostile --corpus` replays all of them and requires each to be clean now.\n\
         # `cargo xtask hostile --miri --corpus` replays the same entries under Miri.\n\
         #\n\
         # There is no format here beyond *a line is an argv*, and one rule about the\n\
         # comments: **a blank line ends a block**, so the comments immediately above a\n\
         # line belong to that entry and this header belongs to none. The fuzzer's own\n\
         # command-line parser reads an entry, so an entry this binary cannot run is an\n\
         # entry that fails to load. `sim/corpus.txt` is the file this shape comes from,\n\
         # and following it rather than inventing a second one was the point.\n\
         #\n\
         # Append-only. An entry removed because somebody believed the bug was gone is\n\
         # the entry that would have caught it coming back. `--record` carries every\n\
         # existing block through verbatim; a recorder that read only the argv would\n\
         # delete the provenance of everything an earlier run wrote, and did.\n\
         #\n\
         # ---- what an entry is worth, stated rather than implied ----\n\
         #\n\
         # Each block says what was found, at which commit and under which of the\n\
         # deliberate defects in `ring/Cargo.toml`. The `# also` line says the thing a\n\
         # corpus cannot say by construction: how many of {control} *control* episodes —\n\
         # episodes deliberately not in this file — reproduce the same finding with the\n\
         # same defect armed. Without it there is no way to tell a rare seed from the\n\
         # first one the recorder happened to try.\n\
         #\n\
         # **The two answers differ, and that is the useful part.**\n\
         #\n\
         #   {panic_defect}\n\
         #     {panic_hits} of {control}. Every episode reaches it, so an entry recorded\n\
         #     under it is *provenance and not rarity*: a seed, a commit and an evidence\n\
         #     line that outlive the run, and nothing an arbitrary episode does not have.\n\
         #\n\
         #   {stuck_defect}\n\
         #     {stuck_hits} of {control}. Most episodes do **not** reach it, so an entry\n\
         #     recorded under it carries something an arbitrary episode does not. This is\n\
         #     the half of the file that is a regression suite in the full sense.\n\
         #\n\
         # `HOSTILE_SELECTIVITY` in `xtask/src/main.rs` holds those two numbers and\n\
         # `cargo xtask hostile --mutate` checks each for equality on every run — in both\n\
         # directions, because a defect that became easier to reach and one that became\n\
         # harder are each a fact about the generator.\n\
         #\n\
         # And the file is shown to be able to go red rather than assumed to be: for each\n\
         # defect `--mutate` arms it and requires every entry whose `# under` names it to\n\
         # fail, then disarms and requires every entry to pass. `sweep_mutate`'s [3/5]\n\
         # makes the same argument about `sim/corpus.txt`, and this file shipped once\n\
         # without the step.\n\
         #\n\
         # The third defect, {HOSTILE_DEFECT_UNSAFE}, has no entry and cannot have one.\n\
         # It is memory unsafety, so the ordinary build dies on a signal with no seed\n\
         # attached and Miri aborts before a reproduction can be printed. What checks it\n\
         # is `cargo xtask hostile --miri --mutate`, and RFC 0046 is why that is a\n\
         # separate command with a separate count.\n\
         #\n\
         # Reproduce one by hand:\n\
         #   cargo test -q --release -p f-ring --test hostile -- <the line>\n"
    )
}

// ---------------------------------------------------------------------------
// E1-P05 — the structure-aware entry fuzzer, its coverage measurement, its
// corpus and the harness that says a clean run means something.
//
// The division of labour is `hostile`'s, restated because it is what makes a
// number out of here worth anything:
//
//   the fuzzer  draws a case from a seed, answers it, checks three oracles,
//               counts every family and every refusal it reached, and reports a
//               finding as a *corpus line* rather than as a backtrace. It reads
//               no clock and it knows nothing about llvm-cov.
//   xtask       supplies the count, the instrumentation, and the verdict about
//               which lines of which functions the run reached. RFC 0048.
// ---------------------------------------------------------------------------

/// Cases `cargo xtask entries` runs when it is not told a count. Unit: cases.
///
/// A quarter of a million. It is the gate, and
/// `claims/0009-entry-validation-coverage.toml` carries it as `cases` so the
/// number in this file and the number in the registry cannot be two numbers.
const ENTRIES_GATE: u64 = 262_144;

/// Cases `--record` draws before it minimises. Unit: cases.
///
/// Four times the gate, because the recording run is the only one whose *point*
/// is to find inputs rather than to check them: a corpus is a cover, and a cover
/// found by a shorter search is a cover with holes in it. It costs about an
/// order of magnitude more per case than the gate — resetting and reading the
/// profile runtime's counter array once per case is what per-input coverage
/// costs — which is why this is a command somebody runs and not a step in
/// `verify`.
const ENTRIES_RECORD_CASES: u64 = 1_048_576;

/// Cases the mutation harness runs against one armed defect. Unit: cases.
///
/// Sixty-five thousand five hundred and thirty-six, and it is larger than it
/// looks it needs to be because one of the three defects lives behind
/// `World::Spent`, which is drawn once in two hundred and fifty-six cases. The
/// harness fails loudly if a defect stops being reached inside this count,
/// which is the reversal condition rather than a comment.
const ENTRIES_MUTATE_CASES: u64 = 65_536;

/// The defect that breaks the *envelope* oracle: `execute` masks an unknown
/// flag off instead of refusing the entry.
const ENTRIES_DEFECT_ENVELOPE: &str = "mutate-ignored-flag";

/// The defect that breaks the *ledger* oracle: `Table::register` fills a slot
/// whose generations have run out, so it reissues a live name.
const ENTRIES_DEFECT_LEDGER: &str = "mutate-reusable-slot";

/// The defect that breaks the *reach* oracle: `Table::resolve` masks a buffer
/// index instead of checking it.
const ENTRIES_DEFECT_REACH: &str = "mutate-lenient-index";

/// Each defect, the words of the oracle that must be the one to find it, and
/// what that oracle is called.
///
/// Required to be found by **its own** oracle and by no other, for RFC 0042's
/// arithmetic: three oracles with one defect between them is one oracle under
/// test and two decorations, and a defect found by the wrong oracle is a
/// harness whose three properties are one property wearing three names.
const ENTRIES_DEFECTS: &[(&str, &str, &str)] = &[
    (ENTRIES_DEFECT_ENVELOPE, "the code R04 names", "the envelope"),
    (ENTRIES_DEFECT_LEDGER, "never reissued", "the ledger"),
    (ENTRIES_DEFECT_REACH, "inside its own set", "the reach"),
];

/// The feature that compiles the per-case coverage signal in.
///
/// Not a defect: it is the FFI into the profile runtime's counters, and those
/// symbols exist only in a build carrying `-Cinstrument-coverage`. A feature
/// rather than a `cfg` so that referring to them without the flag is a link
/// error rather than a silently absent signal.
const ENTRIES_FEEDBACK: &str = "coverage-feedback";

/// Where the entry corpus lives, relative to the workspace root.
const ENTRIES_CORPUS: &str = "ring/entries-corpus.txt";

/// Where `claims/0009` lives.
const ENTRIES_CLAIM: &str = "claims/0009-entry-validation-coverage.toml";

/// The case the recorder runs *before* the corpus when it minimises it.
///
/// A process's first case lights every region the harness itself needs to start
/// — the argument parser, the allocator's first call — and an entry credited
/// with those is an entry whose `adds` figure says nothing about the validation
/// path. One warm-up case absorbs them, and it is not in the file.
const ENTRIES_WARMUP: [&str; 4] = ["--world", "frame", "--sqe", "op=0x0"];

/// Cases the corpus always carries, whatever the feedback keeps.
///
/// # Why a coverage-fed corpus needs hand-written seeds at all
///
/// Because the feedback signal is **blind to some of the path**, and finding
/// that out is worth more than the number it cost. LLVM gives a region a
/// physical counter only when it needs one: a region whose execution count can
/// be *derived* — an else-arm whose count is the parent's minus its sibling's —
/// carries an expression rather than a counter, and there is nothing in
/// `__llvm_prf_cnts` for the harness to read. So an input that is the first to
/// reach such a region adds no bit, is not kept, and never reaches the corpus.
///
/// It is not a hypothetical, and it is not two regions either. `Name::read`'s
/// `AUTHORITY/NO_SUCH_CAP` arm, `Request::read`'s, and **both** arms of the
/// short-write test in `f_ring::write_serial` are derived regions. The generator
/// reaches all of them thousands of times in a quarter of a million cases —
/// `arena bytes copied` runs to five figures in four thousand — and the corpus
/// the signal built covered none of them. The third was found by review rather
/// than by this file, which is the honest way to record that the first two were
/// found by a measurement and nothing then asked whether there were more.
///
/// The fourth was found by *fixing* the third, and is the sharpest statement of
/// the gap this list has. Seeding the short write covered `write_serial`'s
/// `break` and **uncovered the line below it**: the loop's go-round is the same
/// test's other arm, derived from the same parent, and the corpus entry that had
/// been reaching it was dropped by minimisation as adding nothing — because what
/// it added was invisible. Two seeds, one per arm, and the total is what moved.
/// A signal that cannot see a region cannot see it being lost either, which is
/// why the measurement is a separate step from the feedback rather than the same
/// tool trusted twice. `llvm-cov` — which computes the expressions — is what
/// says so.
///
/// # The seed this list used to carry, and why it is gone
///
/// A third seed named `Table::slot_of`'s *index past the table* refusal, and its
/// reason was that the signal could not see the region. Two measurements say
/// otherwise. Four other entries in the corpus reach it — entries the signal
/// itself kept — and removing the seed left the figure exactly where it was. A
/// seed whose reason a measurement contradicts is worse than no seed: it is a
/// comment a reader would believe, in the one file whose whole purpose is to be
/// believed about coverage.
///
/// The line in `slot_of` that really is uncovered is its *generation zero*
/// refusal, and that one is a second bound rather than a blind spot:
/// `Name::read` and `Request::read` both refuse a generation-zero id before
/// `slot_of` is asked, so no entry this harness can write reaches it. RFC 0048
/// lists it with the other second bounds instead of pretending a seed could
/// reach it.
///
/// The test that distinguishes a blind spot from a hole, since it cost a run to
/// find: replay a near-miss case and then the reaching case into one process
/// with the signal on. If the reaching case is **not kept**, what it reached is
/// invisible. The short write is not kept and so is a seed; the two `slot_of`
/// cases are kept and so are not.
///
/// Each seed names the region it exists for. They are replayed **first** when
/// `--record` minimises, and written first, so an entry that only duplicates a
/// seed is dropped rather than surviving because minimisation never saw the
/// seed. `COUNTER_GAP`, RFC 0048.
const ENTRIES_SEEDS: &[(&str, &[&str])] = &[
    (
        "Name::read refuses an id at generation zero, which no table ever issued",
        &["--world", "live", "--sqe", "op=0x0,flags=0x4,set=0x3,index=0x2"],
    ),
    (
        "Request::read refuses an unregistration naming an id nobody could have issued",
        &["--world", "live", "--sqe", "op=0xff,flags=0x4,set=0x5"],
    ),
    (
        "write_serial stops on a short write: 200 bytes asked of a 96-byte sink",
        &["--world", "frame", "--sqe", "op=0x1,len=0xc8"],
    ),
    (
        "write_serial's loop goes round instead: 34 bytes the same sink takes whole",
        &["--world", "frame", "--sqe", "op=0x1,offset=0xa5,len=0x22"],
    ),
];

/// The files the entry-validation path lives in.
///
/// Passed to `llvm-cov` as the sources to report on, absolute, because that is
/// what the coverage mapping records and what the tool matches against.
const ENTRIES_SOURCES: [&str; 5] = [
    "abi/src/buf.rs",
    "abi/src/lib.rs",
    "ring/src/lib.rs",
    "ring/src/registry.rs",
    "ring/src/buffers.rs",
];

/// One member of the entry-validation path: what it is called, and how its
/// symbol is recognised.
///
/// # Why a list of name fragments and not a demangler
///
/// Because `llvm-cov --show-functions` prints the *mangled* symbol, and v0
/// mangling writes every identifier as a length followed by the identifier —
/// `6f_ring`, `8registry`, `5Table`, `7resolve`. Matching all of a member's
/// fragments as substrings is therefore exact enough to be unambiguous and
/// short enough to read, and it needs no dependency. A member matched by no row
/// fails the run rather than being quietly counted wrong.
///
/// Where two members' fragment lists both match a row, the **longer** list wins
/// — which is what separates `f_ring::execute` from `Table::execute`.
struct PathMember {
    /// What a reader calls it.
    name: &'static str,
    /// The fragments its mangled symbol must contain, all of them.
    fragments: &'static [&'static str],
}

/// The entry-validation path, function by function.
///
/// # Why this list exists at all
///
/// Because *95% of the validation path* means nothing until somebody says which
/// lines are in it. This is that list, and `claims/0009` carries the same names
/// so the two cannot drift — `entries_path_matches_claim` checks it on every
/// run, the way `hostile_thresholds_match` does for `claims/0008`.
///
/// # What is deliberately not in it
///
/// **`kernel/src/ring.rs`.** The kernel has no host harness — `kernel/Cargo.toml`
/// says why — so no instrument in this repository can produce a coverage figure
/// for it at all. RFC 0048 declares that as `FRAME_GAP` and names what stands in
/// its place: `cargo xtask run`'s boot drives the same `f_ring::execute` and
/// both of its refusal paths on the target.
///
/// **`SetId::new`, `bits`, `index`, `generation` and `from_bits`.** They are on
/// the path — `slot_of` reads all of them — and each is a single expression, and
/// all five are fully covered. Including them would raise the figure by about
/// two and a half points and add no evidence, which is the definition of
/// padding a metric.
///
/// **`Request::write`, `Name::write` and `registration`/`unregistration`.** They
/// are how a *client* composes an entry, not how a service validates one.
const ENTRIES_PATH: &[PathMember] = &[
    // The envelope, and the two readings of an entry's buffer fields.
    PathMember { name: "SetId::is_issuable", fragments: &["5f_abi", "5SetId", "11is_issuable"] },
    PathMember {
        name: "SetId::from_completion",
        fragments: &["5f_abi", "5SetId", "15from_completion"],
    },
    PathMember { name: "Name::read", fragments: &["5f_abi", "3buf", "4Name", "4read"] },
    PathMember { name: "Request::read", fragments: &["5f_abi", "3buf", "7Request", "4read"] },
    PathMember {
        name: "opcode::is_registration",
        fragments: &["5f_abi", "6opcode", "15is_registration"],
    },
    PathMember { name: "op::known", fragments: &["5f_abi", "2op", "5known"] },
    PathMember { name: "Cqe::error", fragments: &["5f_abi", "3Cqe", "5error"] },
    PathMember { name: "error::unpack", fragments: &["5f_abi", "5error", "6unpack"] },
    // The frame's executor.
    PathMember { name: "execute", fragments: &["6f_ring", "7execute"] },
    PathMember { name: "write_serial", fragments: &["6f_ring", "12write_serial"] },
    PathMember { name: "Arena::copy_out", fragments: &["6f_ring", "5Arena", "8copy_out"] },
    // The service's registration table.
    PathMember {
        name: "Table::execute",
        fragments: &["6f_ring", "8registry", "5Table", "7execute"],
    },
    PathMember {
        name: "Table::register",
        fragments: &["6f_ring", "8registry", "5Table", "8register"],
    },
    PathMember {
        name: "Table::unregister",
        fragments: &["6f_ring", "8registry", "5Table", "10unregister"],
    },
    PathMember {
        name: "Table::retire_all",
        fragments: &["6f_ring", "8registry", "5Table", "10retire_all"],
    },
    PathMember {
        name: "Table::resolve",
        fragments: &["6f_ring", "8registry", "5Table", "7resolve"],
    },
    PathMember {
        name: "Table::release",
        fragments: &["6f_ring", "8registry", "5Table", "7release"],
    },
    PathMember {
        name: "Table::slot_of",
        fragments: &["6f_ring", "8registry", "5Table", "7slot_of"],
    },
    PathMember { name: "Table::issued", fragments: &["6f_ring", "8registry", "5Table", "6issued"] },
    PathMember { name: "retire", fragments: &["6f_ring", "8registry", "6retire"] },
    PathMember { name: "negotiated_for", fragments: &["6f_ring", "8registry", "14negotiated_for"] },
    PathMember {
        name: "Registered::bind",
        fragments: &["6f_ring", "8registry", "10Registered", "4bind"],
    },
    PathMember {
        name: "SharedVirtual::bind",
        fragments: &["6f_ring", "8registry", "13SharedVirtual", "4bind"],
    },
    PathMember {
        name: "Registered::resolve",
        fragments: &["6f_ring", "8registry", "10Registered", "9Transport", "7resolve"],
    },
    PathMember {
        name: "Registered::release",
        fragments: &["6f_ring", "8registry", "10Registered", "9Transport", "7release"],
    },
    PathMember {
        name: "SharedVirtual::resolve",
        fragments: &["6f_ring", "8registry", "13SharedVirtual", "9Transport", "7resolve"],
    },
    PathMember {
        name: "SharedVirtual::release",
        fragments: &["6f_ring", "8registry", "13SharedVirtual", "9Transport", "7release"],
    },
    // The client's side of what a service wrote.
    PathMember {
        name: "Fixed::from_completion",
        fragments: &["6f_ring", "7buffers", "5Fixed", "15from_completion"],
    },
    PathMember {
        name: "Fixed::name",
        fragments: &["6f_ring", "7buffers", "5Fixed", "6Naming", "4name"],
    },
    PathMember {
        name: "Virtual::name",
        fragments: &["6f_ring", "7buffers", "7Virtual", "6Naming", "4name"],
    },
    PathMember {
        name: "BufferSet::bind",
        fragments: &["6f_ring", "7buffers", "9BufferSet", "4bind"],
    },
    PathMember {
        name: "BufferSet::carve",
        fragments: &["6f_ring", "7buffers", "9BufferSet", "5carve"],
    },
    PathMember { name: "Idle::submit", fragments: &["6f_ring", "7buffers", "4Idle", "6submit"] },
    PathMember {
        name: "InFlight::complete",
        fragments: &["6f_ring", "7buffers", "8InFlight", "8complete"],
    },
    PathMember {
        name: "InFlight::reclaim",
        fragments: &["6f_ring", "7buffers", "8InFlight", "7reclaim"],
    },
    PathMember {
        name: "InFlight::drop",
        fragments: &["6f_ring", "7buffers", "8InFlight", "4Drop", "4drop"],
    },
    PathMember { name: "PeerGone::of", fragments: &["6f_ring", "7buffers", "8PeerGone", "2of"] },
];

/// One member's measured coverage.
struct MemberCoverage {
    /// Lines in the member's coverage mapping. Unit: lines.
    lines: u64,
    /// Lines never executed. Unit: lines.
    missed: u64,
    /// Instantiations that contributed. Unit: symbols.
    rows: usize,
    /// Placeholder records dropped from the denominator. Unit: symbols.
    ///
    /// Printed rather than kept quiet: this number decides how large the
    /// denominator is, and a change in it is the shape of a figure moving
    /// because the *measurement* moved. Every one of them is required to have
    /// executed nothing — `entries_measure` refuses the run otherwise.
    skipped: usize,
}

/// Run the fuzzer once, answering `(clean, output)`.
///
/// A non-zero exit is *a finding* rather than an error — the binary uses the
/// status that way deliberately — so this cannot use [`capture`], which treats
/// one as a failure.
fn entries_run(features: &[&str], args: &[&str], loud: bool) -> Result<(bool, String), String> {
    let mut argv: Vec<String> = ["test", "-q", "--release", "-p", "f-ring", "--test", "entries"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    if !features.is_empty() {
        argv.push("--features".into());
        argv.push(features.join(","));
    }
    argv.push("--".into());
    argv.extend(args.iter().map(|s| (*s).to_string()));

    let out = Command::new("cargo")
        .args(&argv)
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;

    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if loud {
        print!("{text}");
    }
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if loud && !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    text.push_str(&stderr);
    // The one output a caller must never read as a result: a run that printed
    // no report at all did not run.
    if !text.contains("entries \u{2014} ") {
        return Err(format!("the fuzzer printed no report, so it did not run:\n{}", stderr.trim()));
    }
    Ok((out.status.success(), text))
}

/// What a finding looks like when it is reported to a person.
fn entries_failure(text: &str) -> String {
    let finding = text
        .lines()
        .find(|line| line.starts_with("finding 1  "))
        .unwrap_or("finding 1  (the report's shape moved)");
    format!(
        "the fuzzer found something.\n\n  {finding}\n\n\
         The `repro` line above is a whole case — a world and an entry — and it stands\n\
         alone: nothing carries over between cases, so it reproduces at the cost of one\n\
         case rather than of the run. RFC 0048.\n\n\
         `wrong` is an oracle refusing the answer, and the oracle names itself in the\n\
         line. `panic` is the ring panicking on an entry, which is the property\n\
         `ring/tests/hostile.rs` owns at a much larger count."
    )
}

/// The `entries` verb.
fn entries_verb(args: &[String]) -> Result<(), String> {
    let mut base: Option<String> = None;
    let mut rest: Vec<&str> = Vec::new();
    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        match arg.as_str() {
            "--base" => {
                let value =
                    walk.next().ok_or("--base needs a seed: 0x-prefixed hex, or decimal")?;
                base = Some(value.clone());
            }
            other => rest.push(other),
        }
    }
    let base = base.as_deref().unwrap_or(TRACE_SEED);

    match rest.first().copied() {
        Some("--corpus") => entries_corpus(),
        Some("--record") => entries_record(base),
        Some("--coverage") => entries_coverage(),
        Some("--mutate") => entries_mutate(),
        Some(other) if other.starts_with('-') => {
            Err(format!("unknown option for entries: {other}"))
        }
        count => {
            let cases = match count {
                None => ENTRIES_GATE,
                Some(text) => {
                    text.parse().map_err(|_| format!("entries takes a case count, not `{text}`"))?
                }
            };
            if cases == 0 {
                return Err("entries 0 asks for a run with no cases in it, which is a result \
                            that is green because it asserted nothing. R04."
                    .to_string());
            }
            entries_draw(cases, base)
        }
    }
}

/// The gate: [`ENTRIES_GATE`] cases, in `verify` and in CI.
fn entries_gate() -> Result<(), String> {
    entries_draw(ENTRIES_GATE, TRACE_SEED)
}

/// One drawn run of the fuzzer.
fn entries_draw(cases: u64, base: &str) -> Result<(), String> {
    let cases_text = cases.to_string();
    let (clean, text) = entries_run(&[], &["--seed", base, "--ops", &cases_text], true)?;
    if !clean {
        return Err(entries_failure(&text));
    }
    entries_path_matches_claim()?;
    entries_reached(&text, cases)
}

/// Replay every case in the corpus, and require each to be clean.
fn entries_corpus() -> Result<(), String> {
    let lines = entries_corpus_lines()?;
    if lines.is_empty() {
        return Err(format!(
            "{ENTRIES_CORPUS} holds no entries.\n\n\
             An empty corpus replays clean and asserts nothing, which is the shape of\n\
             false green this whole task is arranged against. `cargo xtask entries\n\
             --record` is what fills it."
        ));
    }
    let argv: Vec<&str> = lines.iter().flat_map(|line| line.iter().map(String::as_str)).collect();
    let (clean, text) = entries_run(&[], &argv, true)?;
    if !clean {
        return Err(entries_failure(&text));
    }
    println!(
        "\nentries: {} corpus entr{} replayed clean from {ENTRIES_CORPUS}.\n\
         \x20        Each one lights a coverage region of this binary no earlier entry in\n\
         \x20        the file lights — `--record` minimises against exactly that, and\n\
         \x20        against nothing narrower: the counter array covers the harness too.\n\
         \x20        What each is worth to the *path* is `--coverage`'s per-member table.",
        lines.len(),
        if lines.len() == 1 { "y" } else { "ies" }
    );
    Ok(())
}

/// Every corpus entry, as an argv.
///
/// `sim/corpus.txt`'s shape, which `ring/corpus.txt` already follows: a line is
/// an argv and a comment is a comment. Following it rather than inventing a
/// third is the whole of the format decision.
fn entries_corpus_lines() -> Result<Vec<Vec<String>>, String> {
    let path = root().join(ENTRIES_CORPUS);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| line.split_whitespace().map(str::to_string).collect())
        .collect())
}

/// Draw cases with the coverage signal on, keep the ones that reach something
/// new, minimise the result against the corpus already in the tree, and write
/// it back.
fn entries_record(base: &str) -> Result<(), String> {
    println!(
        "[1/3] drawing with the per-case coverage signal on\n\
         \x20     `-Cinstrument-coverage` and `--features {ENTRIES_FEEDBACK}`: the profile\n\
         \x20     runtime's counters are reset before each case and read after it, which is\n\
         \x20     per-input coverage and not a summary of the run."
    );
    let cases = ENTRIES_RECORD_CASES.to_string();
    let (clean, text) =
        entries_instrumented(&[ENTRIES_FEEDBACK], &["--seed", base, "--ops", &cases, "--emit"])?;
    if !clean {
        return Err(entries_failure(&text));
    }
    let mut candidates = entries_kept(&text);
    if candidates.is_empty() {
        return Err("the instrumented run kept nothing, so there was no coverage signal.\n\n\
             That is the failure this command exists to notice rather than to survive: a\n\
             recorder that writes an empty corpus and exits zero is a fuzzer claiming\n\
             feedback it does not have. Check that the build carried both\n\
             `-Cinstrument-coverage` and `--features coverage-feedback`, and that the\n\
             report's `feedback` line said `per case`."
            .to_string());
    }
    println!("      {} case(s) reached something no earlier case had", candidates.len());

    // The seeds first, then the corpus already in the tree, then the new
    // candidates by what they added.
    //
    // The seeds lead because they are written to the file first, and a
    // minimisation whose order is not the file's order makes the file's own
    // header false: review found three entries that lit nothing a seed above
    // them had not, kept because minimisation had never seen the seeds. An
    // entry that only duplicates a seed is now dropped, which is what makes the
    // sentence *no earlier line lights this* checkable rather than asserted.
    //
    // Existing entries next, because the file is append-only in spirit: an
    // entry that still reaches something nothing else reaches stays, and one
    // that no longer does is the only kind this drops.
    let seeds: Vec<Vec<String>> = ENTRIES_SEEDS
        .iter()
        .map(|(_, argv)| argv.iter().map(|word| (*word).to_string()).collect())
        .collect();
    let mut ordered: Vec<Vec<String>> = seeds.clone();
    for case in entries_corpus_lines().unwrap_or_default() {
        if !ordered.contains(&case) {
            ordered.push(case);
        }
    }
    candidates.sort_by_key(|(added, _)| std::cmp::Reverse(*added));
    for (_, case) in candidates {
        if !ordered.contains(&case) {
            ordered.push(case);
        }
    }

    println!(
        "\n[2/3] minimising: {} candidate(s) replayed in order behind the {} seed(s), and an\n\
         \x20     entry stays only if it lights a region the ones before it did not",
        ordered.len(),
        seeds.len()
    );
    let warmup: Vec<String> = ENTRIES_WARMUP.iter().map(|s| (*s).to_string()).collect();
    let mut argv: Vec<String> = warmup.clone();
    for case in &ordered {
        argv.extend(case.iter().cloned());
    }
    argv.push("--emit".to_string());
    let borrowed: Vec<&str> = argv.iter().map(String::as_str).collect();
    let (clean, text) = entries_instrumented(&[ENTRIES_FEEDBACK], &borrowed)?;
    if !clean {
        return Err(entries_failure(&text));
    }
    // The warm-up is not an entry, and the seeds are written from
    // `ENTRIES_SEEDS` with their own reasons rather than from what the signal
    // said about them — they exist for regions the signal cannot see, so a
    // `kept` line for one of them would be crediting them with the wrong thing.
    let kept: Vec<(usize, Vec<String>)> = entries_kept(&text)
        .into_iter()
        .filter(|(_, case)| *case != warmup && !seeds.contains(case))
        .collect();
    println!(
        "      {} entr{} survive behind the {} seed(s), which minimisation may not drop",
        kept.len(),
        if kept.len() == 1 { "y" } else { "ies" },
        ENTRIES_SEEDS.len()
    );

    let commit = capture("git", &["rev-parse", "HEAD"]).unwrap_or_else(|_| "unknown".into());
    let body = entries_corpus_text(&kept, commit.trim());
    let path = root().join(ENTRIES_CORPUS);
    std::fs::write(&path, body).map_err(|e| format!("writing {}: {e}", relative(&path)))?;
    println!("\n[3/3] wrote {ENTRIES_CORPUS}");
    Ok(())
}

/// The `kept <n> <argv>` lines a run with `--emit` prints.
fn entries_kept(text: &str) -> Vec<(usize, Vec<String>)> {
    let mut out = Vec::new();
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("kept ") else { continue };
        let mut words = rest.split_whitespace();
        let Some(added) = words.next().and_then(|n| n.parse::<usize>().ok()) else { continue };
        out.push((added, words.map(str::to_string).collect()));
    }
    out
}

/// The header every write of the corpus regenerates.
///
/// Regenerated rather than preserved, which is the opposite of what
/// `ring/corpus.txt` does and is right for the opposite reason: an entry there
/// carries provenance a later run cannot recompute — what it found, and under
/// which defect — while an entry here carries a *measurement*, how many regions
/// it adds, and that number is only true of the file it is in. A recorder that
/// carried an old `adds` figure through would be publishing a number about a
/// corpus that no longer exists.
const ENTRIES_CORPUS_HEADER: &str = "\
# The entry corpus: inputs a coverage signal kept.
#
# Every line below that is not a comment is an argument list for
# `ring/tests/entries.rs`, and every one of them is a *case* — a named world and
# an entry — that lit a coverage region **of this test binary** no earlier line
# in this file lit. `cargo xtask entries --corpus` replays them all and requires
# each to be clean; `cargo xtask entries --coverage` measures what they cover of
# the entry-validation path, and that measurement is the number `claims/0009`
# publishes.
#
# *Of this binary*, said in those words because the two are not the same and the
# difference used to be papered over here. The profile runtime's counter array
# covers every region in the binary, this harness's own included, so an entry
# kept for lighting something new may have lit something new **in the harness**.
# What the entry is worth to the *path* is what `--coverage` measures, one member
# at a time, and that is the number that gates. Making the signal path-aware —
# intersecting the counter set with the regions belonging to the thirty-seven
# members — is the fix that would let this file say the stronger thing; nothing
# in the tree needs it yet, and RFC 0048 carries it as the open end.
#
# There is no format here beyond *a line is an argv*. `sim/corpus.txt` is where
# that shape comes from and `ring/corpus.txt` is the sibling that already
# follows it; a third format would be a third parser and a third thing to keep
# working. The fuzzer's own command-line parser reads an entry, so an entry this
# binary cannot run is an entry that fails to load.
#
# ---- what an entry is worth, and how this file differs from its sibling ----
#
# `ring/corpus.txt` is a regression suite: each line found something once, and
# the comment above it says what, at which commit, under which deliberate
# defect. This file is a **cover**. A line is here because it lit a region the
# lines above it did not, and the `adds` figure is how many it was the first to
# light — regions of this binary, counted by the profile runtime, which is the
# signal minimisation has and not the one the published number is about.
#
# That makes it minimisable in a way a regression suite is not, and
# `--record` minimises it: candidates are replayed in order into one process
# with the coverage signal on, behind the seeds and in this file's own order,
# and a candidate that lights nothing new is dropped. So this file has the
# property a corpus is supposed to have and usually cannot demonstrate — **no
# entry in it lights only what an earlier entry already lit**, and the command
# that says so is the command that wrote it.
#
# It is at a fixpoint, which is the checkable form of that sentence: running
# `cargo xtask entries --record` against this file writes these bytes back. It
# takes two runs to get there from an arbitrary file, because minimisation reads
# the file's order and rewriting the file changes it, and a corpus that never
# settles would be a cover whose membership depended on how many times somebody
# had run the command.
#
# That is a weaker sentence than *no entry is redundant* and it is deliberately
# weaker. Redundant *for the published number* would mean removing the entry
# leaves the coverage figure where it was, and by that measure several entries
# here are redundant: the signal is per binary region and the number is per path
# member, so an entry can earn its place by the first and not move the second.
# The measurement is what settles which, one entry at a time, and the header of a
# corpus is not the place to assert what a measurement has not been run to check.
#
# What that costs, said rather than left to be discovered: minimisation is
# greedy and in order, so a *smaller* cover may exist. And the order is the
# file's order, so an entry near the top is credited with regions an entry near
# the bottom would also have lit. Neither affects what the file is used for.
#
# One consequence is visible on the face of this file and would otherwise read as
# a bug, so: an entry below may be the *same case* as a seed above it, written
# without a field the seed writes explicitly. It survives because the two argvs
# take different branches through this binary's own argument parser, and the
# counter array does not know a parser region from a validation one. It is the
# clearest thing in the tree to point at when arguing for a path-aware signal,
# and it is left here rather than hand-deleted because this file is generated and
# an entry somebody edited in is an entry no command stands behind.
#
# Not append-only, and that is the one rule it does not share with its sibling.
# An entry whose regions are all covered by others is dropped on the next
# `--record`, because a cover that has stopped being minimal is a cover with a
# line nobody can justify. The exception is the seeds: they are hand-written,
# they are never dropped, and each says which region it exists for — because the
# coverage signal is blind to a region LLVM gives an expression rather than a
# counter, and a cover built by a blind signal has holes a measurement finds.
# `ENTRIES_SEEDS` in `xtask/src/main.rs` is the list and RFC 0048's `COUNTER_GAP`
# is the argument.
#
# Reproduce one by hand:
#   cargo test -q --release -p f-ring --test entries -- <the line>
";

/// The corpus file, header and all.
fn entries_corpus_text(kept: &[(usize, Vec<String>)], commit: &str) -> String {
    let mut out = String::from(ENTRIES_CORPUS_HEADER);
    for (why, argv) in ENTRIES_SEEDS {
        out.push_str(
            "
# ----------------------------------------------------------------------
",
        );
        out.push_str(
            "# seed      a region the coverage signal cannot see, so nothing keeps it
",
        );
        out.push_str(&format!(
            "# reaches   {why}
"
        ));
        out.push_str(&format!(
            "{}
",
            argv.join(" ")
        ));
    }
    for (added, case) in kept {
        out.push_str(
            "\n# ----------------------------------------------------------------------\n",
        );
        out.push_str(&format!(
            "# adds      {added} region(s) no earlier entry in this file reaches\n"
        ));
        out.push_str(&format!("# commit    {commit}\n"));
        out.push_str(&format!("{}\n", case.join(" ")));
    }
    out
}

/// A cargo invocation with the coverage instrumentation on.
///
/// Link-time optimisation is **off** for these runs, and that is a measured
/// decision rather than a preference: with `lto = true` — which is what
/// `[profile.release]` carries — `llvm-cov` warns that functions have mismatched
/// data and hands back near-zero coverage for functions that demonstrably ran.
/// So the instrumented build is not the shipped build, and the figure is one
/// about the source rather than about the optimiser's output. RFC 0048.
fn entries_instrumented(features: &[&str], args: &[&str]) -> Result<(bool, String), String> {
    let profiles = target_dir().join("entries-coverage");
    if profiles.exists() {
        std::fs::remove_dir_all(&profiles)
            .map_err(|e| format!("clearing {}: {e}", relative(&profiles)))?;
    }
    std::fs::create_dir_all(&profiles)
        .map_err(|e| format!("creating {}: {e}", relative(&profiles)))?;

    let mut argv: Vec<String> = ["test", "-q", "--release", "-p", "f-ring", "--test", "entries"]
        .iter()
        .map(|s| (*s).to_string())
        .collect();
    if !features.is_empty() {
        argv.push("--features".into());
        argv.push(features.join(","));
    }
    argv.push("--".into());
    argv.extend(args.iter().map(|s| (*s).to_string()));

    let out = Command::new("cargo")
        .args(&argv)
        .envs(entries_instrument_env())
        .env("LLVM_PROFILE_FILE", profiles.join("e-%p-%m.profraw"))
        .current_dir(root())
        .output()
        .map_err(|e| format!("could not run cargo: {e}"))?;

    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&stderr);
    if !text.contains("entries \u{2014} ") {
        return Err(format!(
            "the instrumented fuzzer printed no report, so it did not run:\n{}",
            stderr.trim()
        ));
    }
    if features.contains(&ENTRIES_FEEDBACK) && !text.contains("feedback   per case") {
        return Err("the build carried the feedback feature and the run reported no per-case\n\
             signal, which means the profile runtime's counters were not there. A\n\
             recorder that keeps nothing and exits zero is the failure this refuses."
            .to_string());
    }
    Ok((out.status.success(), text))
}

/// The two environment variables an instrumented run needs.
const fn entries_instrument_env() -> [(&'static str, &'static str); 2] {
    [("RUSTFLAGS", "-Cinstrument-coverage"), ("CARGO_PROFILE_RELEASE_LTO", "false")]
}

/// Measure the entry-validation path's line coverage, from the corpus alone.
///
/// # Why the corpus alone
///
/// Because that is what makes the corpus an artefact rather than a souvenir. A
/// number measured over a long drawn run says the *generator* reaches the path;
/// a number measured over the committed file says a stranger with this
/// repository reaches it, in seconds, with no seed and no fuzzing run.
fn entries_coverage() -> Result<(), String> {
    entries_path_matches_claim()?;
    let profdata = llvm_tool("llvm-profdata")?;
    let llvm_cov = llvm_tool("llvm-cov")?;

    let lines = entries_corpus_lines()?;
    if lines.is_empty() {
        return Err(format!("{ENTRIES_CORPUS} holds no entries, so there is nothing to measure"));
    }
    let argv: Vec<&str> = lines.iter().flat_map(|line| line.iter().map(String::as_str)).collect();

    println!(
        "measuring the entry-validation path over {} corpus entr{}, with the feedback\n\
         feature off: the per-case signal resets the counters, so a build that has it\n\
         writes a profile for the last case rather than for the run. RFC 0048.",
        lines.len(),
        if lines.len() == 1 { "y" } else { "ies" }
    );
    let (clean, text) = entries_instrumented(&[], &argv)?;
    if !clean {
        return Err(entries_failure(&text));
    }

    let profiles = target_dir().join("entries-coverage");
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
             The run happened and wrote no profile, so the instrumented build did not take\n\
             effect. Check that RUSTFLAGS is not already set in the environment or in a\n\
             cargo config, which replaces rather than adds.",
            relative(&profiles)
        ));
    }

    let merged = profiles.join("entries.profdata");
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

    let probe = [
        "test",
        "--release",
        "-p",
        "f-ring",
        "--test",
        "entries",
        "--no-run",
        "--message-format=json",
    ];
    let manifest = capture_with("cargo", &probe, &entries_instrument_env())?;
    let binary = executables(&manifest)
        .into_iter()
        .next_back()
        .ok_or("cargo reported no test executable for `entries`")?;

    let mut report = Command::new(&llvm_cov);
    report.arg("report").arg(format!("--instr-profile={}", merged.display()));
    report.arg(&binary);
    report.arg("--show-functions");
    for source in ENTRIES_SOURCES {
        report.arg(root().join(source));
    }
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

    let measured = entries_measure(&text)?;
    let total: u64 = measured.iter().map(|(_, c)| c.lines).sum();
    let missed: u64 = measured.iter().map(|(_, c)| c.missed).sum();
    #[allow(clippy::cast_precision_loss)]
    let percent = if total == 0 { 0.0 } else { (total - missed) as f64 * 100.0 / total as f64 };

    println!("\ncoverage — the entry-validation path, line by line\n");
    for (member, cover) in &measured {
        #[allow(clippy::cast_precision_loss)]
        let share = if cover.lines == 0 {
            100.0
        } else {
            (cover.lines - cover.missed) as f64 * 100.0 / cover.lines as f64
        };
        let note = match (cover.rows > 1, cover.skipped) {
            (true, 0) => format!("   ({} instantiations, summed)", cover.rows),
            (true, n) => format!("   ({} instantiations, summed; {n} placeholder(s))", cover.rows),
            (false, 0) => String::new(),
            (false, n) => format!("   ({n} placeholder(s))"),
        };
        println!(
            "  {:<24} {:>6.1}%   {:>4} of {:>4}{note}",
            member,
            share,
            cover.lines - cover.missed,
            cover.lines
        );
    }
    println!("  ------------------------");
    println!("  {:<24} {:>6.2}%   {:>4} of {:>4}", "the path", percent, total - missed, total);

    let floor = entries_claim_floor()?;
    #[allow(clippy::cast_precision_loss)]
    let wanted = floor as f64 / 100.0;
    if percent < wanted {
        return Err(format!(
            "the corpus covers {percent:.2}% of the entry-validation path and\n\
             {ENTRIES_CLAIM} states `path_line_coverage = {{ min = {floor} }}`, which is\n\
             {wanted:.2}%.\n\n\
             Two causes needing opposite fixes. The corpus stopped covering a member —\n\
             `cargo xtask entries --record` draws again and re-minimises. Or the path\n\
             *grew*: a new branch in one of the functions `ENTRIES_PATH` names, which is a\n\
             line nothing reaches and is a finding about the code rather than about the\n\
             corpus. The per-member table above says which."
        ));
    }
    println!(
        "\nentries: {percent:.2}% of the entry-validation path, against\n\
         \x20        `path_line_coverage >= {wanted:.2}%` in {ENTRIES_CLAIM}.\n\
         \x20        {} member(s), measured from the committed corpus alone: no seeded run, no\n\
         \x20        generator, and reproducible by a stranger with this checkout.",
        measured.len()
    );
    Ok(())
}

/// Fold `llvm-cov report --show-functions` into one row per path member.
///
/// # The one thing this refuses rather than tolerates
///
/// A member matched by no row. That is a member the fuzzer never reached at all,
/// or a symbol whose mangling moved — and both would otherwise appear as a
/// *smaller denominator*, which is a coverage figure going up because something
/// stopped being measured.
///
/// A member with several instantiations is summed across all of them, which
/// under-reports rather than over-reports when they cover different lines: the
/// union is not computable from this report, and the conservative direction is
/// the one to take with a number that gates.
///
/// Rows whose mangled name carries a placeholder type argument are skipped:
/// rustc emits a zero-coverage record for every generic it did **not**
/// instantiate in this binary, and counting those would report a function that
/// ran as half never having run.
fn entries_measure(report: &str) -> Result<Vec<(&'static str, MemberCoverage)>, String> {
    let mut out: Vec<(&'static str, MemberCoverage)> = ENTRIES_PATH
        .iter()
        .map(|member| (member.name, MemberCoverage { lines: 0, missed: 0, rows: 0, skipped: 0 }))
        .collect();

    for line in report.lines() {
        let row: Vec<&str> = line.split_whitespace().collect();
        if row.len() < 10 {
            continue;
        }
        let Some(symbol) = row.first() else { continue };
        if !symbol.starts_with("_R") {
            continue;
        }
        // The longest matching fragment list wins, which is what separates
        // `f_ring::execute` from `Table::execute`.
        let mut best: Option<(usize, usize)> = None;
        for (index, member) in ENTRIES_PATH.iter().enumerate() {
            if member.fragments.iter().all(|fragment| symbol.contains(fragment))
                && best.is_none_or(|(_, len)| member.fragments.len() > len)
            {
                best = Some((index, member.fragments.len()));
            }
        }
        let Some((index, _)) = best else { continue };

        let cell = |at: usize| -> Result<u64, String> {
            row.get(at)
                .ok_or_else(|| format!("llvm-cov row too short: {line}"))?
                .parse::<u64>()
                .map_err(|_| format!("llvm-cov row not understood: {line}"))
        };
        let lines = cell(4)?;
        let missed = cell(5)?;

        // A row this drops is a row that leaves the denominator, so the pattern
        // that decides it is checked against the thing it is supposed to be
        // true of rather than trusted. A record rustc emitted for a generic it
        // never instantiated here executed nothing, so every one of its lines is
        // missed; a row matching the pattern that has an executed line is a real
        // instantiation the pattern caught by accident, and dropping it would
        // raise the figure by removing uncovered lines — which is exactly the
        // failure this function's contract refuses.
        if entries_is_placeholder(symbol) {
            if missed != lines {
                return Err(format!(
                    "a row of the report reads as a placeholder and is not one:\n  {symbol}\n\n\
                     {} of its {lines} line(s) executed. `entries_is_placeholder` recognises an\n\
                     un-instantiated generic by the `p` and `Kp` v0 mangling writes for a type\n\
                     argument it never filled in, and such a record executes nothing at all. A\n\
                     row with an executed line that matches the pattern is a real instantiation,\n\
                     and dropping it would take *uncovered* lines out of the denominator — a\n\
                     coverage figure going up because something stopped being measured.\n\n\
                     The fix is in the recogniser, not in the threshold: parse the generic\n\
                     argument position rather than matching a substring of the whole symbol.",
                    lines - missed
                ));
            }
            out[index].1.skipped += 1;
            continue;
        }

        out[index].1.lines += lines;
        out[index].1.missed += missed;
        out[index].1.rows += 1;
    }

    let absent: Vec<&str> =
        out.iter().filter(|(_, c)| c.rows == 0).map(|(name, _)| *name).collect();
    if !absent.is_empty() {
        return Err(format!(
            "{} member(s) of the entry-validation path matched no symbol in the report:\n  {}\n\n\
             That is not a coverage failure, it is a *measurement* failure, and it is\n\
             refused rather than counted as a smaller denominator — which is a coverage\n\
             figure going up because something stopped being measured.\n\n\
             Two causes. The fuzzer no longer reaches the function at all, so rustc emitted\n\
             only the placeholder record for it: fix the generator. Or the symbol mangling\n\
             moved, in which case `ENTRIES_PATH`'s fragments need the new one — `llvm-cov\n\
             report --show-functions` prints what it actually saw.",
            absent.len(),
            absent.join("\n  ")
        ));
    }
    Ok(out)
}

/// Is this a record rustc emitted for a generic it never instantiated here?
///
/// v0 mangling writes an un-instantiated type argument as a bare `p` and a const
/// one as `Kp`. Those records exist so a never-called generic shows as uncovered
/// rather than as absent, which is right for a whole-crate report and wrong
/// here: the same function's *real* instantiation is in the same report, and
/// counting both would report every generic on the path as half covered.
///
/// # Why this is a substring match and what stops it failing open
///
/// It is load-bearing — it removes better than a third of the raw lines
/// `llvm-cov` attributes to the path — and a substring of a whole mangled symbol
/// is not the same thing as a generic argument in the argument position. A real,
/// entirely uncovered instantiation whose symbol happened to end an identifier in
/// `p` before an `E` would be dropped, and dropping an uncovered row *raises* the
/// figure.
///
/// So the pattern is not trusted on its own. [`entries_measure`] requires every
/// row this drops to have executed nothing — which a placeholder record does by
/// construction and a real instantiation caught by accident does not — and fails
/// the run naming the symbol when one has an executed line. The recogniser can
/// still be wrong; it can no longer be wrong in the direction that makes the
/// number look better.
fn entries_is_placeholder(symbol: &str) -> bool {
    symbol.contains("Kp") || symbol.contains("pE") || symbol.ends_with('p')
}

/// The floor `claims/0009` states, in hundredths of a per cent.
fn entries_claim_floor() -> Result<u64, String> {
    let path = root().join(ENTRIES_CLAIM);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;
    for line in text.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.strip_prefix("path_line_coverage") else { continue };
        let Some((_, after)) = rest.split_once("min") else { continue };
        let value = after
            .trim_start()
            .strip_prefix('=')
            .and_then(|v| v.split_whitespace().next())
            .map(|v| v.trim_end_matches([',', '}']))
            .and_then(|v| v.parse::<u64>().ok());
        if let Some(value) = value {
            return Ok(value);
        }
    }
    Err(format!(
        "{ENTRIES_CLAIM} states no `path_line_coverage` threshold.\n\n\
         The number this command produces is published, so it gates; a run that invented\n\
         its own floor would be a number checking itself."
    ))
}

/// The path this file walks and the path the claim publishes are one list.
///
/// The guard `hostile_thresholds_match` is for `claims/0008`, here for the same
/// reason: a member named in one and not the other is either a published figure
/// nothing measures or a measured figure nothing publishes, and both read as
/// agreement.
fn entries_path_matches_claim() -> Result<(), String> {
    let path = root().join(ENTRIES_CLAIM);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;
    let mut published: Vec<String> = Vec::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("path = [") {
            inside = true;
            continue;
        }
        if inside {
            if trimmed.starts_with(']') {
                inside = false;
                continue;
            }
            // A comment inside the list is a comment, not a member. The list is
            // grouped by which file each member lives in and those headings are
            // the whole reason it is readable.
            if trimmed.starts_with('#') {
                continue;
            }
            let name = trimmed.trim_start_matches('"').split('"').next().unwrap_or("");
            if !name.is_empty() {
                published.push(name.to_string());
            }
        }
    }

    let declared: Vec<&str> = ENTRIES_PATH.iter().map(|member| member.name).collect();
    let here: Vec<String> =
        published.iter().filter(|name| !declared.contains(&name.as_str())).cloned().collect();
    let there: Vec<String> = declared
        .iter()
        .filter(|name| !published.iter().any(|p| p == *name))
        .map(|name| (*name).to_string())
        .collect();

    if here.is_empty() && there.is_empty() {
        return Ok(());
    }
    let say = |what: &str, list: &[String]| {
        if list.is_empty() { String::new() } else { format!("\n  {what}: {}", list.join(", ")) }
    };
    Err(format!(
        "{ENTRIES_CLAIM}'s `path` and `ENTRIES_PATH` in xtask have drifted.{}{}\n\n\
         The list is the whole of what *95% of the validation path* means, and two copies\n\
         of it is one copy nobody reads. RFC 0048.",
        say("in the claim and not in ENTRIES_PATH", &here),
        say("in ENTRIES_PATH and not in the claim", &there)
    ))
}

/// Every counter the report is required to have moved.
///
/// A fuzzer that reached nothing prints the same two words as one that reached
/// everything. These are the families it must have drawn and the refusals it
/// must have earned for *no finding* to be a statement about the validation path
/// rather than about a run that refused everything at the first check — which is
/// exactly what the first version of `ring/tests/hostile.rs` did.
const ENTRIES_REACHED: &[(&str, &str)] = &[
    ("well-formed", "family_well_formed"),
    ("unknown flag", "family_unknown_flag"),
    ("unknown opcode", "family_unknown_opcode"),
    ("reserved bit", "family_reserved_bit"),
    ("index past set", "family_index_past_set"),
    ("forged generation", "family_forged_generation"),
    ("past arena", "family_past_arena"),
    ("unstatable length", "family_unstatable_length"),
    ("past deadline", "family_past_deadline"),
    ("indivisible region", "family_indivisible_region"),
    ("refused capability", "family_refused_capability"),
    ("malformed name", "family_malformed_name"),
    ("malformed registration", "family_malformed_registration"),
    ("nudged field", "family_nudged_field"),
    ("flipped bytes", "family_flipped_bytes"),
    ("spent", "world_spent"),
    ("accepted", "answers_accepted"),
    ("silent, NO_CQE", "answers_silent"),
    ("ids issued", "answers_ids_issued"),
    ("buffers resolved", "answers_buffers_resolved"),
    ("buffers released", "answers_buffers_released"),
    ("sets torn down", "answers_sets_torn_down"),
    ("arena bytes copied", "answers_arena_bytes_copied"),
    ("argument/bad-address", "refused_bad_address"),
    ("argument/feature-not-negotiated", "refused_feature_not_negotiated"),
    ("argument/reserved-not-zero", "refused_reserved_not_zero"),
    ("argument/unknown-flag", "refused_unknown_flag"),
    ("argument/unknown-opcode", "refused_unknown_opcode"),
    ("authority/no-such-cap", "refused_no_such_cap"),
    ("authority/revoked", "refused_revoked"),
    ("peer/feature-required", "refused_feature_required"),
    ("resource/quota-exhausted", "refused_quota_exhausted"),
    ("namings taken", "client_namings_taken"),
    ("buffers returned", "client_buffers_returned"),
    ("buffers reclaimed", "client_buffers_reclaimed"),
    ("submissions refused", "client_submissions_refused"),
    ("regions refused", "client_regions_refused"),
];

/// The rows of `claims/0009`'s `[threshold]` table that are not reach counts.
///
/// Named as an exclusion rather than deriving the reach rows by a prefix, which
/// is `HOSTILE_NOT_REACH`'s arrangement and its reason: a prefix is a convention
/// and this is a short list. A new row added to that table without a counter in
/// [`ENTRIES_REACHED`] is a published minimum nothing reads, and
/// [`entries_thresholds_match`] is what says so.
const ENTRIES_NOT_REACH: &[&str] = &["path_line_coverage", "cases", "panics", "wrong"];

/// The reach rows the claim publishes and the counters this file reads are one
/// list.
///
/// The same guard `hostile_thresholds_match` is for `claims/0008`, and it exists
/// because that task found the two had been one list written twice with nothing
/// checking it. A row deleted from either side is otherwise a minimum nobody
/// enforces on a counter nobody publishes, and both read as agreement.
fn entries_thresholds_match() -> Result<(), String> {
    let rows = entries_thresholds()?;
    let published: Vec<&String> =
        rows.keys().filter(|key| !ENTRIES_NOT_REACH.contains(&key.as_str())).collect();
    let read: Vec<&str> = ENTRIES_REACHED.iter().map(|(_, key)| *key).collect();

    let here: Vec<String> = published
        .iter()
        .filter(|key| !read.contains(&key.as_str()))
        .map(|key| (*key).clone())
        .collect();
    let there: Vec<String> = read
        .iter()
        .filter(|key| !published.iter().any(|p| p.as_str() == **key))
        .map(|key| (*key).to_string())
        .collect();

    if here.is_empty() && there.is_empty() {
        return Ok(());
    }
    let say = |what: &str, list: &[String]| {
        if list.is_empty() {
            String::new()
        } else {
            format!(
                "
  {what}: {}",
                list.join(", ")
            )
        }
    };
    Err(format!(
        "{ENTRIES_CLAIM}'s reach thresholds and ENTRIES_REACHED have drifted.{}{}

         A minimum with no counter behind it is a published number nothing checks, and a
         counter with no minimum behind it is a path that can fall to zero without a
         word. RFC 0048.",
        say("in the claim and not in ENTRIES_REACHED", &here),
        say("in ENTRIES_REACHED and not in the claim", &there)
    ))
}

/// Every counter the run reported, against the minimum `claims/0009` states.
fn entries_reached(text: &str, cases: u64) -> Result<(), String> {
    entries_thresholds_match()?;
    let rows = entries_thresholds()?;
    let mut short: Vec<String> = Vec::new();
    let mut met = 0usize;

    for (label, key) in ENTRIES_REACHED {
        let seen = text
            .lines()
            .filter_map(|line| {
                let rest = line.trim_start().strip_prefix(label)?;
                rest.split_whitespace().next()?.parse::<u64>().ok()
            })
            .next()
            .unwrap_or(0);
        let stated = rows.get(*key).copied().unwrap_or(0);
        // Stated per the gate's own count and scaled to the run, `claims/0008`'s
        // arrangement: a short diagnostic run still enforces *reached at all*
        // rather than being held to a count it cannot make.
        let scaled = stated.saturating_mul(cases) / ENTRIES_GATE.max(1);
        let floor = if stated > 0 { scaled.max(1) } else { 0 };
        if seen < floor {
            short.push(format!("  {label:<34}{seen:>10}  against a minimum of {floor}"));
        } else {
            met += 1;
        }
    }

    if !short.is_empty() {
        return Err(format!(
            "{} of {} path(s) {ENTRIES_CLAIM} names were not reached:\n{}\n\n\
             The run was clean and something stopped being exercised, which is the failure\n\
             this list exists to make visible. Two causes needing opposite fixes: the\n\
             generator stopped producing that input — `Draw::family`'s weights, or\n\
             `Draw::case` — or the code stopped having that path, which is a finding about\n\
             the ring.",
            short.len(),
            ENTRIES_REACHED.len(),
            short.join("\n")
        ));
    }
    println!("entries: clean, and every one of the {met} paths the claim names was reached.");
    Ok(())
}

/// `claims/0009`'s reach minimums, read rather than restated.
fn entries_thresholds() -> Result<std::collections::BTreeMap<String, u64>, String> {
    let path = root().join(ENTRIES_CLAIM);
    let text =
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", relative(&path)))?;
    let mut rows = std::collections::BTreeMap::new();
    let mut inside = false;
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[threshold]";
            continue;
        }
        if !inside || trimmed.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = trimmed.split_once('=') else { continue };
        let Some((_, after)) = rest.split_once("min") else { continue };
        let value = after
            .trim_start()
            .strip_prefix('=')
            .and_then(|v| v.split_whitespace().next())
            .map(|v| v.trim_end_matches([',', '}']))
            .and_then(|v| v.parse::<u64>().ok());
        if let Some(value) = value {
            rows.insert(key.trim().to_string(), value);
        }
    }
    Ok(rows)
}

/// Arm each deliberate defect in turn, and require the oracle it breaks — and no
/// other — to find it.
fn entries_mutate() -> Result<(), String> {
    let cases = ENTRIES_MUTATE_CASES.to_string();
    for (feature, words, oracle) in ENTRIES_DEFECTS {
        println!("\n--- {feature}: {oracle} must be the oracle that finds it");
        let (clean, text) =
            entries_run(&[feature], &["--seed", TRACE_SEED, "--ops", &cases], false)?;
        if clean {
            return Err(format!(
                "the fuzzer ran {cases} case(s) against `{feature}` and found nothing.\n\n\
                 That is {oracle} failing rather than holding: the defect is armed and the\n\
                 run is green, so either the generator no longer reaches it or the check\n\
                 that would have caught it has stopped being made."
            ));
        }
        let finding = text
            .lines()
            .find(|line| line.starts_with("finding 1  "))
            .unwrap_or("finding 1  (the report's shape moved)");
        if !finding.contains(words) {
            return Err(format!(
                "`{feature}` was found, and not by {oracle}:\n\n  {finding}\n\n\
                 A defect found by the wrong oracle is a harness whose oracles are one\n\
                 oracle wearing three names, which is RFC 0042's finding applied here. The\n\
                 line above should contain `{words}`."
            ));
        }
        println!("{feature}: caught —\n  {}", finding.trim());
    }

    println!("\n--- and without any of them, the same run must be clean");
    let (clean, text) = entries_run(&[], &["--seed", TRACE_SEED, "--ops", &cases], false)?;
    if !clean {
        return Err(format!(
            "the fuzzer found something with no defect armed, so the three runs above say\n\
             nothing about the defects.\n\n{}",
            entries_failure(&text)
        ));
    }
    println!(
        "\nentries: all {} defect(s) caught, each by the oracle it breaks and by no other,\n\
         \x20        and the same run is clean without them.",
        ENTRIES_DEFECTS.len()
    );
    Ok(())
}
