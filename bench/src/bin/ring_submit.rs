// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Workload for claim `ring-submit-latency`.
//!
//! Measures a single ring submission with a warm cache, batched, against the
//! target in `claims/0001-ring-submit-latency.toml`.
//!
//! # What this cannot yet establish
//!
//! It runs on the host, against host memory, with a host allocator and a host
//! scheduler. That measures the protocol's instruction path and nothing about
//! the system — no user interrupts, no registered buffers, no deadline class.
//! The claim stays `pending` until it runs on the kernel target, and this
//! binary exists now so the workload is version-controlled alongside the claim
//! rather than written the day someone wants a number.

use std::cell::UnsafeCell;
use std::hint::black_box;
use std::sync::atomic::AtomicU32;
use std::time::Instant;

use f_abi::Sqe;
use f_bench::Sample;
use f_ring::{Channel, Consumer, Cursor, Producer};

const RING: usize = 256;
const BATCH: usize = 32;
const ITERATIONS: usize = 1_000_000;

fn main() {
    let head = Cursor::new();
    let tail = Cursor::new();
    let flags = AtomicU32::new(0);
    let entries: Vec<UnsafeCell<Sqe>> =
        (0..RING).map(|_| UnsafeCell::new(Sqe::ZERO)).collect();

    let chan = || Channel { head: &head, tail: &tail, flags: &flags, entries: &entries };
    let producer = Producer::new(chan()).expect("ring size is a power of two");
    let consumer = Consumer::new(chan()).expect("ring size is a power of two");

    // A draining consumer, so the doorbell is suppressed and this measures the
    // submission path rather than a wakeup.
    consumer.disarm_wakeup();

    let mut sample = Sample::new("ring-submit-latency");

    // Warm the cache and the branch predictors. Measuring a cold path would
    // report a number this claim does not make.
    for _ in 0..BATCH * 64 {
        let _ = producer.submit(Sqe::ZERO);
        let _ = consumer.pop();
    }

    let batches = ITERATIONS / BATCH;
    for i in 0..batches {
        let start = Instant::now();
        for slot in 0..BATCH {
            let mut sqe = Sqe::ZERO;
            sqe.user_data = (i * BATCH + slot) as u64;
            black_box(producer.submit(black_box(sqe))).expect("ring must not fill");
        }
        let elapsed = start.elapsed().as_nanos() as u64;

        // Per-operation cost, recorded per observation rather than averaged
        // across the run. See the harness docs.
        sample.latency.record(elapsed / BATCH as u64);

        for _ in 0..BATCH {
            black_box(consumer.pop()).expect("ring must not be corrupt");
        }
    }

    sample.report();
    println!();
    println!("note: host measurement only — see the module docs for what this");
    println!("      does not establish. Claim remains `pending`.");
}
