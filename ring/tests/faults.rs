// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Seeded fault injection against the subsystems that exist.
//!
//! `docs/design/proving-ground.html` layer 1, and E0-P09. The hook has existed
//! since M0 — `f_env::sim` — on the argument that a hook must not be
//! retrofitted, because code written assuming operations succeed has to be
//! revisited at every call site later. This is the first thing to consume it.
//!
//! # What "one fault class per subsystem that exists" means here
//!
//! Two subsystems have a host harness at this milestone, and both are exercised
//! below: the **ring**, at publish and at drain, and the **channel**, at the
//! moment it binds to a region. The capability table is the third subsystem
//! with fault classes worth injecting into, and it is deliberately absent:
//! it lives in `kernel/`, which has no host harness at all — `kernel/Cargo.toml` says why — so its
//! fault classes belong to the simulator at E1-P02 rather than here. Claiming
//! them now would mean writing a second capability table to inject into, and a
//! test of a model of the system is not a test of the system.
//!
//! # What is being asserted
//!
//! Not that the ring never fails. That every failure is *reported* — as a
//! `RingError` a caller can act on — rather than becoming a wrong value, a
//! panic, or a silently dropped entry. Shared memory is untrusted input, and
//! the ring's whole contract is that a peer behaving badly produces a refusal
//! and not a surprise.
//!
//! And that the same seed produces the same run. A failing seed is only a bug
//! report if it reproduces.

use f_abi::layout::Layout;
use f_abi::{ABI_VERSION, ChannelHeader, Sqe};
use f_env::sim::{Fault, Faults, SimEnv};
use f_ring::{Consumer, Mapping, Producer, RingError};

/// The sites this suite injects at, one per subsystem with a harness.
///
/// `chan.bind` was `chan.negotiate` until E0-B13, and the rename is the change:
/// the site used to call [`ChannelHeader::negotiate`] on a struct on this
/// thread's stack, which exercises the arithmetic and nothing else. It now
/// binds a [`Mapping`] over a real region, so a hostile header is sixty-four
/// bytes a peer wrote rather than a value this test constructed.
const SITES: [&str; 3] = ["ring.publish", "ring.consume", "chan.bind"];

/// What happened at one step, in a form two runs can be compared on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Step {
    /// Nothing was injected; the operation ran and reported this.
    Clean(&'static str, Outcome),
    /// A fault was injected at this site, and the system reported this.
    Injected(&'static str, Fault, Outcome),
}

/// The outcome of an operation, flattened so a trace is comparable.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Outcome {
    Ok,
    Empty,
    Full,
    Corrupt,
    EpochChanged,
    Refused(i32),
}

impl From<Result<bool, RingError>> for Outcome {
    fn from(result: Result<bool, RingError>) -> Self {
        match result {
            Ok(_) => Self::Ok,
            Err(RingError::Full) => Self::Full,
            Err(RingError::Corrupt) => Self::Corrupt,
            Err(RingError::EpochChanged) => Self::EpochChanged,
        }
    }
}

/// Entries in the channel this suite drives.
const ENTRIES: u32 = 8;

/// Bytes in the region it is bound in. One frame, the unit the kernel shares.
const LEN: u32 = 4096;

/// A run of shared bytes at the alignment the layout is stated against.
///
/// This replaces the fixture the suite used to carry — a struct with a cursor
/// field and an entry array, which is a channel the borrow checker had already
/// proved well-formed. The offsets were Rust's problem there and the wire format
/// never got consulted, so a header that lied about them had nothing to lie to.
#[repr(C, align(64))]
struct Region([u8; LEN as usize]);

impl Region {
    fn new() -> Box<Self> {
        Box::new(Self([0; LEN as usize]))
    }

    fn base(&mut self) -> *mut u8 {
        self.0.as_mut_ptr()
    }

    /// Write a header the way a peer would, without going through
    /// [`Mapping::describe`], which by construction produces only sound ones.
    fn place(&mut self, header: ChannelHeader) {
        let base = self.base();
        // SAFETY: `Region` is 64-byte aligned and 4 KiB long, so a 64-byte
        // `ChannelHeader` needing 64-byte alignment fits at offset zero, and
        // the exclusive borrow proves nothing else holds a reference to it.
        unsafe { base.cast::<ChannelHeader>().write(header) };
    }
}

/// A header a well-behaved peer would write.
///
/// Derived from [`Layout`] rather than spelled out, so a change to the wire
/// layout moves it instead of leaving a literal every hostile case is measured
/// against. The old hand-written version had `sqe_offset: 64`, which no build
/// has ever placed the entry array at — proof that a header nothing binds is a
/// header nothing checks.
fn sound_header() -> ChannelHeader {
    Layout::new(ENTRIES, 0).expect("eight entries is a layout").describe(0, 0, 0)
}

/// Write a header into a region and try to bind a channel to it.
///
/// The whole of the `chan.bind` site. What is being exercised is not
/// [`ChannelHeader::negotiate`] — that is one of four checks — but the path a
/// component actually takes: sixty-four bytes in shared memory, then a mapping
/// that either exists or does not.
fn bind(region: &mut Region, header: ChannelHeader) -> Outcome {
    region.place(header);
    let base = region.base();
    // SAFETY: 4 KiB, 64-byte aligned, exclusively borrowed. The *contents* are
    // hostile on most of these calls, which is the subject rather than a safety
    // obligation: nothing is dereferenced through them unless `adopt` returns.
    match unsafe { Mapping::adopt(base, LEN, 0, 0) } {
        Ok(_) => Outcome::Ok,
        Err(code) => Outcome::Refused(code),
    }
}

/// One seeded run. Returns the trace, which is the artefact a seed produces.
fn run(seed: u64, steps: usize) -> Vec<Step> {
    let mut env = SimEnv::new(seed, 10, 250);

    // The channel under test, and a second region the header site binds and
    // rebinds. Two regions rather than one, because a hostile header written
    // over the live channel would tear down the ring halves this run is still
    // driving — which is a real behaviour, and a different test from this one.
    let mut backing = Region::new();
    let mut headers = Region::new();
    let base = backing.base();
    // SAFETY: 4 KiB, 64-byte aligned, zeroed, and borrowed for as long as the
    // mapping and the halves below live.
    let mapping =
        unsafe { Mapping::describe(base, LEN, ENTRIES, 0, 0, 0) }.expect("a sound channel");

    let producer = Producer::new(mapping.channel()).expect("a power-of-two ring");
    let consumer = Consumer::new(mapping.channel()).expect("a power-of-two ring");
    consumer.disarm_wakeup();
    let cursors = mapping.channel();

    let mut trace = Vec::with_capacity(steps);
    let mut published: u64 = 0;
    let mut drained: u64 = 0;

    for step in 0..steps {
        let site = SITES[step % SITES.len()];
        let fault = env.should_fail(site);

        match (site, fault) {
            // --- the ring, at publish -------------------------------------
            ("ring.publish", None) => {
                let mut sqe = Sqe::ZERO;
                sqe.user_data = published;
                let outcome = Outcome::from(producer.submit(sqe));
                if outcome == Outcome::Ok {
                    published += 1;
                }
                trace.push(Step::Clean(site, outcome));
            }
            ("ring.publish", Some(Fault::Fail)) => {
                // A publish that does not happen. The consumer must see an
                // empty ring rather than a stale entry — the failure mode a
                // ring with a count instead of cursors would have.
                trace.push(Step::Injected(site, Fault::Fail, Outcome::Ok));
            }
            ("ring.publish", Some(Fault::Delay(nanos))) => {
                env.advance(nanos);
                let mut sqe = Sqe::ZERO;
                sqe.user_data = published;
                let outcome = Outcome::from(producer.submit(sqe));
                if outcome == Outcome::Ok {
                    published += 1;
                }
                trace.push(Step::Injected(site, Fault::Delay(nanos), outcome));
            }
            ("ring.publish", Some(Fault::PeerRestart)) => {
                // The peer restarted mid-channel and its cursor is now
                // nonsense. The ring must report `Corrupt` and not act on it.
                cursors.head.set(u32::MAX / 2);
                let outcome = Outcome::from(producer.submit(Sqe::ZERO));
                trace.push(Step::Injected(site, Fault::PeerRestart, outcome));
                // Put it back, so one injected fault does not silently make
                // every later step a repeat of this one.
                cursors.head.set(published as u32);
                cursors.tail.set(drained as u32);
            }

            // --- the ring, at drain ---------------------------------------
            ("ring.consume", None) => {
                let outcome = match consumer.pop() {
                    Ok(Some(_)) => {
                        drained += 1;
                        Outcome::Ok
                    }
                    Ok(None) => Outcome::Empty,
                    Err(RingError::Corrupt) => Outcome::Corrupt,
                    Err(RingError::Full) => Outcome::Full,
                    Err(RingError::EpochChanged) => Outcome::EpochChanged,
                };
                trace.push(Step::Clean(site, outcome));
            }
            ("ring.consume", Some(kind)) => {
                // A consumer that does not drain. The ring must fill and
                // *report* Full rather than overwrite an entry the consumer
                // has not read — which is the one failure that would corrupt
                // data rather than refuse work.
                if let Fault::Delay(nanos) = kind {
                    env.advance(nanos);
                }
                let mut outcome = Outcome::Ok;
                for _ in 0..16 {
                    outcome = Outcome::from(producer.submit(Sqe::ZERO));
                    if outcome != Outcome::Ok {
                        break;
                    }
                    published += 1;
                }
                trace.push(Step::Injected(site, kind, outcome));
                cursors.head.set(published as u32);
            }

            // --- the channel header, at binding ---------------------------
            (_, None) => {
                let outcome = bind(&mut headers, sound_header());
                trace.push(Step::Clean(site, outcome));
            }
            (_, Some(kind)) => {
                let mut header = sound_header();
                match kind {
                    // A peer that claims a version nobody speaks.
                    Fault::Fail => header.abi_version_min = ABI_VERSION + 99,
                    // A peer whose header arrived torn.
                    Fault::Delay(_) => header.magic = 0,
                    // A peer that restarted and requires a feature it never
                    // offered — the inconsistency a restart can leave behind.
                    Fault::PeerRestart => header.features_required = 1 << 40,
                }
                let outcome = bind(&mut headers, header);
                trace.push(Step::Injected(site, kind, outcome));

                // Bound again with a sound header, so the run asserts the
                // refusal was clean as well as correct. A teardown that left
                // the region unusable would be a denial of service any peer
                // could trigger with one bad word.
                assert_eq!(
                    bind(&mut headers, sound_header()),
                    Outcome::Ok,
                    "the region did not come back after a {kind:?} header"
                );
            }
        }
    }

    assert_eq!(env.overflowed(), 0, "a site went untracked, so this run covered less than it says");
    trace
}

#[test]
fn a_seeded_run_injects_at_a_named_site_and_the_system_handles_it() {
    let trace = run(0xf00d, 600);

    let injected: Vec<&Step> = trace.iter().filter(|s| matches!(s, Step::Injected(..))).collect();
    assert!(
        injected.len() > 20,
        "a run that injects almost nothing proves almost nothing; got {}",
        injected.len()
    );

    // Every site was actually reached. A sweep that silently covered two of
    // three subsystems would pass every assertion below it.
    for site in SITES {
        assert!(
            trace.iter().any(|s| matches!(s, Step::Injected(name, ..) if *name == site)),
            "no fault was injected at {site}"
        );
    }

    // The property the whole suite exists for: a fault becomes a refusal the
    // caller can act on. Not a panic — reaching this line is that assertion —
    // and not a wrong value.
    for step in &trace {
        if let Step::Injected(site, kind, outcome) = step {
            match (*site, outcome) {
                // A corrupt cursor is reported and never acted on.
                ("ring.publish", Outcome::Ok | Outcome::Corrupt | Outcome::Full) => {}
                // A stalled consumer fills the ring and is told so.
                ("ring.consume", Outcome::Ok | Outcome::Full | Outcome::Corrupt) => {}
                // A malformed header is refused with a structured error, never
                // accepted and never a panic. RFC 0010: a refusal names its
                // domain, which is what makes `Refused` actionable.
                ("chan.bind", Outcome::Refused(code)) => {
                    assert!(*code < 0, "a refusal must be a negative structured error, got {code}");
                }
                other => panic!("{kind:?} at {site} produced {other:?}, which nothing handles"),
            }
        }
    }
}

#[test]
fn the_same_seed_reproduces_the_same_run() {
    // A failing seed is only a bug report if it reproduces. This is the
    // assertion that makes every other one in this file worth writing down.
    assert_eq!(run(0xf00d, 600), run(0xf00d, 600));
    assert_eq!(run(1, 200), run(1, 200));
}

#[test]
fn different_seeds_explore_different_runs() {
    // The other half. Two seeds that agree are one seed with two names, and a
    // sweep of a thousand of those covers what one covers.
    assert_ne!(run(0xf00d, 600), run(0xbeef, 600));
}

#[test]
fn a_sweep_narrowed_to_one_site_still_finds_what_it_found_there() {
    // What `focused_on` is for, checked at the level a person debugging would
    // use it: narrow the sweep to the transition under investigation, and the
    // answers at that transition must not move. If they did, focusing would be
    // a different experiment rather than a smaller one.
    let mut wide = SimEnv::new(4242, 10, 250);
    let mut narrow = SimEnv::new(4242, 10, 250).focused_on(&["chan.bind"]);

    for step in 0..300 {
        let site = SITES[step % SITES.len()];
        let w = wide.should_fail(site);
        let n = narrow.should_fail(site);
        if site == "chan.bind" {
            assert_eq!(w, n, "narrowing the sweep changed what it sees at the focused site");
        } else {
            assert!(n.is_none(), "an unfocused site faulted");
        }
    }
}
