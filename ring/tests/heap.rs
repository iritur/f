// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The heap's arithmetic, over host memory.
//!
//! The allocator a component gets is the one these tests drive: `Heap::over`
//! takes any base, so the identical code runs against an arena here and against
//! the region the frame granted there. That is the reason the static holds an
//! address rather than being a zero-sized type — a heap whose base was a
//! compile-time constant could not be tested without a boot, and a test that
//! needed a boot would be a test nobody ran while writing this.
//!
//! # The control
//!
//! `a_free_list_is_a_function_of_the_live_set` is the one that matters, and it
//! is a control rather than a check: it does not assert that the allocator
//! works, it asserts that two *different histories* reaching one live set leave
//! one free list. `user/virtio-net/src/driver.rs` says a free list is an
//! allocation order and an allocation order a component chose is a place a
//! seeded run stops reproducing. Address-ordered first fit with immediate
//! coalescing is the form that answers it, and this is the test that would go
//! red if somebody made the list insertion-ordered for speed.
//!
//! # Why the two helpers exist
//!
//! `take` and `give` state `GlobalAlloc`'s obligation once each, rather than
//! repeating it at forty call sites. `undocumented_unsafe_blocks` is denied in
//! this workspace, and a safety comment copied forty times is a safety comment
//! nobody reads by the fourth.

use core::alloc::{GlobalAlloc, Layout};

use f_ring::heap::{self, GRANULARITY, HEADER_BYTES, Heap};

/// Sixteen-aligned backing store, because every block is `GRANULARITY`-aligned
/// from the base and a misaligned base would make that false everywhere.
#[repr(align(16))]
struct Arena {
    bytes: [u8; 4096],
}

impl Arena {
    fn new() -> Box<Self> {
        Box::new(Self { bytes: [0; 4096] })
    }

    /// The address of the bytes themselves — taken through the field so the
    /// storage is read rather than merely reserved.
    fn base(&self) -> u64 {
        self.bytes.as_ptr() as u64
    }
}

fn layout(size: usize) -> Layout {
    Layout::from_size_align(size, 8).expect("a layout this test wrote")
}

/// An arena described as an empty heap, and the heap over it.
fn described(arena: &Arena) -> Heap {
    // SAFETY: `arena` is 4096 bytes, sixteen-aligned, alive for the call, and
    // held by nobody else — the test owns it.
    unsafe { heap::describe(arena.base(), 4096) };
    // SAFETY: the region was described immediately above, is readable and
    // writable for the 4096 bytes that prologue records, and outlives the value.
    unsafe { Heap::over(arena.base()) }
}

/// Take a block.
fn take(heap: &Heap, size: usize) -> *mut u8 {
    // SAFETY: `heap` is over a region this test described and owns, and the
    // layout is one this test made. `alloc`'s obligation on the caller is only
    // that the layout is valid, which `Layout::from_size_align` established.
    unsafe { heap.alloc(layout(size)) }
}

/// Give one back.
fn give(heap: &Heap, pointer: *mut u8, size: usize) {
    // SAFETY: `pointer` is one this same heap answered and `size` is the size it
    // was taken with, which together are the whole of `dealloc`'s obligation.
    unsafe { heap.dealloc(pointer, layout(size)) };
}

/// Offsets rather than addresses, so a failure reads as arithmetic.
fn offset_of(pointer: *mut u8, arena: &Arena) -> u32 {
    u32::try_from(pointer as u64 - arena.base()).expect("inside the arena")
}

#[test]
fn a_described_region_is_valid_and_an_undescribed_one_is_not() {
    let arena = Arena::new();
    let heap = described(&arena);
    assert!(heap.valid(), "a region just described reads as a heap");
    assert_eq!(heap.bytes(), 4096);
    assert_eq!(heap.live(), 0);

    // The control: the same bytes with no magic word. A component that adopted
    // an undescribed region would allocate out of arbitrary memory, so the
    // refusal is the property and this is what says the check is doing it.
    let blank = Arena::new();
    // SAFETY: `blank` is alive, owned by this test and 4096 bytes; it is simply
    // not a heap, which is what the assertions below are about.
    let heap = unsafe { Heap::over(blank.base()) };
    assert!(!heap.valid(), "zeroed bytes are not a heap");
    assert!(take(&heap, 16).is_null(), "an invalid heap allocates nothing");
}

#[test]
fn the_first_block_starts_after_the_prologue_and_sizes_round_up() {
    let arena = Arena::new();
    let heap = described(&arena);

    let one = take(&heap, 1);
    assert_eq!(offset_of(one, &arena), HEADER_BYTES, "the first block is after the header");
    assert_eq!(heap.live(), GRANULARITY, "one byte occupies one block");

    let next = take(&heap, 17);
    assert_eq!(
        offset_of(next, &arena),
        HEADER_BYTES + GRANULARITY,
        "the second block follows the first"
    );
    assert_eq!(heap.live(), GRANULARITY * 3, "seventeen bytes occupy two blocks");
}

#[test]
fn first_fit_takes_the_lowest_block_that_fits() {
    let arena = Arena::new();
    let heap = described(&arena);

    // Three blocks, then free the first and the third, so the free list has two
    // entries and the lower one is what a first fit must take.
    let first = take(&heap, 16);
    let second = take(&heap, 16);
    let third = take(&heap, 16);
    give(&heap, first, 16);
    give(&heap, third, 16);

    let again = take(&heap, 16);
    assert_eq!(
        offset_of(again, &arena),
        offset_of(first, &arena),
        "first fit takes the lowest free block, not the most recently freed"
    );

    give(&heap, second, 16);
    give(&heap, again, 16);
}

#[test]
fn a_split_leaves_a_remainder_only_when_the_remainder_is_a_whole_block() {
    let arena = Arena::new();
    let heap = described(&arena);

    let big = take(&heap, 64);
    let guard = take(&heap, 16);
    give(&heap, big, 64);

    // Taking 48 of the 64 leaves 16, which is a whole block and survives.
    let part = take(&heap, 48);
    assert_eq!(offset_of(part, &arena), offset_of(big, &arena));
    let tail = take(&heap, 16);
    assert_eq!(
        offset_of(tail, &arena),
        offset_of(big, &arena) + 48,
        "the remainder of a split is itself allocatable"
    );

    give(&heap, part, 48);
    give(&heap, tail, 16);
    give(&heap, guard, 16);
}

#[test]
fn a_free_coalesces_with_both_neighbours() {
    let arena = Arena::new();
    let heap = described(&arena);

    let a = take(&heap, 16);
    let b = take(&heap, 16);
    let c = take(&heap, 16);
    let keep = take(&heap, 16);

    give(&heap, a, 16);
    give(&heap, c, 16);
    // The middle one closes the gap on both sides, so what is free is one block
    // of forty-eight rather than three of sixteen.
    give(&heap, b, 16);

    let whole = take(&heap, 48);
    assert_eq!(
        offset_of(whole, &arena),
        offset_of(a, &arena),
        "three adjacent frees are one block, so a request for all of it fits"
    );

    give(&heap, whole, 48);
    give(&heap, keep, 16);
}

#[test]
fn a_free_list_is_a_function_of_the_live_set() {
    // THE CONTROL. Two histories, one live set, one free list.
    //
    // Both arenas end with the same blocks live and the same ones free, reached
    // by different orders of allocation and release. If the free list were
    // ordered by when a block came back, the next allocation would differ
    // between them, and a seeded run would stop reproducing.
    let first = Arena::new();
    let second = Arena::new();
    let one = described(&first);
    let two = described(&second);

    let a: Vec<*mut u8> = (0..4).map(|_| take(&one, 16)).collect();
    give(&one, a[1], 16);
    give(&one, a[3], 16);

    let b: Vec<*mut u8> = (0..4).map(|_| take(&two, 16)).collect();
    // The other order, and one extra round trip through a block for good measure.
    give(&two, b[3], 16);
    give(&two, b[1], 16);
    let churn = take(&two, 16);
    give(&two, churn, 16);

    assert_eq!(one.live(), two.live(), "the two histories reach the same live set");
    assert_eq!(
        offset_of(take(&one, 16), &first),
        offset_of(take(&two, 16), &second),
        "the same live set allocates the same address whatever history produced it"
    );
}

#[test]
fn a_request_past_the_region_is_refused_and_recorded() {
    let arena = Arena::new();
    let heap = described(&arena);

    assert!(!heap.starved(), "a fresh heap has not been starved");
    assert!(take(&heap, 8192).is_null(), "a request larger than the region is refused");
    assert!(heap.starved(), "and the refusal is recorded where a reader can see it");

    // An alignment this allocator will not serve is refused *without* being
    // recorded as starvation: it is a request it does not answer, not a region
    // that ran out, and a reader has to be able to tell those apart.
    let wide = Layout::from_size_align(16, 64).expect("a layout this test wrote");
    // SAFETY: `heap` is over a region this test described and owns, and `wide`
    // is a valid layout.
    let refused = unsafe { heap.alloc(wide) };
    assert!(refused.is_null(), "an alignment wider than the granularity is refused");
}

#[test]
fn peak_remembers_what_live_forgets() {
    let arena = Arena::new();
    let heap = described(&arena);

    let a = take(&heap, 64);
    let b = take(&heap, 64);
    assert_eq!(heap.live(), 128);
    assert_eq!(heap.peak(), 128);

    give(&heap, a, 64);
    give(&heap, b, 64);
    assert_eq!(heap.live(), 0, "everything came back");
    assert_eq!(heap.peak(), 128, "and the high-water mark is what a reader wants");
}
