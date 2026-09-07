// SPDX-License-Identifier: Apache-2.0 OR MIT
//! `E2-B05`'s exit, demonstrated rather than argued.
//!
//! Two sentences, and this file is arranged as the two of them:
//!
//! 1. **The same root produces a byte-identical topology.** The topology is
//!    instantiated twice from one root — with the bus shuffled between the two
//!    runs under a seeded `Env` — and the two renderings are compared byte for
//!    byte. `f_assembler::render` is where *what byte-identical is taken over*
//!    is written down; the short answer is *every decision the assembler made*,
//!    and deliberately not the module it was handed, which is the input.
//! 2. **A driver that fails to start leaves its subtree unstarted rather than
//!    failing the boot.** One driver is failed on purpose, twice over — once by
//!    a starter that refuses it, once by taking its card out of the machine —
//!    and what is checked is that its subtree is unstarted, that everything
//!    outside the subtree started, and that the run finished with a report
//!    rather than an error.
//!
//! # Why the module is built here rather than read from a build
//!
//! Because `cargo xtask generation` needs a kernel image and four component
//! images, and a test that took a full build to run is a test nobody runs. What
//! is built here goes through **the same encoders the compiler uses** —
//! `f_abi::store` for the tree, `f_abi::manifest::encode` for each component
//! file, `f_generation::fold` for the root — so nothing about the format is
//! restated. The one thing this file writes for itself is the boot module's
//! head, and it writes it through `f_abi::boot`'s own constants.
//!
//! # No harness, because the exit is what the log has to contain
//!
//! A libtest harness captures output, and both halves above are things a reader
//! of the log has to be able to see: the digest that was equal, and the state
//! of every component after the failure. RFC 0046's arrangement, the same one
//! `index/tests/query.rs` and `zone/tests/cycle.rs` take.

use f_abi::cap::{CapType, rights};
use f_abi::manifest::{Binding, NAME_MAX, Need, Record, class, domain, name_bytes, restart, route};
use f_abi::store::{Generation, Leaf, Route, Topology, node};
use f_abi::transfer::Declaration;
use f_assembler::render;
use f_assembler::start::{Report, Start, State};
use f_assembler::{Address, Assembly, Bus, Discovered, bind};
use f_env::split::Stream;

/// The vendor every device in this workload is made by, which is the one this
/// tree's drivers declare.
/// Unit: none — a PCI vendor identifier.
const VENDOR: u16 = 0x1AF4;

/// The seed the bus permutation is drawn from.
///
/// One number, in the source, so that a reader can run the exact permutation
/// this file asserts about. RFC 0026: the identity below is what makes adding a
/// draw site later unable to move it.
const SEED: u64 = 0x0000_00E0_2B05_0001;

/// The identity the bus permutation's stream is split at: the ASCII of `bus`.
/// Unit: none — a stream identity.
const BUS_IDENTITY: u64 = 0x0062_7573;

fn main() {
    let workload = Workload::new();
    println!("assemble  the assembler: one root, one topology\n");
    println!(
        "  workload   {} components, {} routes, {} devices on the bus, seed {SEED:#x}",
        workload.names.len(),
        workload.routes.len(),
        workload.devices.len()
    );
    println!("  root       {}\n", hex(&workload.root));

    let mut tally = Tally::default();
    let mut failures = 0;
    failures += the_same_root_produces_a_byte_identical_topology(&workload, &mut tally);
    failures += a_bus_in_any_order_produces_one_topology(&workload, &mut tally);
    failures += a_module_that_does_not_fold_to_the_root_is_refused(&workload, &mut tally);
    failures += a_driver_that_fails_to_start_leaves_its_subtree_unstarted(&workload, &mut tally);
    failures += a_driver_whose_card_is_absent_costs_its_subtree_and_no_more(&workload, &mut tally);
    failures += two_drivers_claiming_one_device_is_refused(&workload, &mut tally);

    println!();
    tally.rows();

    println!();
    if failures == 0 {
        println!("assemble: ok  (6 demonstrations)");
    } else {
        println!("assemble: FAILED  ({failures} of 6)");
        std::process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// The rows `claims/0027 topology-renderings-per-root` compares against.
// ---------------------------------------------------------------------------

/// What the six demonstrations counted, printed once at the end as
/// `name value` rows.
///
/// # Why a struct and not six `println!`s where the numbers are computed
///
/// Because `xtask`'s claim route reads a row as *a claim-registered name,
/// whitespace, and a count*, and a row emitted in the middle of a demonstration
/// is a row whose name a reader has to re-find every time this file is
/// reordered. Collected here, the block at the bottom is the claim's table and
/// the demonstrations above it are the evidence.
///
/// The load-bearing member is [`Tally::renderings`], and it is a set rather
/// than a pair of booleans on purpose: **every** instantiation of this file's
/// one root goes into it, so the headline row is the size of a set and not a
/// count of equalities that happened to hold pairwise. Two renderings that
/// agree with each other and disagree with the other nine would satisfy every
/// `==` in this file and would still put a 2 in that row.
#[derive(Default)]
struct Tally {
    /// Every rendering produced from `Workload::root`, deduplicated.
    /// Unit: distinct byte strings.
    renderings: std::collections::BTreeSet<Vec<u8>>,
    /// How many instantiations went into that set.
    /// Unit: instantiations.
    instantiations: usize,
    /// The width of one rendering, which is what *byte-identical* is taken
    /// over. A zero here would make the headline row true and empty.
    /// Unit: bytes.
    rendered_bytes: usize,
    /// Bus permutations drawn.
    /// Unit: permutations.
    orders_drawn: usize,
    /// How many of those were distinct device orders.
    /// Unit: distinct orders.
    orders_distinct: usize,
    /// Refusals reached: a wrong root, a tampered file, a route nothing
    /// declared, a need nothing routes, and two claimants for one part.
    /// Unit: refusals.
    refusals: usize,
    /// The second half of the exit, taken off the failing run's report.
    /// Unit: components.
    failed: usize,
    /// Unit: components.
    unstarted: usize,
    /// Unit: components.
    started_outside_the_subtree: usize,
    /// Members left unstarted that are not in the failed driver's subtree, plus
    /// members of that subtree that started anyway. Zero in both directions,
    /// and the only row here whose threshold is a ceiling.
    /// Unit: components.
    subtree_mismatches: usize,
    /// Unit: components.
    unstarted_for_an_absent_card: usize,
}

impl Tally {
    /// Record one instantiation's rendering.
    fn saw(&mut self, rendering: &[u8]) {
        self.instantiations += 1;
        self.rendered_bytes = rendering.len();
        self.renderings.insert(rendering.to_vec());
    }

    /// The block `cargo xtask claim topology-renderings-per-root` reads.
    fn rows(&self) {
        println!("  claims/0027 topology-renderings-per-root");
        for (name, value) in [
            ("distinct_topology_renderings_per_root", self.renderings.len()),
            ("topology_instantiations_rendered", self.instantiations),
            ("rendered_topology_bytes", self.rendered_bytes),
            ("bus_orders_drawn", self.orders_drawn),
            ("bus_orders_distinct", self.orders_distinct),
            ("refusals_demonstrated", self.refusals),
            ("components_failed_on_purpose", self.failed),
            ("components_left_unstarted", self.unstarted),
            ("components_started_outside_the_subtree", self.started_outside_the_subtree),
            ("subtree_membership_mismatches", self.subtree_mismatches),
            ("components_unstarted_for_an_absent_card", self.unstarted_for_an_absent_card),
        ] {
            println!("    {name:<42} {value}");
        }
    }
}

// ---------------------------------------------------------------------------
// The first half of the exit: boot is a pure function of one hash.
// ---------------------------------------------------------------------------

/// Instantiate twice from one root, start both the same way, and compare the
/// bytes.
fn the_same_root_produces_a_byte_identical_topology(
    workload: &Workload,
    tally: &mut Tally,
) -> usize {
    let mut first = Assembly::instantiate(&workload.root, &workload.module).expect("first");
    bind::bind(&mut first, &workload.bus_in_scan_order()).expect("bind");
    let report_one = f_assembler::start::start(&mut first, &mut Always);

    let mut second = Assembly::instantiate(&workload.root, &workload.module).expect("second");
    bind::bind(&mut second, &workload.bus_in_scan_order()).expect("bind");
    let report_two = f_assembler::start::start(&mut second, &mut Always);

    let one = render::topology(&first);
    let two = render::topology(&second);
    tally.saw(&one);
    tally.saw(&two);

    println!("  byte-identical over {} bytes of rendered topology", one.len());
    println!("    run 1    {}  {report_one:?}", hex(&render::digest(&first)));
    println!("    run 2    {}  {report_two:?}", hex(&render::digest(&second)));

    let mut failures = 0;
    failures += check("two instantiations render to one blob", one == two);
    failures +=
        check("both runs started every component", report_one.whole() && report_two.whole());
    // A rendering that covered nothing would also be equal, so the blob has to
    // carry what it claims to: the root, and a byte for every member.
    failures += check("the rendering names its own root", one[8..40] == workload.root);
    failures
}

/// The same root, with the bus handed over in a different order each time.
///
/// The property that makes the first demonstration mean something: if a
/// discovery order could reach a topology, *this* is the test that would go
/// red, and the one above would still pass on a machine whose scan never
/// changed.
fn a_bus_in_any_order_produces_one_topology(workload: &Workload, tally: &mut Tally) -> usize {
    let mut digests = Vec::new();
    let mut renderings = Vec::new();
    for run in 0..8u64 {
        let mut assembly = Assembly::instantiate(&workload.root, &workload.module).expect("root");
        bind::bind(&mut assembly, &workload.bus_shuffled(run)).expect("bind");
        f_assembler::start::start(&mut assembly, &mut Always);
        digests.push(hex(&render::digest(&assembly)));
        let rendering = render::topology(&assembly);
        tally.saw(&rendering);
        renderings.push(rendering);
    }

    tally.orders_drawn = 8;
    tally.orders_distinct = workload.distinct_orders(8);

    let distinct: std::collections::BTreeSet<&String> = digests.iter().collect();
    println!("\n  a shuffled bus, 8 seeded permutations");
    println!("    orders   {}", workload.orders_seen(8));
    println!("    digests  {} distinct", distinct.len());

    let mut failures = 0;
    failures += check("eight discovery orders give one topology", distinct.len() == 1);
    failures += check("and it is the same one the unshuffled bus gave", {
        let mut plain = Assembly::instantiate(&workload.root, &workload.module).expect("root");
        bind::bind(&mut plain, &workload.bus_in_scan_order()).expect("bind");
        f_assembler::start::start(&mut plain, &mut Always);
        let rendering = render::topology(&plain);
        tally.saw(&rendering);
        renderings[0] == rendering
    });
    failures
}

/// A module whose fold is not the root the boot selected is refused, and
/// refused before anything in it is believed.
fn a_module_that_does_not_fold_to_the_root_is_refused(
    workload: &Workload,
    tally: &mut Tally,
) -> usize {
    let mut wrong = workload.root;
    wrong[0] ^= 0x01;
    let refused = Assembly::instantiate(&wrong, &workload.module);

    // And a module whose *bytes* were edited under a correct root: the tree
    // still checks, the root still matches nothing, and the refusal is the same
    // one — which is what "the assembler recomputes the fold" buys.
    let mut tampered = workload.module.clone();
    let at = tampered.len() - 1;
    tampered[at] ^= 0xFF;
    let content = Assembly::instantiate(&workload.root, &tampered);

    // A route to a capability nobody declared. The refusal `user/generation.toml`
    // was waiting for: until it existed, a route was a statement about intent
    // that changed the root and bound nothing.
    let unrouted = workload.module_with_an_unasked_route();
    let unrouted = Assembly::instantiate(&workload.root_of(&unrouted), &unrouted);

    // And the same disagreement from the other side: a component that needs a
    // sibling's capability and is routed none.
    let unsupplied = workload.module_with_an_unrouted_need();
    let unsupplied = Assembly::instantiate(&workload.root_of(&unsupplied), &unsupplied);

    println!("\n  refusals");
    println!("    wrong root       {refused:?}");
    println!("    tampered file    {content:?}");
    println!("    route, no need   {}", why(workload, &unrouted));
    println!("    need, no route   {}", why(workload, &unsupplied));

    let mut failures = 0;
    failures += check(
        "a root the module does not fold to is refused",
        matches!(refused, Err(f_assembler::Refusal::Root)),
    );
    failures += check(
        "a component file that is not the bytes its leaf names is refused",
        matches!(content, Err(f_assembler::Refusal::Content(_))),
    );
    failures += check(
        "a route handing over a capability nothing declared is refused",
        matches!(unrouted, Err(f_assembler::Refusal::Unrouted(_, _))),
    );
    failures += check(
        "a required sibling need the topology routes nothing is refused",
        matches!(unsupplied, Err(f_assembler::Refusal::Unsupplied(_, _))),
    );
    // Counted off the four `check`s above rather than written as a literal 4:
    // a demonstration deleted from this function then moves the row rather than
    // leaving a constant behind that says it is still here.
    tally.refusals += 4 - failures;
    failures
}

// ---------------------------------------------------------------------------
// The second half: a failure costs its subtree and not the boot.
// ---------------------------------------------------------------------------

/// Fail `blk` on purpose. Its subtree is `objects` and, through it, `shell`;
/// `net`, `gpu` and `log` are outside the subtree and start.
fn a_driver_that_fails_to_start_leaves_its_subtree_unstarted(
    workload: &Workload,
    tally: &mut Tally,
) -> usize {
    let mut assembly = Assembly::instantiate(&workload.root, &workload.module).expect("root");
    bind::bind(&mut assembly, &workload.bus_in_scan_order()).expect("bind");

    let blk = workload.index_of("blk");
    let report = f_assembler::start::start(&mut assembly, &mut Refuse(blk, REFUSED));

    println!("\n  blk is failed on purpose");
    print_states(workload, &assembly);
    println!("    report   {report:?}");

    let objects = workload.index_of("objects");
    let shell = workload.index_of("shell");
    let mut failures = 0;
    failures += check("blk failed", state(&assembly, blk) == State::Failed(REFUSED));

    // The general statement, rather than three names: the set of components
    // left unstarted **is** the subtree the routes define, computed from the
    // topology and not written down here. A test that named the three would
    // pass on an assembler that stopped a component too many or too few, as
    // long as those three were among them.
    let cost: Vec<u16> = assembly
        .members()
        .iter()
        .filter(|member| matches!(member.state, State::Unstarted(_)))
        .map(|member| member.index)
        .collect();
    // The symmetric difference between what stopped and what the topology says
    // blk's subtree is: a member stopped that is not in the subtree, plus a
    // member of the subtree that started anyway. Both directions, because a
    // one-directional count is green on an assembler that stops everything.
    let subtree = assembly.subtree(blk);
    tally.subtree_mismatches = cost.iter().filter(|index| !subtree.contains(index)).count()
        + subtree.iter().filter(|index| !cost.contains(index)).count();
    failures += check(
        &format!("what stopped is exactly blk's subtree ({})", workload.names_of(&cost)),
        cost == subtree,
    );
    failures += check(
        "and each one names the nearest component that did not start",
        state(&assembly, objects) == State::Unstarted(blk)
            && state(&assembly, shell) == State::Unstarted(objects),
    );
    for outside in ["net", "gpu", "log"] {
        failures += check(
            &format!("{outside}, outside the subtree, started"),
            state(&assembly, workload.index_of(outside)) == State::Started,
        );
    }
    // The boot is alive: a report came back, and it is a report about a
    // topology rather than an abandoned one.
    failures += check("the boot is alive", report != Report::default() && !report.whole());
    failures += check("exactly one component failed", report.failed == 1);
    failures += check("exactly two were left unstarted", report.unstarted == 2);
    tally.failed = report.failed;
    tally.unstarted = report.unstarted;
    tally.started_outside_the_subtree = report.started;
    failures
}

/// The same shape reached the other way: the driver is fine and the card is
/// gone. The subtree is the same subtree, and the state tells the operator
/// which of the two it was.
fn a_driver_whose_card_is_absent_costs_its_subtree_and_no_more(
    workload: &Workload,
    tally: &mut Tally,
) -> usize {
    let mut assembly = Assembly::instantiate(&workload.root, &workload.module).expect("root");
    let mut bus = workload.bus_in_scan_order();
    let bus = {
        // Take the block card out of the machine, and nothing else.
        let mut without = Bus::new();
        for device in bus.devices() {
            if device.device != BLK_PART {
                without.saw(*device);
            }
        }
        bus = without;
        bus
    };
    let bound = bind::bind(&mut assembly, &bus).expect("bind");
    let report = f_assembler::start::start(&mut assembly, &mut Always);

    println!("\n  the block card is not in the machine");
    print_states(workload, &assembly);
    println!("    report   {report:?}  ({bound} bound of {} devices)", bus.len());

    let blk = workload.index_of("blk");
    let mut failures = 0;
    failures += check(
        "blk has no device, and says so rather than saying it failed",
        state(&assembly, blk) == State::NoDevice,
    );
    failures += check(
        "objects is unstarted",
        state(&assembly, workload.index_of("objects")) == State::Unstarted(blk),
    );
    failures +=
        check("net still started", state(&assembly, workload.index_of("net")) == State::Started);
    failures += check("nothing was reported as failed", report.failed == 0 && report.absent == 1);
    tally.unstarted_for_an_absent_card = report.unstarted;
    failures
}

/// Two components declaring one part is refused, not resolved.
fn two_drivers_claiming_one_device_is_refused(workload: &Workload, tally: &mut Tally) -> usize {
    let module = workload.module_with_a_second_claimant();
    let root = workload.root_of(&module);
    let mut assembly = Assembly::instantiate(&root, &module).expect("root");
    let refused = bind::bind(&mut assembly, &workload.bus_in_scan_order());

    println!("\n  two components declare one part");
    println!("    bind     {refused:?}");

    let held = matches!(refused, Err(bind::Refusal::Claimed(_, _, _)));
    tally.refusals += usize::from(held);
    check("the binding refuses rather than choosing", held)
}

// ---------------------------------------------------------------------------
// The workload: a topology of six components, built through the real encoders.
// ---------------------------------------------------------------------------

/// The packed error a refusing starter reports.
/// Unit: none — a packed `f_abi::error`.
const REFUSED: i32 = -1;

/// The part number the block driver in this workload declares.
/// Unit: none — a PCI device identifier.
const BLK_PART: u16 = 0x1042;

/// A starter that starts everything.
struct Always;

impl Start for Always {
    fn start(&mut self, _index: u16, _record: &Record, _at: Option<Address>) -> Result<(), i32> {
        Ok(())
    }
}

/// A starter that refuses exactly one member, and starts everything else.
struct Refuse(u16, i32);

impl Start for Refuse {
    fn start(&mut self, index: u16, _record: &Record, _at: Option<Address>) -> Result<(), i32> {
        if index == self.0 { Err(self.1) } else { Ok(()) }
    }
}

/// The six components, their routes and the devices they bind.
///
/// Six rather than four, because the exit's word is **subtree** and a
/// one-component subtree cannot tell a subtree apart from a child: `shell`
/// routes from `objects`, which routes from `blk`, so failing `blk` has to
/// reach two components deep or the demonstration is weaker than the sentence.
struct Workload {
    names: Vec<&'static str>,
    routes: Vec<Route>,
    devices: Vec<Discovered>,
    module: Vec<u8>,
    root: [u8; 32],
}

impl Workload {
    fn new() -> Self {
        // Canonical order: bytewise on the zero-padded name, which for this
        // alphabet is alphabetical. The compiler enforces it and so does the
        // checker, so this list is written in it rather than sorted here.
        let names = vec!["blk", "gpu", "log", "net", "objects", "shell"];
        let files: Vec<Vec<u8>> = names.iter().map(|name| component(name)).collect();

        // Routes, canonical on (component index, capability). `objects` needs a
        // block device from `blk`; `shell` needs objects from `objects` and a
        // place to write from `log`.
        let index = |want: &str| names.iter().position(|n| *n == want).unwrap() as u16;
        let mut routes = vec![
            Route {
                component: index("objects"),
                source: index("blk"),
                capability: name_bytes("block").unwrap(),
            },
            Route {
                component: index("shell"),
                source: index("log"),
                capability: name_bytes("log").unwrap(),
            },
            Route {
                component: index("shell"),
                source: index("objects"),
                capability: name_bytes("objects").unwrap(),
            },
        ];
        routes.sort_by_key(|route| (route.component, route.capability));

        let tree = encode_tree(&names, &files, &routes);
        let module = pack(&tree, &files);
        let checked = f_generation::Tree::check(&tree).expect("a tree this file encoded");
        let root = f_generation::fold::root(&checked);

        // Eight devices on the bus, three of which are the parts the three
        // drivers declare and five of which are nothing anybody asked for. The
        // five matter: a bus with only the wanted devices on it would let a
        // binding that took *the first device* pass.
        let devices = vec![
            Discovered {
                at: Address { bus: 0, device: 1, function: 0 },
                vendor: VENDOR,
                device: 0x1009,
            },
            Discovered {
                at: Address { bus: 0, device: 2, function: 0 },
                vendor: VENDOR,
                device: 0x1041,
            },
            Discovered {
                at: Address { bus: 0, device: 3, function: 0 },
                vendor: VENDOR,
                device: 0x100a,
            },
            Discovered {
                at: Address { bus: 0, device: 4, function: 0 },
                vendor: VENDOR,
                device: BLK_PART,
            },
            Discovered {
                at: Address { bus: 0, device: 4, function: 1 },
                vendor: VENDOR,
                device: 0x100b,
            },
            Discovered {
                at: Address { bus: 1, device: 0, function: 0 },
                vendor: VENDOR,
                device: 0x1050,
            },
            Discovered {
                at: Address { bus: 1, device: 1, function: 0 },
                vendor: VENDOR,
                device: 0x100c,
            },
            Discovered {
                at: Address { bus: 2, device: 0, function: 0 },
                vendor: VENDOR,
                device: 0x100d,
            },
        ];

        Self { names, routes, devices, module, root }
    }

    fn index_of(&self, name: &str) -> u16 {
        self.names.iter().position(|n| *n == name).expect("a name this workload has") as u16
    }

    /// The bus as this file wrote it down.
    fn bus_in_scan_order(&self) -> Bus {
        let mut bus = Bus::new();
        for device in &self.devices {
            bus.saw(*device);
        }
        bus
    }

    /// A list of member indices, as names, for a line a reader can check.
    fn names_of(&self, indices: &[u16]) -> String {
        indices.iter().map(|index| self.names[*index as usize]).collect::<Vec<_>>().join(", ")
    }

    /// The bus, handed over in the permutation seed `run` names.
    ///
    /// A seeded permutation rather than one written by hand, because the
    /// property is about the *class* of orders and a hand-written one is a
    /// single member of it. RFC 0026's split: the identity is `bus` and the run
    /// number, so adding a draw site anywhere else cannot move these.
    fn bus_shuffled(&self, run: u64) -> Bus {
        let mut bus = Bus::new();
        for device in self.permutation(run) {
            bus.saw(device);
        }
        bus
    }

    fn permutation(&self, run: u64) -> Vec<Discovered> {
        let mut stream = Stream::from_seed(SEED).split(BUS_IDENTITY).split(run);
        let mut order = self.devices.clone();
        // Fisher-Yates, drawn from the stream: every permutation is reachable,
        // which a rotation or a reversal would not be.
        for i in (1..order.len()).rev() {
            let j = (stream.next_u64() % (i as u64 + 1)) as usize;
            order.swap(i, j);
        }
        order
    }

    /// How many distinct device orders the first `runs` permutations were.
    ///
    /// Split out from the line below because `claims/0027` needs the number and
    /// a reader needs the sentence, and a claim row parsed back out of a
    /// formatted sentence is the two-readers problem in miniature. It is the
    /// row that goes red if `permutation` ever stops permuting: eight draws of
    /// one order would leave every equality in this file true and the property
    /// it is about untested.
    fn distinct_orders(&self, runs: u64) -> usize {
        let mut seen = std::collections::BTreeSet::new();
        for run in 0..runs {
            let order: Vec<String> = self
                .permutation(run)
                .iter()
                .map(|d| format!("{:02x}:{:02x}.{}", d.at.bus, d.at.device, d.at.function))
                .collect();
            seen.insert(order.join(" "));
        }
        seen.len()
    }

    /// How the first `runs` permutations addressed the bus, as one line.
    fn orders_seen(&self, runs: u64) -> String {
        format!("{} distinct of {runs}", self.distinct_orders(runs))
    }

    /// The same six components, with `log` also declaring the block driver's
    /// part — which is the manifest set `cargo xtask lint-manifests` refuses at
    /// compile time, built here so that the run-time refusal is exercised too.
    fn module_with_a_second_claimant(&self) -> Vec<u8> {
        let files: Vec<Vec<u8>> = self
            .names
            .iter()
            .map(|name| if *name == "log" { claimant("log", BLK_PART) } else { component(name) })
            .collect();
        let tree = encode_tree(&self.names, &files, &self.routes);
        pack(&tree, &files)
    }

    /// The same six components, with one more route: `gpu` is handed a
    /// capability its manifest never asked for.
    fn module_with_an_unasked_route(&self) -> Vec<u8> {
        let files: Vec<Vec<u8>> = self.names.iter().map(|name| component(name)).collect();
        let mut routes = self.routes.clone();
        routes.push(Route {
            component: self.index_of("gpu"),
            source: self.index_of("log"),
            capability: name_bytes("frames").expect("a literal"),
        });
        routes.sort_by_key(|route| (route.component, route.capability));
        let tree = encode_tree(&self.names, &files, &routes);
        pack(&tree, &files)
    }

    /// The same six components, with one route removed: `objects` still needs a
    /// block device from `blk` and the topology hands it none.
    fn module_with_an_unrouted_need(&self) -> Vec<u8> {
        let files: Vec<Vec<u8>> = self.names.iter().map(|name| component(name)).collect();
        let objects = self.index_of("objects");
        let routes: Vec<Route> =
            self.routes.iter().copied().filter(|route| route.component != objects).collect();
        let tree = encode_tree(&self.names, &files, &routes);
        pack(&tree, &files)
    }

    fn root_of(&self, module: &[u8]) -> [u8; 32] {
        let read = f_abi::boot::Module::read(module).expect("a module this file packed");
        let tree = f_generation::Tree::check(read.tree()).expect("a tree this file encoded");
        f_generation::fold::root(&tree)
    }
}

/// What each component needs from a sibling, as `(capability, sibling)`.
///
/// The other side of the routes, and it has to be here rather than implied: the
/// assembler checks the topology's routes against the manifests in both
/// directions, so a workload whose components asked for nothing would be
/// refused before a single component was started.
fn needs_of(name: &str) -> &'static [(&'static str, &'static str)] {
    match name {
        "objects" => &[("block", "blk")],
        "shell" => &[("log", "log"), ("objects", "objects")],
        _ => &[],
    }
}

/// One component file: a record and a one-page image.
///
/// The three drivers declare a part; the three that are not drivers declare
/// none, which is the common and correct shape.
fn component(name: &str) -> Vec<u8> {
    let part = match name {
        "blk" => Some(BLK_PART),
        "net" => Some(0x1041),
        "gpu" => Some(0x1050),
        _ => None,
    };
    build(name, part)
}

/// A component file for `name` that claims `part` — the second claimant.
fn claimant(name: &str, part: u16) -> Vec<u8> {
    build(name, Some(part))
}

fn build(name: &str, part: Option<u16>) -> Vec<u8> {
    let mut record = Record::EMPTY;
    record.name = name_bytes(name).expect("a name from this file");
    record.domain = domain::PRIVATE;
    record.restart = restart::ON_FAULT;
    record.class = class::SOFT;
    record.memory_bytes = 64 * 4096;
    record.backoff_first_ticks = 8;
    record.backoff_max_ticks = 64;
    record.max_restarts = 3;
    record.budget_window_ticks = 3_000;
    record.transfer = Declaration::RESTART_ONLY;
    record.capability[0] = Need {
        name: name_bytes("account").expect("a literal"),
        bytes: 32 * 4096,
        kind: CapType::Untyped.to_wire(),
        rights: rights::READ | rights::DERIVE | rights::REVOKE,
        route: route::SUPERVISOR,
        ..Need::EMPTY
    };
    record.capabilities = 1;
    for (capability, sibling) in needs_of(name) {
        record.capability[record.capabilities as usize] = Need {
            name: name_bytes(capability).expect("a literal"),
            sibling: name_bytes(sibling).expect("a literal"),
            kind: CapType::Endpoint.to_wire(),
            rights: rights::WRITE,
            route: route::SIBLING,
            ..Need::EMPTY
        };
        record.capabilities += 1;
    }
    if let Some(device) = part {
        record.binding[0] = Binding { vendor: VENDOR, device };
        record.devices = 1;
    }

    // An image whose bytes differ per component, so that two components cannot
    // accidentally share a content address and make the rendering look more
    // determined than it is.
    let image: Vec<u8> = name.bytes().cycle().take(64).collect();
    record.image_bytes = image.len() as u32;

    let mut file = vec![0u8; core::mem::size_of::<Record>() + image.len()];
    f_abi::manifest::encode(&record, &mut file).expect("a buffer this file sized");
    let at = core::mem::size_of::<Record>();
    file[at..].copy_from_slice(&image);
    Record::read_unaligned(&file).expect("a record this file built");
    file
}

/// The record tree, through `f_abi::store`'s own codec.
fn encode_tree(names: &[&str], files: &[Vec<u8>], routes: &[Route]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&Generation::to_bytes());
    out.extend_from_slice(
        &Leaf {
            kind: node::BYTES,
            hash: f_hash::sha256(b"a frame image this test does not build"),
            name: name_bytes("kernel").expect("a literal"),
        }
        .to_bytes(),
    );
    let head = Topology { members: names.len() as u16, routes: routes.len() as u16 };
    out.extend_from_slice(&head.to_bytes());
    for route in routes {
        out.extend_from_slice(&route.to_bytes());
    }
    for (name, file) in names.iter().zip(files) {
        out.extend_from_slice(
            &Leaf {
                kind: node::COMPONENT,
                hash: f_hash::sha256(file),
                name: name_bytes(name).expect("a name from this file"),
            }
            .to_bytes(),
        );
    }
    out
}

/// The boot module, through `f_abi::boot`'s own constants.
fn pack(tree: &[u8], files: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&f_abi::boot::MODULE_MAGIC.to_le_bytes());
    out.extend_from_slice(&(tree.len() as u32).to_le_bytes());
    out.extend_from_slice(&(files.len() as u32).to_le_bytes());
    for file in files {
        out.extend_from_slice(&(file.len() as u32).to_le_bytes());
    }
    out.extend_from_slice(tree);
    for file in files {
        out.extend_from_slice(file);
    }
    out
}

// ---------------------------------------------------------------------------
// Reporting.
// ---------------------------------------------------------------------------

fn state(assembly: &Assembly, index: u16) -> State {
    assembly.member(index).expect("a member this workload has").state
}

fn print_states(workload: &Workload, assembly: &Assembly) {
    for member in assembly.members() {
        let name = workload.names[member.index as usize];
        let at = member.bound.map_or_else(
            || "-".to_string(),
            |a| format!("{:02x}:{:02x}.{}", a.bus, a.device, a.function),
        );
        println!("    {name:<9} {:<10} device {at:<9} {:?}", member.state.label(), member.state);
    }
}

/// A refusal as a line, with the member named and the padded capability
/// trimmed. `Refusal` carries a NUL-padded name because it is a `no_std` type
/// with no formatter in reach; turning it into words is the caller's job, and
/// this is a caller.
fn why(workload: &Workload, refused: &Result<Assembly, f_assembler::Refusal>) -> String {
    match refused {
        Ok(_) => "no refusal at all".to_string(),
        Err(refusal) => {
            let named = |index: &u16, name: &[u8; NAME_MAX]| {
                let end = name.iter().position(|b| *b == 0).unwrap_or(NAME_MAX);
                format!(
                    "{} `{}`",
                    workload.names[*index as usize],
                    String::from_utf8_lossy(&name[..end])
                )
            };
            match refusal {
                f_assembler::Refusal::Unrouted(index, capability) => {
                    format!("{} — {}", refusal.message(), named(index, capability))
                }
                f_assembler::Refusal::Unsupplied(index, capability) => {
                    format!("{} — {}", refusal.message(), named(index, capability))
                }
                other => format!("{} ({other:?})", other.message()),
            }
        }
    }
}

fn check(what: &str, held: bool) -> usize {
    if held {
        println!("    [ok]   {what}");
        0
    } else {
        println!("    [FAIL] {what}");
        1
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

// The name width this file writes into a leaf is the record's, and a change to
// it would silently truncate every name in the workload.
const _: () = assert!(NAME_MAX == 32);
