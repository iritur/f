// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Workload for claims `unmap-churn` and `unmap-churn-cost`.
//!
//! The two churn sources the datapath actually has, driven through the real
//! registration table, at the rate the datapath produces them:
//!
//! **A client cycling its registered buffers.** RFC 0024 says the memory is the
//! client's and it is entitled to take it back, so a client that reuses a
//! region pays a registration and a retirement per cycle. `Table::register`
//! then `Table::unregister`, over and over.
//!
//! **A driver restart.** RFC 0008 revokes a dead component's buffer sets
//! whether or not anything is convenient, and `Table::retire_all` is that call:
//! every set the dead instance held, retired in one pass, one `Domains::unmap`
//! per set.
//!
//! # Which half of the number this is
//!
//! The counts, and the counts alone, are the same on any machine — so the boot
//! takes them and `cargo xtask churn` requires this workload to agree with it.
//! What this adds is the shape of the *time*: how long one unmap request takes
//! on the service's side of the ring, recorded per observation rather than
//! averaged, into a histogram [`Sample`] will decline to publish here. That
//! refusal is `bench/src/lib.rs` working: a p99 in nanoseconds taken in a
//! container is not a number anybody may quote, and `claims/0015` is `pending`
//! on the machine `E0-D10` owes for exactly that reason.
//!
//! # What this cannot establish, and it is most of the cost
//!
//! The frame's unmap is a page-table walk followed by a global invalidation of
//! the remapping unit — a register write and a spin on the unit clearing the
//! request bit, twice. **None of that is here.** [`Modelled`] walks the pages
//! and touches no hardware, because there is no hardware on the host to touch,
//! so what this times is the registry's own arithmetic and a loop: the slot
//! lookup, the generation retirement, the in-flight word, and one iteration per
//! page. That is a real cost and it is the smaller one.
//!
//! So the honest division is this. **How many** invalidations a churn costs is
//! the boot's number and it is exact — `cargo xtask churn`, `claims/0014`.
//! **How much each one costs** is the boot's number too: `kernel/src/churn.rs`
//! now times a thousand and twenty-four unmap requests through the shipped path
//! on the machine's own remapping unit, and `claims/0015` names that as its
//! workload. What is left here is the half above the hardware, measured where
//! there is no hardware, and it is kept for two reasons rather than out of
//! sentiment: it is the only place the registry's own arithmetic can be timed
//! without a walk and an invalidation on top of it, and `cargo xtask churn`
//! requires this and the boot to report the same counts — which is what stops
//! the two becoming separate experiments sharing one claim.

use std::hint::black_box;
use std::path::Path;
use std::time::Instant;

use f_bench::Sample;
use f_ring::registry::{Domains, Refusal, Table};

/// Registration slots the service holds. A power of two, because a slot index
/// is masked and not clamped.
const SLOTS: usize = 16;

/// Pages in one registered buffer set.
///
/// Eight, and the same eight the boot uses — `kernel/src/churn.rs`'s
/// `SET_PAGES` — because `cargo xtask churn` requires the two sides to report
/// the same page count and a workload that measured a different geometry would
/// be a second experiment wearing the first one's name.
const SET_PAGES: u32 = 8;

/// Bytes in a set: one page per buffer.
const REGION: u32 = SET_PAGES * 4096;

/// Buffers in a set. One per page, so a name is a page.
const BUFFERS: u32 = 8;

/// Sets a component holds when it dies. The restart half's width.
const SETS: usize = 8;

/// Cycles the steady half performs per round.
const CYCLES: usize = 32;

/// Rounds, so the histogram has a tail worth reading.
///
/// The boot does one round because a boot is evidence and one is enough of it;
/// this does many because a percentile over forty observations is not a
/// percentile. The counts this reports are per round, so the two agree.
const ROUNDS: usize = 10_000;

/// Where a set's memory would be, if any of this were memory.
const BASE: u64 = 0x1_0000_0000;

/// The frame's unmap, with the hardware taken out.
///
/// It walks the pages, because that part is arithmetic and is the same
/// arithmetic on any machine. It invalidates nothing, because there is nothing
/// here to invalidate — and that absence is the whole of what this workload
/// cannot establish, stated in the module documentation rather than left for a
/// reader to infer from a fast number.
///
/// The counters are here rather than derived afterwards for the reason
/// `vtd::Unit`'s are: a count computed from a rule somebody wrote down stops
/// being true the day the rule changes and nothing notices.
struct Modelled {
    /// Sets given a translation. Unit: sets.
    mapped: u64,
    /// Unmap requests. Unit: requests.
    requests: u64,
    /// Leaf entries a request would have cleared. Unit: pages.
    pages: u64,
}

impl Domains for Modelled {
    fn map(&mut self, _cap: u32, _len: u32) -> Result<u64, Refusal> {
        // A distinct address per set, because two live sets at one address
        // would make `Grants`-style bookkeeping agree with itself about a
        // domain no run ever had. Wrapping is unreachable at this scale and is
        // written as `wrapping` rather than `+` because a benchmark that
        // panicked at its far end would be a workload with a length limit
        // nobody stated.
        let at = BASE.wrapping_add(self.mapped.wrapping_mul(u64::from(REGION)));
        self.mapped = self.mapped.wrapping_add(1);
        Ok(at)
    }

    fn unmap(&mut self, _cap: u32, address: u64, len: u32) {
        self.requests = self.requests.saturating_add(1);
        let pages = u64::from(len) / 4096;
        for page in 0..pages {
            // The walk, and nothing else. `black_box` so that a loop whose
            // result is discarded is not a loop the optimiser removes — which
            // would leave this timing the call overhead and reporting it as the
            // cost of an unmap.
            black_box(address.wrapping_add(page * 4096));
        }
        self.pages = self.pages.saturating_add(pages);
    }
}

fn main() {
    let mut sample = Sample::new("unmap-churn-cost");
    let mut frame = Modelled { mapped: 0, requests: 0, pages: 0 };
    let mut table = Table::<SLOTS>::new();

    // Warm the cache and the branch predictors. A cold path would report a
    // number this claim does not make — the same reason `ring_submit` and
    // `buffer_register` both warm.
    for _ in 0..1_000 {
        let set = table.register(0, REGION, BUFFERS, &mut frame).expect("a region that divides");
        table.unregister(set, &mut frame).expect("a set this loop just made");
    }
    frame = Modelled { mapped: frame.mapped, requests: 0, pages: 0 };

    for _ in 0..ROUNDS {
        // Half one: a client cycling its buffers.
        for _ in 0..CYCLES {
            let set =
                table.register(0, REGION, BUFFERS, &mut frame).expect("a region that divides");
            // The registration is outside the measurement and the retirement is
            // inside it, deliberately: `claims/0004` is the cost of registering
            // and this is the cost of taking it back. Averaging the two would
            // make both claims unreadable, which is the mistake
            // `buffer_register` records having avoided one claim over.
            let start = Instant::now();
            black_box(table.unregister(set, &mut frame)).expect("a set this loop just made");
            sample.latency.record(start.elapsed().as_nanos() as u64);
        }

        // Half two: a driver restart, every live set retired in one pass.
        for _ in 0..SETS {
            table.register(0, REGION, BUFFERS, &mut frame).expect("a table with room");
        }
        let start = Instant::now();
        let retired = black_box(table.retire_all(&mut frame));
        let elapsed = start.elapsed().as_nanos() as u64;
        assert_eq!(retired, SETS, "a restart must retire every live set");
        // Per request, not per sweep, so that one observation means the same
        // thing in both halves. A sweep recorded whole would put a number eight
        // times the size of the others into the same histogram and move every
        // percentile above p95 — which is exactly the summary this harness
        // exists to refuse.
        for _ in 0..SETS {
            sample.latency.record(elapsed / SETS as u64);
        }
    }

    let per_round = (CYCLES + SETS) as u64;
    sample.report();
    println!();
    println!("churn   {ROUNDS} round(s) of {CYCLES} cycle(s) and one restart of {SETS} set(s)");
    // The line `cargo xtask churn` reads, and it is written per round so that
    // it is comparable with the boot's — which does exactly one round. A
    // workload that reported its totals would be a number nothing could check
    // against the frame's.
    println!(
        "counts  {} unmap request(s) and {} page(s) per round, {} page(s) per set",
        frame.requests / ROUNDS as u64,
        frame.pages / ROUNDS as u64,
        frame.pages / frame.requests.max(1)
    );
    assert_eq!(
        frame.requests,
        per_round * ROUNDS as u64,
        "every registration must be retired exactly once, or the counts this claim \
         publishes are about a different workload"
    );

    // Beside the claim it belongs to rather than in the build directory, for
    // `ring_submit`'s reason: the distribution is the artefact and a
    // `cargo clean` should not be able to delete a measurement.
    match sample.persist(Path::new("claims")) {
        Ok(path) => println!("\nfull distribution written to {}", path.display()),
        Err(e) => println!("\ncould not write the distribution: {e}"),
    }

    println!();
    println!("note: the walk without the hardware. What an unmap costs on a machine");
    println!("      with a remapping unit is two register round trips per invalidation");
    println!("      and this touches none — see the module docs. Both halves of the");
    println!("      number are the boot's: claims/0014 counts the invalidations and");
    println!("      gates, claims/0015 times the requests through the real unit and is");
    println!("      `pending` until a machine may publish a percentile. This times the");
    println!("      arithmetic above the unit, and agrees with the boot about the counts.");
}
