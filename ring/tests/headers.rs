// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Hostile headers, against a real mapping.
//!
//! E0-B13. Until this file existed the header tests called
//! [`f_abi::ChannelHeader::negotiate`] on a struct that lived on the test's own
//! stack — which checks the arithmetic and nothing else. A header is not a
//! struct. It is the first cache line of a region two components share, and the
//! question that matters is what happens when a peer writes sixty-four hostile
//! bytes there and the other end binds to them.
//!
//! # What is being asserted
//!
//! One property, in two halves. Every invalid header is **refused with a
//! structured error** — never accepted, never a panic, and never a `Mapping`
//! that would go on to hand out slices computed from numbers nobody checked.
//! And the refusal is **clean**: the region survives it, so a channel that was
//! torn down can be set up again over the same bytes. A teardown that poisoned
//! the mapping would be a denial of service any peer could trigger at will.
//!
//! # Why the region is a real allocation and not a fixture
//!
//! Because the fixture it replaces could only ever be laid out correctly. A
//! `struct` with a cursor field and an entry array is a channel the borrow
//! checker already proved well-formed; the offsets are Rust's problem and the
//! wire format never gets consulted. Here the region is a flat, 64-byte-aligned
//! run of bytes, every region inside it is found through the offsets the header
//! claims, and an off-by-one-line header is a test case rather than a compile
//! error.

use std::sync::atomic::Ordering;

use f_abi::layout::{self, Layout};
use f_abi::{ABI_VERSION, CHANNEL_MAGIC, ChannelHeader, Sqe, error, feature};
use f_ring::{Consumer, Mapping, Producer};

/// Entries in the test channel. Eight, so a 4 KiB region has room for the rings
/// and an arena, and the arithmetic stays small enough to check by hand.
const ENTRIES: u32 = 8;

/// The region a channel is bound in. One frame, which is the unit the kernel
/// shares.
const LEN: u32 = 4096;

/// A run of shared bytes at the alignment the layout is stated against.
///
/// `align(64)` is the whole point of the type: the fixed regions are placed on
/// cache lines measured from the first byte, so a region that is not itself
/// line-aligned makes every one of those offsets wrong.
#[repr(C, align(64))]
struct Region([u8; LEN as usize]);

impl Region {
    fn new() -> Box<Self> {
        // Boxed rather than a local, because 4 KiB of stack in a test that runs
        // on every architecture is the kind of thing that fails only on the
        // runner with the smallest thread stack.
        Box::new(Self([0; LEN as usize]))
    }

    fn base(&mut self) -> *mut u8 {
        self.0.as_mut_ptr()
    }

    /// Write a header the way a peer would: straight into the first cache line,
    /// without going through [`Mapping::describe`], which by construction can
    /// only produce sound ones.
    fn place(&mut self, header: ChannelHeader) {
        let base = self.base();
        // SAFETY: `Region` is 64-byte aligned and 4 KiB long, so a 64-byte
        // `ChannelHeader` needing 64-byte alignment fits at offset zero. The
        // exclusive borrow proves nothing else holds a reference to it.
        unsafe { base.cast::<ChannelHeader>().write(header) };
    }
}

/// A header a well-behaved peer writes: this build's own layout, described.
///
/// Derived from [`Layout`] rather than written out by hand, so that a change to
/// the wire layout moves the sound header with it instead of leaving a stale
/// literal that every hostile case is measured against.
fn sound() -> ChannelHeader {
    Layout::new(ENTRIES, 0).expect("eight entries is a layout").describe(0, 0, 0)
}

/// One hostile case: the peer behaviour it stands for, the header that peer
/// writes, and the domain and code the refusal has to carry.
type Hostile = (&'static str, fn(&mut ChannelHeader), u8, u16);

fn refusal(code: i32) -> (u8, u16) {
    error::unpack(code).expect("a refusal is a negative structured error")
}

#[test]
fn a_sound_header_binds_and_the_two_ends_agree_on_the_bytes() {
    // The control. Without it every assertion in this file would also pass
    // against a `Mapping::adopt` that refused everything.
    let mut region = Region::new();
    let base = region.base();

    // SAFETY: the region is 4 KiB, 64-byte aligned, zeroed, and borrowed for as
    // long as the mappings below live.
    let written =
        unsafe { Mapping::describe(base, LEN, ENTRIES, 0, 0, 0) }.expect("a sound channel");
    // SAFETY: as above. Adopting the same bytes a second time is the point:
    // this is the reader's end of the channel the writer just described.
    let read = unsafe { Mapping::adopt(base, LEN, 0, 0) }.expect("what was written is readable");

    assert_eq!(written.layout(), read.layout(), "the two ends disagree about where the rings are");
    assert_eq!(read.negotiated().version, ABI_VERSION);
    assert_eq!(read.negotiated().features, 0, "neither side offered a feature");
    assert_eq!(read.epoch(), 0);

    // The arena is what the mapping has left over, and never what the header
    // says — the one number a peer does not get to choose.
    assert_eq!(read.layout().arena_len(), LEN - read.layout().arena_offset());

    // And the halves bind to it. A layout that adopts and then produces a
    // channel the ring will not accept would pass every assertion above.
    let producer = Producer::new(read.channel()).expect("a power-of-two ring");
    let consumer = Consumer::new(read.channel()).expect("a power-of-two ring");
    consumer.disarm_wakeup();
    producer.submit(Sqe::ZERO).expect("an empty ring takes an entry");
    assert!(consumer.pop().expect("a sound ring").is_some(), "the entry did not arrive");
}

#[test]
fn every_hostile_header_is_refused_with_a_structured_error() {
    // Each case names the peer behaviour it stands for, not the field it pokes.
    // The field is visible in the code; what is worth writing down is why a
    // peer would ever produce it.
    let hostile: [Hostile; 15] = [
        (
            "an unmapped or never-written page, which reads as zeroes",
            |h| {
                *h = ChannelHeader {
                    magic: 0,
                    features: 0,
                    features_required: 0,
                    abi_version: 0,
                    abi_version_min: 0,
                    ring_size: 0,
                    sqe_offset: 0,
                    cqe_offset: 0,
                    epoch: 0,
                    _reserved: [0; 4],
                };
            },
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a mapping of something that is not a channel at all",
            |h| h.magic = 0x4142_4344_4546_4748,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a magic one bit away from ours, which is what a corrupted line looks like",
            |h| h.magic = CHANNEL_MAGIC ^ 1,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a ring with no slots",
            |h| h.ring_size = 0,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a ring size that is not a power of two, so the mask would not be one",
            |h| h.ring_size = 9,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a ring larger than any this build will index",
            |h| h.ring_size = 1 << 25,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a ring whose regions are self-consistent and do not fit the mapping",
            |h| *h = Layout::new(1024, 0).expect("a large layout").describe(0, 0, 0),
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a version floor above the version it claims to speak",
            |h| h.abi_version_min = h.abi_version + 1,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a reserved word carrying something, which is a field we do not know we are ignoring",
            |h| h._reserved[2] = 1,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "an entry array overlapping the header that describes it",
            |h| h.sqe_offset = 0,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "an entry array one cache line from where this build puts it",
            |h| h.sqe_offset += 64,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a completion ring at some other build's arithmetic",
            |h| h.cqe_offset -= 64,
            error::ARGUMENT,
            error::argument::MALFORMED_HEADER,
        ),
        (
            "a peer from the future, speaking nothing this build speaks",
            |h| {
                h.abi_version = ABI_VERSION + 100;
                h.abi_version_min = ABI_VERSION + 99;
            },
            error::PEER,
            error::peer::VERSION_UNSUPPORTED,
        ),
        (
            "a peer from before there was a version",
            |h| {
                h.abi_version = 0;
                h.abi_version_min = 0;
            },
            error::PEER,
            error::peer::VERSION_UNSUPPORTED,
        ),
        (
            "a peer that cannot proceed without something this build does not offer",
            |h| {
                h.features = feature::SHARED_VIRTUAL_MEMORY;
                h.features_required = feature::SHARED_VIRTUAL_MEMORY;
            },
            error::PEER,
            error::peer::FEATURE_REQUIRED,
        ),
    ];

    let mut region = Region::new();

    for (peer, bend, domain, code) in hostile {
        let mut header = sound();
        bend(&mut header);
        region.place(header);

        let base = region.base();
        // SAFETY: the region is 4 KiB, aligned, and borrowed exclusively. Its
        // *contents* are hostile, which is this test's subject and not a safety
        // obligation: nothing is dereferenced through those numbers unless
        // `adopt` returned a mapping, and here it does not.
        let bound = unsafe { Mapping::adopt(base, LEN, 0, 0) };

        let Err(refused) = bound.map(|_| ()) else {
            panic!("{peer} was accepted");
        };
        assert_eq!(refusal(refused), (domain, code), "{peer} was refused for the wrong reason");

        // The other half: the refusal did not poison the region. A peer that
        // could make one bad header cost the channel permanently would have a
        // denial of service rather than a rejected message.
        region.place(sound());
        let base = region.base();
        // SAFETY: as above, over a header this build has just written.
        let again = unsafe { Mapping::adopt(base, LEN, 0, 0) };
        assert!(again.is_ok(), "the channel did not come back after refusing {peer}");
    }
}

#[test]
fn an_address_the_layout_cannot_be_stated_against_is_refused_before_the_header_is_read() {
    // These two checks cannot come from the header, because they are what makes
    // reading the header defined. A build that took the caller's word for them
    // would have undefined behaviour where it wanted a refusal.
    let mut region = Region::new();
    region.place(sound());
    let base = region.base();

    // SAFETY: one byte into a 4 KiB region is still inside it.
    let skewed = unsafe { base.add(1) };
    // SAFETY: the region really does have `LEN - 1` bytes from there. It is
    // simply not aligned, which is what this asserts is caught rather than
    // assumed.
    let bound = unsafe { Mapping::adopt(skewed, LEN - 1, 0, 0) };
    let Err(code) = bound.map(|_| ()) else { panic!("an unaligned base was accepted") };
    assert_eq!(refusal(code), (error::ARGUMENT, error::argument::BAD_ADDRESS));

    // SAFETY: a truthful length for a caller whose region really is this short.
    let stub = unsafe { Mapping::adopt(base, 63, 0, 0) };
    let Err(code) = stub.map(|_| ()) else {
        panic!("a region too short for a header was accepted")
    };
    assert_eq!(refusal(code), (error::ARGUMENT, error::argument::BAD_ADDRESS));
}

#[test]
fn a_peer_that_offers_what_we_require_is_the_only_one_that_binds() {
    // RFC 0011: peers meet in the middle. Both directions of the required-set
    // check matter, and only one of them is visible in the hostile table above
    // — that is the peer requiring something of us. This is the mirror: us
    // requiring something of the peer.
    let mut region = Region::new();

    let mut offered = sound();
    offered.features = feature::CONTROL_EVENTS;
    region.place(offered);
    let base = region.base();
    // SAFETY: aligned, in bounds, exclusively borrowed.
    let bound =
        unsafe { Mapping::adopt(base, LEN, feature::CONTROL_EVENTS, feature::CONTROL_EVENTS) };
    assert_eq!(
        bound.map(|m| m.negotiated().features),
        Ok(feature::CONTROL_EVENTS),
        "a feature both sides offered was not agreed"
    );

    let mut silent = sound();
    silent.features = 0;
    region.place(silent);
    let base = region.base();
    // SAFETY: as above.
    let bound =
        unsafe { Mapping::adopt(base, LEN, feature::CONTROL_EVENTS, feature::CONTROL_EVENTS) };
    let Err(code) = bound.map(|_| ()) else {
        panic!("a peer that cannot do what we require was bound")
    };
    assert_eq!(refusal(code), (error::PEER, error::peer::FEATURE_REQUIRED));
}

#[test]
fn a_peer_that_restarts_is_a_different_channel() {
    // The epoch is carried at bind time rather than re-read, so a restart is
    // noticed as a mismatch instead of quietly becoming the new truth. A
    // channel bound at epoch 3 and a mapping now saying 4 are two channels, and
    // every token outstanding on the first is stale.
    let mut region = Region::new();
    let base = region.base();
    // SAFETY: aligned, 4 KiB, zeroed, exclusively borrowed.
    let bound = unsafe { Mapping::describe(base, LEN, ENTRIES, 3, 0, 0) }.expect("a sound channel");
    assert_eq!(bound.epoch(), 3);

    let mut restarted = sound();
    restarted.epoch = 4;
    region.place(restarted);
    let base = region.base();
    // SAFETY: as above.
    let after =
        unsafe { Mapping::adopt(base, LEN, 0, 0) }.expect("a restart is not a malformed header");
    assert_ne!(bound.epoch(), after.epoch(), "a restart left the two ends agreeing");
}

#[test]
fn the_bytes_a_sound_mapping_hands_out_are_the_ones_the_header_named() {
    // The arithmetic is checked against the region rather than against itself:
    // write through the slices `Mapping` produced, then read the same values
    // back out of the raw region at the offsets the header claims. A `Layout`
    // that adopted successfully and then pointed somewhere else would pass
    // every other test in this file.
    let mut region = Region::new();
    let base = region.base();
    // SAFETY: aligned, 4 KiB, zeroed, exclusively borrowed for the mapping.
    let mapping =
        unsafe { Mapping::describe(base, LEN, ENTRIES, 0, 0, 0) }.expect("a sound channel");
    let bound = mapping.layout();

    mapping.channel().head.set(5);
    mapping.channel().index[3].store(0x2143, Ordering::Relaxed);
    let arena = mapping.arena_cells();
    // SAFETY: the arena came from the adopted layout, and nothing else holds a
    // reference to this byte.
    unsafe { arena[7].get().write(0xAB) };

    let at = layout::HEAD as usize;
    let head = u32::from_le_bytes(region.0[at..at + 4].try_into().expect("four bytes"));
    assert_eq!(head, 5, "the head cursor is not on the line the layout names");

    let at = (bound.sq_index_offset() + 3 * 4) as usize;
    let slot = u32::from_le_bytes(region.0[at..at + 4].try_into().expect("four bytes"));
    assert_eq!(slot, 0x2143, "the index ring is not where the layout says");

    assert_eq!(region.0[bound.arena_offset() as usize + 7], 0xAB, "the arena is somewhere else");
}
