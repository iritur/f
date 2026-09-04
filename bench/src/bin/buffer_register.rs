// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Workload for claim `buffer-registration-cost`.
//!
//! Two questions, because `claims/0004-buffer-registration-cost.toml` asks two.
//!
//! **What does a registration cost?** One full round trip: the client writes
//! the entry, the service pops it, fills a slot, asks the frame for a
//! translation, and answers with a set id the client reads back. That is the
//! per-*set* cost `docs/design/fast-path.html` section 13 says disappears on
//! capable hardware, and it is the primary metric.
//!
//! **What does naming a buffer cost on each path?** A registered name is a
//! bounds check, a mask and one word of in-flight bitmap; a virtual name is a
//! page walk the IOMMU does. RFC 0024's reversal condition is that these two
//! turn out indistinguishable — at which point the naming type parameter is
//! carrying a distinction nothing pays for and one path is the design. So the
//! two are drawn side by side, from one loop shape, and the file the registry
//! ingests carries both.
//!
//! # What this cannot yet establish
//!
//! Almost all of the interesting half. It runs on the host, against host
//! memory, with no IOMMU under either path: [`Domains::map`] hands back an
//! address it was given and [`PageWalk::reaches`] answers a range comparison.
//! `E1-B01` is where both acquire hardware, and until then the registered
//! path's number is *this side's table arithmetic plus a ring round trip* and
//! the virtual path's number is a range comparison standing where a page walk
//! will be. The second of those will get slower and the first will not.
//!
//! So the number that is nearly honest today is the *shape* of the first
//! question — a registration is a round trip, and a round trip is amortised
//! over however many buffers the set holds — and the number that is not honest
//! at all is the ratio between the two paths. The claim is `pending` for
//! exactly that reason, and it says so.

use std::cell::UnsafeCell;
use std::hint::black_box;
use std::path::Path;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use f_abi::buf::{Name, SetId};
use f_abi::{ABI_VERSION, Cqe, Negotiated, Sqe, feature};
use f_bench::{Histogram, Sample};
use f_ring::registry::{
    Domains, PageWalk, Refusal, Registered, SharedVirtual, Table, Transport, registration,
    unregistration,
};
use f_ring::{Channel, Collector, Completions, Consumer, Cursor, Poster, Producer};

/// Entries in each ring. The same as `ring_submit`'s, so that the round trip
/// this measures is the round trip that claim measured the submission half of.
const RING: usize = 256;

/// Registration slots the service holds. A power of two, because the slot index
/// a peer writes is masked and not clamped — `f_ring::registry` says why.
const SLOTS: usize = 64;

/// Buffers per set. Eight is a plausible device queue depth and, more to the
/// point, it is the number the registration cost is divided by when anybody
/// asks what a *buffer* cost: a set of one would make registration look eight
/// times as expensive as it is.
const BUFFERS: u32 = 8;

/// Bytes per set. Thirty-two kilobytes into eight four-kilobyte buffers, which
/// is one page each — the grain an IOMMU will actually work in.
const REGION: u32 = 32 * 1024;

/// Registrations measured.
///
/// More than one slot's worth on purpose. A slot's generation retires at
/// `SetId::RETIRED_GENERATION` rather than wrapping, so 65 534 registrations
/// use one slot up and the 65 535th moves to the next — which means this
/// workload walks past that boundary twice and would notice a table that
/// stopped there instead of stepping over. The `SLOTS` below is what pays for
/// it: sixty-four slots is four million registrations before this run could
/// exhaust the table, and the run makes a hundred thousand.
const ITERATIONS: usize = 100_000;

/// Buffer names resolved, on each path.
const RESOLUTIONS: usize = 1_000_000;

/// A frame that hands out one address.
///
/// Standing in for `E1-B01`. It does the bookkeeping a real domain would need
/// to be asked for and none of the work, which is the honest limit of a host
/// measurement and is stated in the module docs rather than in a footnote.
struct Pinned {
    base: u64,
    mapped: u64,
}

impl Domains for Pinned {
    fn map(&mut self, _cap: u32, _len: u32) -> Result<u64, Refusal> {
        self.mapped += 1;
        Ok(self.base)
    }

    fn unmap(&mut self, _cap: u32, _address: u64, _len: u32) {}
}

/// An IOMMU that reaches one region.
struct Reaches {
    base: u64,
    len: u32,
}

impl PageWalk for Reaches {
    fn reaches(&self, address: u64, len: u32) -> bool {
        let end = self.base + u64::from(self.len);
        address >= self.base && address.checked_add(u64::from(len)).is_some_and(|last| last <= end)
    }
}

fn main() {
    let head = Cursor::new();
    let tail = Cursor::new();
    let flags = AtomicU32::new(0);
    let entries: Vec<UnsafeCell<Sqe>> = (0..RING).map(|_| UnsafeCell::new(Sqe::ZERO)).collect();
    let index: Vec<AtomicU32> = (0..RING).map(|_| AtomicU32::new(0)).collect();
    let cq_head = Cursor::new();
    let cq_tail = Cursor::new();
    let slots: Vec<UnsafeCell<Cqe>> = (0..RING).map(|_| UnsafeCell::new(Cqe::ZERO)).collect();

    let chan =
        || Channel { head: &head, tail: &tail, flags: &flags, index: &index, entries: &entries };
    let cq = || Completions { head: &cq_head, tail: &cq_tail, slots: &slots };
    let producer = Producer::new(chan()).expect("ring size is a power of two");
    let consumer = Consumer::new(chan()).expect("ring size is a power of two");
    let poster = Poster::new(cq()).expect("ring size is a power of two");
    let collector = Collector::new(cq()).expect("ring size is a power of two");

    // A draining consumer, so the doorbell is suppressed and this measures the
    // registration rather than a wakeup — the same reason `ring_submit` does it.
    consumer.disarm_wakeup();

    let mut table = Table::<SLOTS>::new();
    let mut frame = Pinned { base: 0x1_0000, mapped: 0 };
    let mut sample = Sample::new("buffer-registration-cost");

    // Warm the cache and the branch predictors. A cold path would report a
    // number this claim does not make.
    for i in 0..1_000u64 {
        let set = round_trip(&producer, &consumer, &poster, &collector, &mut table, &mut frame, i);
        retire(&producer, &consumer, &poster, &collector, &mut table, &mut frame, i, set);
    }

    // The warm-up asked the frame for a thousand translations and they are not
    // the measurement's. Counted from here.
    frame.mapped = 0;

    for i in 0..ITERATIONS {
        let token = i as u64;
        let start = Instant::now();
        let set =
            round_trip(&producer, &consumer, &poster, &collector, &mut table, &mut frame, token);
        let elapsed = start.elapsed().as_nanos() as u64;
        sample.latency.record(elapsed);

        // Retiring is not registering, and a claim about the cost of one must
        // not quietly average the two. Outside the measurement, deliberately.
        retire(&producer, &consumer, &poster, &collector, &mut table, &mut frame, token, set);
    }

    let (registered, virtual_memory) = resolve_both_paths(&mut table, &mut frame);

    sample.report();
    println!("sets    {ITERATIONS} registered, {} translations asked of the frame", frame.mapped);
    println!("slots   {} used up and never reissued, of {SLOTS}", table.retired());

    println!();
    println!("naming one buffer, registered path — a bounds check, a mask, one bit");
    print!("{}", registered.render());
    println!();
    println!("naming one buffer, shared-virtual-memory path — a page walk, on hardware");
    print!("{}", virtual_memory.render());

    // Beside the claim it belongs to rather than in the build directory, for
    // the reason `ring_submit` gives: the distribution is the artefact and a
    // `cargo clean` should not be able to delete a measurement.
    match sample.persist(Path::new("claims")) {
        Ok(path) => println!("\nfull distribution written to {}", path.display()),
        Err(e) => println!("\ncould not write the distribution: {e}"),
    }

    // The two secondary distributions, in the same form and beside the same
    // claim. Written only where the primary was: a per-path comparison taken on
    // a machine that may not record a latency is not a comparison anybody may
    // quote either, and writing it would leave exactly the quotable number the
    // environment gate exists to refuse.
    if sample.environment.records() {
        let path = Path::new("claims").join("buffer-registration-cost.paths.local.jsonl");
        let mut out = registered.to_jsonl("buffer-registration-cost/resolve-registered");
        out.push_str(&virtual_memory.to_jsonl("buffer-registration-cost/resolve-virtual"));
        match std::fs::write(&path, out) {
            Ok(()) => println!("per-path distributions written to {}", path.display()),
            Err(e) => println!("could not write the per-path distributions: {e}"),
        }
    }

    println!();
    println!("note: host measurement only, and no IOMMU under either path — see");
    println!("      the module docs for what this does not establish. Claim");
    println!("      remains `pending`.");
}

/// One registration, from the client's entry to the client's set id.
fn round_trip(
    producer: &Producer<'_>,
    consumer: &Consumer<'_>,
    poster: &Poster<'_>,
    collector: &Collector<'_>,
    table: &mut Table<SLOTS>,
    frame: &mut Pinned,
    token: u64,
) -> SetId {
    producer
        .submit(black_box(registration(token, 0, REGION, BUFFERS)))
        .expect("the ring must not fill");
    let entry = consumer.pop().expect("the cursors must stay sane").expect("one entry");
    let answer = table.execute(&entry, frame, 0);
    poster.post(answer).expect("room to answer");
    let cqe = collector.take().expect("the cursors must stay sane").expect("one completion");
    SetId::from_completion(&cqe).expect("a registration that was not refused")
}

/// And back again, so the next iteration has a slot.
#[allow(clippy::too_many_arguments)]
fn retire(
    producer: &Producer<'_>,
    consumer: &Consumer<'_>,
    poster: &Poster<'_>,
    collector: &Collector<'_>,
    table: &mut Table<SLOTS>,
    frame: &mut Pinned,
    token: u64,
    set: SetId,
) {
    producer.submit(unregistration(token, set)).expect("the ring must not fill");
    let entry = consumer.pop().expect("the cursors must stay sane").expect("one entry");
    let answer = table.execute(&entry, frame, 0);
    poster.post(answer).expect("room to answer");
    let _ = collector.take().expect("the cursors must stay sane").expect("one completion");
}

/// Name one buffer, `RESOLUTIONS` times, on each path.
///
/// One loop shape and one histogram type, so the comparison is a comparison of
/// the thing that differs. RFC 0024 put the two paths behind one naming
/// parameter for the same reason: `E1-B10`'s measurement has to compare one
/// thing.
fn resolve_both_paths(table: &mut Table<SLOTS>, frame: &mut Pinned) -> (Histogram, Histogram) {
    let stride = u64::from(REGION / BUFFERS);
    let agreed = Negotiated { version: ABI_VERSION, features: feature::SHARED_VIRTUAL_MEMORY };

    let set = table.register(0, REGION, BUFFERS, frame).expect("a region that divides");
    let mut service = Registered::bind(agreed, table).expect("the registered path needs nothing");
    let mut registered = Histogram::new();
    for i in 0..RESOLUTIONS {
        let index = (i as u32) % BUFFERS;
        let name = Name::Registered { set, index };
        let start = Instant::now();
        let reach = black_box(service.resolve(name, 64)).expect("a buffer of a live set");
        service.release(name).expect("and back again");
        registered.record(start.elapsed().as_nanos() as u64);
        debug_assert_eq!(reach.len, 64);
    }

    let walk = Reaches { base: frame.base, len: REGION };
    let mut service = SharedVirtual::bind(agreed, &walk).expect("the feature was negotiated");
    let mut virtual_memory = Histogram::new();
    for i in 0..RESOLUTIONS {
        let index = u64::from((i as u32) % BUFFERS);
        let name = Name::Virtual { address: frame.base + index * stride };
        let start = Instant::now();
        let reach = black_box(service.resolve(name, 64)).expect("inside the region");
        service.release(name).expect("nothing to give back, and it says so");
        virtual_memory.record(start.elapsed().as_nanos() as u64);
        debug_assert_eq!(reach.len, 64);
    }

    (registered, virtual_memory)
}
