// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The ring's validation paths, as bounded proofs.
//!
//! # The sentence these are about
//!
//! `ring/src/lib.rs`'s module comment ends with five clauses, and the last of
//! them is the one with no instrument under it: *never panic on anything a peer
//! wrote*. `ring/tests/headers.rs` checks it for fifteen headers somebody
//! thought of. `ring/tests/hostile.rs` checks it for a billion drawn operations
//! and found nothing, which is the strongest available statement of the gap
//! these harnesses close — **a billion samples is still sampling**. What is
//! below is the same sentence with the sampling removed: a solver is asked
//! whether *any* bytes produce a panic, and answers.
//!
//! # The three quantifiers, and which is bounded
//!
//! **The bytes are not bounded.** [`peer::Region::scribbled`] is a mapping
//! every byte of which the solver chose — header, cursors, flags word, index
//! ring, both entry arrays and arena, all at once and all related to each other
//! the way they are in a real channel.
//!
//! **The cursors and the slot numbers are not bounded.** They are part of those
//! bytes, so every one of the 2^32 values of each is in scope, including the
//! ones that make a wrapping occupancy read as full and the ones that name an
//! entry past the end of the array.
//!
//! **The mapping is bounded, and by a smaller one than a channel usually is.**
//! [`peer::REGION`] says how small and what that admits, and three narrower
//! bounds sit under it: [`ARENA`], [`REGISTERED`] with [`SETS`], and the one
//! harness that builds its channel out of fields rather than out of a region.
//! Each is argued at the constant, and RFC 0057 collects them.
//!
//! # Where to start reading
//!
//! At [`popping_an_arbitrary_entry`], which is the shortest statement of what
//! this file is: a region of arbitrary bytes, a real `Mapping::adopt`, a real
//! `Consumer`, and the question *is there a slot number that gets past the
//! check*. It is also the harness `mutate-trusted-slot` has to break, so it is
//! the one place where the two halves of `cargo xtask prove` meet on one page.
//!
//! # Why the assertions are two-sided
//!
//! For the reason `kernel/proofs` gives, which turned out to matter more here:
//! every operation on this path *may refuse*, so *no panic* is satisfied
//! completely by a ring that answers `Corrupt` to everything. Where there is a
//! sentence to state — a refusal is the one R04 names, a resolved buffer is
//! inside the registration, a drain does no more work than its budget — it is
//! stated as an equivalence and not as an implication.
//!
//! # Why there are `cover` statements
//!
//! Because the question this repository asks of a green result is *what input
//! would make it green while the property was false*, and for a proof over
//! arbitrary bytes the answer is: bytes that never get past the first check.
//! A harness whose `Mapping::adopt` always refuses proves nothing about `pop`,
//! costs nothing to write, and looks exactly like one that proves everything.
//! `kani::cover!` is the instrument for that: an unsatisfiable cover is a
//! **failed** verification, so a fixture that stopped reaching the code it is
//! about goes red rather than quiet.
//!
//! Not because the checker says so — it does not; it reports an unreachable
//! cover and then `VERIFICATION:- SUCCESSFUL` — but because `cargo xtask prove`
//! reads the count and refuses. `lib.rs` says why that distinction is the whole
//! of whether this file's claim is a mechanism or a wish.

// The checker injects `kani` as an extern crate; without one, `crate::kani` is
// the shim that lets this file typecheck under the pinned toolchain — which is
// what makes a changed signature in `f-ring` a fifteen-second `cargo xtask
// lint` failure rather than a discovery twenty minutes into a nightly. See
// `lib.rs`.
#[cfg(not(kani))]
use crate::kani;

use f_abi::ChannelHeader;
use f_abi::buf::{Name, Request, SetId};
use f_abi::layout::Layout;
use f_abi::{ABI_VERSION, ABI_VERSION_MIN, error, feature, flags, op};
use f_ring::buffers::{BufferSet, Fixed, PeerGone};
use f_ring::registry::{BUFFERS_MAX, Table, Transport};
use f_ring::{
    Arena, Collector, Consumer, Mapping, Poster, Producer, Region as DeviceRegion, RingError,
    Service, Window, execute, registry,
};

use crate::peer::{
    Bucket, Bytes, DEVICE_BASE, Domain, Lane, Region, Rings, Walk, any_cqe, any_header, any_len,
    any_set, any_sqe,
};

/// Slots in the registration table a harness stands up.
///
/// Two, for [`peer::REGION`](crate::peer::REGION)'s reason one layer up: the
/// table is scanned linearly by `register` and by `live`, so its size is a
/// multiplier on every harness that touches it, and two is the smallest count
/// at which *the lowest free slot* is a choice rather than a tautology. A power
/// of two because `Table::new` asserts one at compile time.
const SLOTS: usize = 2;

/// Bytes of arena the harnesses that reach [`execute`] are proved over.
///
/// The second bound in this crate, and the one that costs something. It is
/// small for a reason a reader should be able to check: `write_serial` has two
/// nested loops whose trip counts an entry's `len` decides, and a checker
/// unrolls both to the same bound — so the cost is quadratic in the arena and
/// the arena is the only thing that bounds `len`. At eight bytes the outer loop
/// is provably one pass and the inner one is eight.
///
/// **What that leaves out, stated because it is the gap and not a detail:**
/// `f_ring::CHUNK` is 256, so an arena of eight never makes the chunking loop
/// take a second pass. The multi-pass path is therefore *checked* and not
/// proved — `ring/src/lib.rs`'s own fixture sizes its arena at `CHUNK * 2 + 16`
/// precisely so that the loop is exercised rather than assumed, and
/// `ring/tests/hostile.rs` drives arenas of thousands of bytes a billion times.
/// What a proof adds over those is quantification over the *entry*, and that is
/// what is here. RFC 0057.
const ARENA: usize = 8;

/// The largest registration a harness makes. Unit: bytes.
///
/// The third bound, and it is on the *geometry* rather than on the name. A set
/// is at most this many bytes in at most [`SETS`] buffers, so a buffer's stride
/// is a small power of two — and the index a peer presents is still every one
/// of the 2^32 there are.
///
/// The reason for the split is arithmetic rather than taste. `Table::resolve`
/// answers `address + index * stride`, and a thirty-two-by-thirty-two-bit
/// multiplication of two symbolic operands is one of the most expensive things
/// a SAT solver can be handed: with both free this harness did not finish in
/// thirteen minutes, and with the stride bounded it is seconds. Bounding the
/// *index* instead would have been the cheap and wrong choice, because the
/// index is the peer's and the defect this harness has to fail on is an index
/// past the end of the set.
const REGISTERED: u32 = 8;

/// The largest buffer count a harness registers. Unit: buffers.
///
/// See [`REGISTERED`]. Two rather than one so that *which* buffer an index
/// names is a question with more than one answer, and so that `index >= buffers`
/// is reachable at an index the set does hold one of.
const SETS: u32 = 2;

/// Bytes of buffer-set region the client-side ownership harness carves.
/// Unit: bytes.
const LENT: usize = 4;

/// Entries in the ring `draining_an_arbitrary_channel` builds by hand.
/// Unit: entries.
///
/// Two, so that *how many entries a drain took* is a question with more than
/// one answer and the budget can be exceeded. A power of two because
/// `Channel::mask` refuses anything else, which is a refusal
/// `adopting_arbitrary_bytes` proves over a region and this fixture does not
/// need to rediscover.
const RING: usize = 2;

// ---------------------------------------------------------------------------
// 1. The layout, which is where a header stops being bytes.
// ---------------------------------------------------------------------------

/// What a header this build wrote looks like, written out rather than asked.
///
/// # Why this is not `ChannelHeader::is_valid`
///
/// Because an oracle that calls the code under test is a change detector and
/// not a specification. `Layout::adopt`'s first act is `is_valid`, so an
/// equivalence whose other side also called it would move whenever that check
/// moved: weaken `is_valid` — drop the magic, admit a non-zero reserved word,
/// which is exactly R04's fail-closed half — and both sides of
/// `adopting_an_arbitrary_layout` would agree about the weakened build, the
/// harness would stay green, and its own sentence, *adopt succeeds exactly when
/// the header is this build's own*, would have stopped being true while the
/// proof still said it was.
///
/// So the five clauses are here in full, from `abi/src/lib.rs`'s field
/// documentation rather than from its implementation, and one of them is
/// deliberately spelled differently: `count_ones() == 1` where `is_valid` says
/// `is_power_of_two()`. Two spellings of one property is what makes the
/// comparison a comparison.
///
/// **The standing cost**, which is the reason this is a doc comment and not a
/// line: a sixth clause added to `is_valid` and not added here would make
/// `adopting_an_arbitrary_layout` fail — loudly, on the clean build, saying a
/// header this build wrote was refused. That is the right direction for this to
/// rot in, and it is the reversal condition.
fn this_builds_header(header: &ChannelHeader) -> bool {
    header.magic == f_abi::CHANNEL_MAGIC
        && header.ring_size.count_ones() == 1
        && header.abi_version_min <= header.abi_version
        && header._reserved[0] == 0
        && header._reserved[1] == 0
        && header._reserved[2] == 0
        && header._reserved[3] == 0
}

/// `Layout::adopt` over every header and every mapping length there is.
///
/// The one harness in this file with nothing bounded at all: no memory is
/// dereferenced, so `mapping_len` is a full symbolic `u32` rather than a length
/// the fixture can answer for, and the header is arbitrary bytes.
///
/// The equivalence is the point. `adopt` is *the* check the module exists for —
/// `abi/src/layout.rs` says a peer whose arithmetic differs from ours is caught
/// here rather than at the first read — so a version of it that refused
/// everything would satisfy every safety property this file states. What is
/// asserted is therefore that it succeeds **exactly** when the header is this
/// build's own, and that when it does, every region it names is inside the
/// mapping.
///
/// The left-hand side of that equivalence is [`this_builds_header`] and **not**
/// `ChannelHeader::is_valid`, which is what `adopt` itself calls. An oracle that
/// re-derives its expectation by calling the code under test is a change
/// detector: weaken `is_valid` and both sides move together, this harness stays
/// green, and the sentence above stops being true while still being printed.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn adopting_an_arbitrary_layout() {
    let header = any_header();
    let mapping_len: u32 = kani::any();

    let computed = Layout::new(header.ring_size, 0);
    let ours = this_builds_header(&header)
        && match computed {
            Some(c) => {
                header.sqe_offset == c.sqe_offset()
                    && header.cqe_offset == c.cqe_offset()
                    && mapping_len >= c.arena_offset()
            }
            None => false,
        };

    match Layout::adopt(&header, mapping_len) {
        Ok(adopted) => {
            assert!(ours, "a layout was adopted from a header this build would not have written");
            // Every region inside the mapping, which is the property every
            // `from_raw_parts` in `ring/src/mapping.rs` rests on.
            assert!(adopted.total() <= mapping_len, "the layout describes more than the mapping");
            assert!(adopted.entries() > 0, "a ring of no entries was adopted");
            assert!(adopted.entries().is_power_of_two(), "a ring size that cannot be a mask");
            assert!(adopted.sqe_offset() >= adopted.sq_index_offset() + 4 * adopted.entries());
            assert!(adopted.cqe_offset() == adopted.sqe_offset() + 64 * adopted.entries());
            assert!(adopted.arena_offset() >= adopted.cqe_offset() + 32 * adopted.entries());
            assert!(adopted.arena_offset() + adopted.arena_len() == adopted.total());
        }
        Err(refused) => {
            assert!(!ours, "a header this build wrote was refused");
            assert_eq!(
                error::unpack(refused),
                Some((error::ARGUMENT, error::argument::MALFORMED_HEADER)),
                "R07: a refusal names its domain"
            );
        }
    }

    kani::cover!(Layout::adopt(&header, mapping_len).is_ok(), "some header is adopted");
    kani::cover!(Layout::adopt(&header, mapping_len).is_err(), "some header is refused");
}

/// `ChannelHeader::negotiate` over every header and every pair of feature sets.
///
/// RFC 0011's rule, quantified: peers meet in the middle. The two-sided form is
/// what stops *the agreed set is a subset of what we offered* from being
/// satisfied by an implementation that agrees to nothing.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn negotiating_with_an_arbitrary_peer() {
    let header = any_header();
    let offers: u64 = kani::any();
    let requires: u64 = kani::any();

    match header.negotiate(offers, requires) {
        Ok(agreed) => {
            assert!(this_builds_header(&header), "a header that is not one was negotiated with");
            assert_eq!(agreed.features, header.features & offers, "a feature nobody offered");
            assert!(agreed.version <= ABI_VERSION, "a version above this build's");
            assert!(agreed.version >= ABI_VERSION_MIN, "a version below this build's floor");
            assert!(agreed.version >= header.abi_version_min, "a version below the peer's floor");
            assert_eq!(header.features_required & !agreed.features, 0, "the peer needs more");
            assert_eq!(requires & !agreed.features, 0, "this side needs more");
        }
        Err(refused) => {
            let version = core::cmp::min(header.abi_version, ABI_VERSION);
            let common = header.features & offers;
            let unmet = header.features_required & !common != 0 || requires & !common != 0;
            let stale = version < header.abi_version_min || version < ABI_VERSION_MIN;
            assert!(
                !this_builds_header(&header) || stale || unmet,
                "a refusal with none of the three reasons behind it"
            );
            assert!(error::unpack(refused).is_some(), "R07: a refusal names its domain");
        }
    }

    kani::cover!(header.negotiate(offers, requires).is_ok(), "some peer is agreed with");
}

/// The ring sizes a region actually admitted, as covers.
///
/// # What this guards, and why nothing else could
///
/// `peer::REGION` is arithmetic against `f_abi::layout`'s offsets: it is 640
/// bytes because `Layout::new` puts the arena of a two-entry ring at 576, and a
/// change to `SQ_INDEX` or to the header's size moves that — silently, and in
/// the direction of admitting *fewer* ring sizes. A region that admitted only a
/// ring of one would still satisfy every other cover in this file, still verify
/// every harness, and still be a strictly smaller proof than the one the file
/// claims. RFC 0057 named that as this arrangement's standing cost and offered
/// these covers as the guard; this is them, and until it was written the guard
/// was a sentence.
///
/// Both sizes are reachable at both bounds — `wide-ring` grows the region to
/// admit four and eight and does not stop admitting one or two — so this is the
/// same assertion in the narrow pass and the wide one, which is what makes it a
/// check on the region rather than on the feature. It is **not** a complete
/// guard: a ring size the fixture can no longer reach that nothing names here
/// would still pass. Two are named because two are what the narrow bound is
/// documented as admitting.
fn reached(entries: u32) {
    kani::cover!(entries == 1, "a ring of one entry is admitted by the region");
    kani::cover!(entries == 2, "a ring of two entries is admitted by the region");
}

// ---------------------------------------------------------------------------
// 2. Adoption, over bytes rather than over fields.
// ---------------------------------------------------------------------------

/// `Mapping::adopt` over a region whose every byte the solver chose.
///
/// The harness the deliberate defect `mutate-believed-header` has to break:
/// that feature turns `Layout::adopt`'s refusal into an `expect`, and this is
/// the only place a header the layout refuses reaches it.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn adopting_arbitrary_bytes() {
    let mut region = Region::scribbled();
    let len = any_len();
    let offers: u64 = kani::any();
    let requires: u64 = kani::any();
    let base = region.base();

    // SAFETY: `base` names `len` bytes this harness owns — `any_len` bounds the
    // length by the region — and no other reference into the range exists while
    // the mapping does. The raw pointer the mapping keeps aliases `region`
    // across later calls, which stacked borrows would object to and CBMC does
    // not model; `kernel/proofs/src/pages.rs` records the same fact about the
    // same checker, and no result here rests on it either way.
    let adopted = unsafe { Mapping::adopt(base, len, offers, requires) };

    match adopted {
        Ok(mapping) => {
            let layout = mapping.layout();
            assert!(layout.total() <= len, "a mapping was bound over bytes it does not have");
            assert_eq!(mapping.negotiated().features & !offers, 0, "a feature nobody offered");
            kani::cover!(true, "a scribbled region is adopted");
            reached(layout.entries());
        }
        Err(refused) => {
            assert!(error::unpack(refused).is_some(), "R07: a refusal names its domain");
            kani::cover!(true, "a scribbled region is refused");
        }
    }
}

// ---------------------------------------------------------------------------
// 3. The four paths the task names, over the bytes of an adopted channel.
// ---------------------------------------------------------------------------

/// `Consumer::pop` against every cursor pair and every slot number.
///
/// **The harness `mutate-trusted-slot` has to break.** The index ring is part
/// of the region, so the slot number `pop` reads is a full symbolic `u32`: the
/// bounds check in `Consumer::pop` is the only thing between it and an entry
/// array of one or two elements, and a proof quantifying over every value of it
/// has to find that out.
///
/// What the covers say is that the fixture reaches all three answers. Without
/// them a region that never adopted would verify this in a line, and the
/// property would be about nothing.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn popping_an_arbitrary_entry() {
    let mut region = Region::scribbled();
    let len = any_len();
    let base = region.base();
    // SAFETY: as `adopting_arbitrary_bytes`.
    let Ok(mapping) = (unsafe { Mapping::adopt(base, len, 0, 0) }) else { return };
    let Some(consumer) = Consumer::new(mapping.channel()) else { return };
    reached(mapping.layout().entries());

    match consumer.pop() {
        Ok(Some(_)) => kani::cover!(true, "an entry comes back"),
        Ok(None) => kani::cover!(true, "the ring reads empty"),
        Err(_) => kani::cover!(true, "the cursors are refused"),
    }
}

/// `Collector::take` against every cursor pair.
///
/// The completion ring has no index ring — RFC 0018 placed completions inline —
/// so the untrusted input here is the two cursors and nothing else. That makes
/// this the cheapest harness in the file and it is here anyway, because *the
/// four paths the promise names* is the property and a list that is the
/// property has to be the whole list.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn taking_an_arbitrary_completion() {
    let mut region = Region::scribbled();
    let len = any_len();
    let base = region.base();
    // SAFETY: as `adopting_arbitrary_bytes`.
    let Ok(mapping) = (unsafe { Mapping::adopt(base, len, 0, 0) }) else { return };
    let Some(collector) = Collector::new(mapping.completions()) else { return };
    reached(mapping.layout().entries());

    match collector.take() {
        Ok(Some(_)) => kani::cover!(true, "a completion comes back"),
        Ok(None) => kani::cover!(true, "the ring reads empty"),
        Err(_) => kani::cover!(true, "the cursors are refused"),
    }
}

/// `Producer::submit` and `Producer::occupancy` against every cursor pair.
///
/// The producing half is peer-facing too: `tail` is the consumer's and a
/// service that has been compromised advances it however it likes. What is
/// asserted is the sentence `submit`'s own comment makes — occupancy never
/// exceeds capacity, and a ring that is full is refused rather than
/// overwritten.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn submitting_against_an_arbitrary_cursor() {
    let mut region = Region::scribbled();
    let len = any_len();
    let base = region.base();
    // SAFETY: as `adopting_arbitrary_bytes`.
    let Ok(mapping) = (unsafe { Mapping::adopt(base, len, 0, 0) }) else { return };
    let Some(producer) = Producer::new(mapping.channel()) else { return };
    let capacity = mapping.layout().entries();
    reached(capacity);

    let before = producer.occupancy();
    if let Ok(used) = before {
        assert!(used <= capacity, "occupancy above capacity was answered rather than refused");
    }

    match producer.submit(any_sqe()) {
        Ok(_) => {
            assert!(
                before.is_ok_and(|used| used < capacity),
                "an entry was published into a ring that had no room"
            );
            kani::cover!(true, "an entry is published");
        }
        Err(_) => kani::cover!(true, "a submission is refused"),
    }
}

/// `Service::drain` over every channel, with a budget the solver chose.
///
/// The harness `mutate-unbounded-drain` has to break, and the property is the
/// one that defect exists to make failable: **the work this call does is the
/// caller's choice and never the peer's.** It is asserted as a count rather
/// than as a duration for `ring/tests/hostile.rs`'s reason — a timeout in a
/// checker is a bound on the checker — and here it is a count over every
/// channel rather than over the ones a generator drew.
///
/// The budget is not bounded. The cursors are not bounded. The slot numbers in
/// the index ring are not bounded, so the entry this drains may be any of them
/// or none. What *is* taken on trust, and only here, is that the regions are
/// where a `Mapping` would put them: [`peer::Rings`](crate::peer::Rings) says
/// what that buys and which four harnesses pay it back.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(8))]
fn draining_an_arbitrary_channel() {
    let rings = Rings::<RING>::scribbled();
    let Some(consumer) = Consumer::new(rings.channel()) else { return };
    let Some(poster) = Poster::new(rings.completions()) else { return };

    // The empty arena, which is legal and is what a channel whose opcodes carry
    // everything inline has. `write_serial` refuses any non-zero length against
    // it before reaching a loop, which is the division of labour this harness
    // wants: `executing_an_arbitrary_entry` proves `execute` over every entry
    // and every arena up to [`ARENA`], and this one proves the loop around it.
    let arena = Arena::new(&[]);
    let budget: u32 = kani::any();
    let now: u64 = kani::any();
    let mut service = Service::new(consumer, poster, arena, Bucket::any());

    match service.drain(budget, now) {
        Ok(done) => {
            assert!(done.executed <= budget, "a drain did more work than its budget");
            assert!(done.executed <= RING as u32, "a drain took more entries than the ring holds");
            assert!(done.completed <= done.executed, "more answers than questions");
            assert!(done.refused <= done.executed, "more refusals than entries");
            kani::cover!(done.executed > 0, "a drain executes something");
            kani::cover!(done.refused > 0, "a drain refuses something");
        }
        Err(_) => kani::cover!(true, "a drain refuses a corrupt channel"),
    }
}

/// `f_ring::execute` over every entry and every arena.
///
/// The fourth path the promise names, and the one whose refusals have an order:
/// the reserved word, then the undefined flag bits, then the opcode. The order
/// is asserted rather than the set, because reporting the opcode first would
/// tell a caller its opcode was wrong when it was not — which is the mistake
/// `execute`'s own comment names and the one a proof over the set alone would
/// not catch.
///
/// The harness `mutate-ignored-flag` has to break: that defect masks the
/// undefined bits off and runs the entry, so the equivalence below stops
/// holding in the direction a fuzzer watching for panics cannot see.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(16))]
fn executing_an_arbitrary_entry() {
    let entry = any_sqe();
    let bytes = Bytes::<ARENA>::scribbled();
    let arena = Arena::new(bytes.cells());
    let mut sink = Bucket::any();
    let now: u64 = kani::any();

    let answer = execute(&entry, &arena, &mut sink, now);

    let unknown_flags = entry.flags & !flags::KNOWN;
    let expected = if entry._reserved != 0 {
        Some(error::argument::RESERVED_NOT_ZERO)
    } else if unknown_flags != 0 {
        Some(error::argument::UNKNOWN_FLAG)
    } else if !op::known(entry.opcode) {
        Some(error::argument::UNKNOWN_OPCODE)
    } else {
        None
    };

    match (expected, answer) {
        (Some(code), Some(cqe)) => {
            // `assert!` with a literal rather than `assert_eq!`, because this
            // is one of the messages `cargo xtask prove` matches a *failing*
            // check against: an `assert_eq!` puts the two values in the
            // description and the sentence stops being a fixed string.
            assert!(
                cqe.error() == Some((error::ARGUMENT, code)),
                "the envelope is checked in the wrong order, or with the wrong list"
            );
            assert_eq!(cqe.user_data, entry.user_data, "a refusal that answers no request");
        }
        (Some(_), None) => {
            panic!("a malformed entry was swallowed: a refusal always completes");
        }
        (None, Some(cqe)) => {
            // A well-formed entry: it ran. It may still refuse, but only for a
            // reason the operation itself found.
            assert_eq!(cqe.user_data, entry.user_data, "a completion that answers no request");
            if let Some((domain, code)) = cqe.error() {
                assert_eq!(domain, error::ARGUMENT, "R07: a refusal names its domain");
                assert_eq!(
                    code,
                    error::argument::BAD_ADDRESS,
                    "an operation refused for no reason"
                );
            } else {
                assert!(cqe.result >= 0, "a success with a negative result");
                assert!(cqe.result as u32 <= entry.len, "more bytes written than were asked for");
            }
            kani::cover!(true, "a well-formed entry completes");
        }
        (None, None) => {
            assert_ne!(entry.flags & flags::NO_CQE, 0, "an entry with no answer and no flag");
            kani::cover!(true, "an entry asks for no completion and gets none");
        }
    }

    kani::cover!(expected == Some(error::argument::UNKNOWN_FLAG), "an unknown flag is refused");
    kani::cover!(entry.opcode == op::WRITE_SERIAL, "the one opcode with a payload is reached");
}

// ---------------------------------------------------------------------------
// 4. Registration, which RFC 0028 added after the task was written.
// ---------------------------------------------------------------------------
//
// **The fourth bound, and it belongs to this section rather than to a
// constant.** Every harness below drives a *freshly built* table through at
// most one registration and then one further operation, with everything about
// both — the entry, the id, the index, the geometry — the solver's. So the
// quantifier is over inputs and not over histories: what is proved is that no
// single operation on a table in one of the states one registration can reach
// panics or answers outside its registration.
//
// What that leaves out is a defect that needs two: a second `register` that
// leaks a translation, a slot refilled after the first occupant left, a
// half-registration visible only when the lowest free slot is not slot zero.
// `SLOTS` is two so that *which* slot is a question, and at depth one nothing
// asks it — `assert!(table.live() == 0)` on a refusal arm is a tautology about
// a table that was empty a line above.
//
// It is a bound rather than an oversight and it is the same shape as the
// others: depth costs what the ring size costs, because a checker unrolls a
// second operation rather than summarising it, and `mutate-reusable-slot` in
// `RING_PROOF_BLIND` is the same argument at sixty-five thousand. What covers
// histories instead is `ring/tests/entries.rs`, which keeps a ledger of every
// id a table has issued across a run, and `ring/tests/hostile.rs`, which drives
// a billion of them. RFC 0057.

/// `Request::read` over every entry.
///
/// The entry point RFC 0028 added, and it is the one place an envelope is
/// checked *instead of* `execute` rather than after it — so the same
/// equivalence is asserted here as there, and a build where the two lists
/// disagreed would fail one of the two.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn reading_an_arbitrary_registration() {
    let entry = any_sqe();
    let read = Request::read(&entry);

    let unknown_flags = entry.flags & !flags::KNOWN;
    if entry._reserved != 0 {
        assert!(read.is_err(), "a reserved word a peer set was read past");
    } else if unknown_flags != 0 {
        assert!(read.is_err(), "R04: an unknown flag was ignored rather than refused");
    } else if !f_abi::buf::opcode::is_registration(entry.opcode) {
        assert!(read.is_err(), "an opcode that is not a registration was read as one");
    }

    match read {
        Ok(Request::Register { buffers, .. }) => {
            assert_eq!(entry.opcode, f_abi::buf::opcode::REGISTER);
            assert_eq!(entry.flags & flags::FIXED_BUF, 0, "a registration naming a set");
            assert_eq!(u64::from(buffers), entry.ext[0], "a buffer count that was truncated");
            kani::cover!(true, "a registration is read");
        }
        Ok(Request::Unregister { set }) => {
            assert_eq!(entry.opcode, f_abi::buf::opcode::UNREGISTER);
            assert_ne!(entry.flags & flags::FIXED_BUF, 0, "an unregistration naming no set");
            assert!(set.is_issuable(), "an id nobody could have issued was read as one");
            kani::cover!(true, "an unregistration is read");
        }
        Err((packed, _)) => {
            assert!(error::unpack(packed).is_some(), "R07: a refusal names its domain");
            kani::cover!(true, "a registration entry is refused");
        }
    }
}

/// `Name::read` over every entry and every negotiated feature set.
///
/// Both readings of the twelve bytes at offset 32, and the rule that decides
/// which: a virtual address on a channel that did not negotiate shared virtual
/// memory is an entry using a feature outside the agreed set, which is the one
/// thing `Negotiated` says a peer must not do.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn reading_an_arbitrary_buffer_name() {
    let entry = any_sqe();
    let features: u64 = kani::any();

    match Name::read(&entry, features) {
        Ok(Name::Registered { set, index }) => {
            assert_ne!(entry.flags & flags::FIXED_BUF, 0, "a registered name with the flag clear");
            assert!(set.is_issuable(), "an id nobody could have issued was read as one");
            assert_eq!(index, entry.buf_index);
            kani::cover!(true, "a registered name is read");
        }
        Ok(Name::Virtual { address }) => {
            assert_eq!(entry.flags & flags::FIXED_BUF, 0, "an address with the flag set");
            assert_ne!(
                features & feature::SHARED_VIRTUAL_MEMORY,
                0,
                "an address on a channel that did not agree to one"
            );
            assert_ne!(address, 0, "a null address was read as a buffer");
            kani::cover!(true, "an address is read");
        }
        Err((packed, _)) => {
            assert!(error::unpack(packed).is_some(), "R07: a refusal names its domain");
            kani::cover!(true, "a name is refused");
        }
    }
}

/// `SetId::from_completion` over every completion a service could post.
///
/// The one place a *client* believes a service, which is the direction the
/// hostile-peer fuzzer reaches least: it drives a hostile producer against an
/// honest service, and this is the mirror.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn believing_an_arbitrary_completion() {
    let cqe = any_cqe();

    match SetId::from_completion(&cqe) {
        Ok(set) => {
            assert!(set.is_issuable(), "an id nobody could have issued was believed");
            assert!(!cqe.is_error(), "an id was read out of a refusal");
            kani::cover!(true, "an id is believed");
        }
        Err(_) => kani::cover!(true, "a completion carrying no id is refused"),
    }

    // The client's type for the same value, which RFC 0028 left with this as
    // its only constructor. The two must agree, because a `Fixed` that could be
    // built where a `SetId` could not would be the compile-time fence removed
    // at run time.
    assert_eq!(
        Fixed::from_completion(&cqe).is_ok(),
        SetId::from_completion(&cqe).is_ok(),
        "the client's type and the wire type disagree about what an id is"
    );
}

/// `registry::Table::execute` over every entry, against a table that answers.
///
/// The service side of RFC 0028, driven the way a service drives it: one
/// arbitrary entry into `execute`, a domain that refuses or answers as the
/// solver chooses, and the invariant that says the two stayed in step —
/// **a translation is outstanding exactly when a slot is live**, which is what
/// a half-registration would break and what `register`'s comment promises by
/// asking for the translation before filling the slot.
/// **Depth one**, which the section comment above argues: one `execute`
/// against a table built a line earlier, so a defect needing a second
/// registration is outside this and inside `ring/tests/entries.rs`.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn registering_from_an_arbitrary_entry() {
    let mut table: Table<SLOTS> = Table::new();
    let mut domain = Domain::empty();
    let now: u64 = kani::any();

    let answered = table.execute(&any_sqe(), &mut domain, now);

    // `execute` answers a completion whatever happens: a registration always
    // completes, because a client that registered and was not told the id holds
    // a set it cannot name.
    assert!(
        i32::try_from(table.live()).is_ok_and(|live| live == domain.outstanding),
        "a registration and its translation are out of step"
    );
    assert!(table.live() <= table.capacity(), "more live sets than slots");
    assert!(table.retired() <= table.capacity(), "more retired slots than slots");

    if answered.is_error() {
        assert!(table.live() == 0, "a refused registration filled a slot");
        kani::cover!(true, "a registration entry is refused");
    } else {
        kani::cover!(true, "a registration entry is answered");
    }
}

/// `Table::resolve` and `Table::release` over every id and every index.
///
/// **The harness `mutate-lenient-index` has to break.** That defect drops the
/// `index >= slot.buffers` check and leaves the mask, so an index past the end
/// of a set resolves — and the sentence below stops holding: *what a resolve
/// answers is inside the registration it names*. RFC 0048 calls this the reach
/// oracle and asserts it over drawn cases; this asserts it over all of them.
///
/// The geometry is the solver's too, so this is not one registration with an
/// arbitrary index but every registration `register` will make.
/// **Depth one**: one registration, then one resolve. See the section
/// comment for what a second would add and what it would cost.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn resolving_an_arbitrary_buffer_name() {
    let mut table: Table<SLOTS> = Table::new();
    let mut domain = Domain::empty();

    let len: u32 = kani::any();
    let buffers: u32 = kani::any();
    // The geometry, and only the geometry. See [`REGISTERED`] for why the
    // stride is bounded and the index below is not.
    kani::assume(len <= REGISTERED && buffers <= SETS);
    let Ok(set) = table.register(kani::any(), len, buffers, &mut domain) else { return };

    // The registration succeeded, so its geometry is one `register` admits.
    // Asserted rather than assumed: these are the bounds every line below
    // rests on, and a `register` that stopped enforcing one of them would make
    // this harness prove something about a set that cannot exist.
    assert!(buffers > 0 && buffers <= BUFFERS_MAX, "a set with an impossible buffer count");
    assert!(len > 0 && len % buffers == 0, "a region that does not divide into its buffers");
    let stride = len / buffers;

    let named = any_set();
    let index: u32 = kani::any();
    let asked: u32 = kani::any();

    match table.resolve(named, index, asked) {
        Ok(reach) => {
            assert_eq!(named, set, "a resolve answered for an id this table never issued");
            assert!(index < buffers, "a buffer past the end of the set was resolved");
            assert!(reach.len <= stride, "more of a buffer was answered for than it holds");
            let offset = u64::from(index) * u64::from(stride);
            assert_eq!(reach.address, DEVICE_BASE + offset, "an address outside the registration");
            assert!(
                reach.address + u64::from(reach.len) <= DEVICE_BASE + u64::from(len),
                "a reach that runs past the end of the registration"
            );

            // Given back, it is free again; given back twice, it is refused.
            assert!(table.release(named, index).is_ok(), "a lent buffer could not be given back");
            assert!(table.release(named, index).is_err(), "a buffer was given back twice");
            kani::cover!(true, "a buffer is resolved");
        }
        Err((packed, _)) => {
            assert!(error::unpack(packed).is_some(), "R07: a refusal names its domain");
            kani::cover!(true, "a buffer name is refused");
        }
    }
}

/// `Table::unregister` and `Table::retire_all` over every id.
///
/// The generation is the whole of this, and the failure it prevents is the one
/// RFC 0028 rejected a plain index to avoid: an id from before a teardown
/// resolving into whatever occupies the slot now. What is asserted is the
/// sentence with the sampling removed — after a retirement **no** id resolves,
/// over all 2^32 of them, and the translation went with it.
/// **Depth one**: one registration, then one retirement. The ids after it
/// are all 2^32; the *histories* before it are one.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn retiring_an_arbitrary_set() {
    let mut table: Table<SLOTS> = Table::new();
    let mut domain = Domain::empty();

    let len: u32 = kani::any();
    let buffers: u32 = kani::any();
    kani::assume(len <= REGISTERED && buffers <= SETS);
    let Ok(set) = table.register(kani::any(), len, buffers, &mut domain) else { return };
    assert_eq!(domain.outstanding, 1, "a live registration with no translation behind it");

    let named = any_set();
    if table.unregister(named, &mut domain).is_ok() {
        assert_eq!(named, set, "an id this table never issued retired a set");
        assert_eq!(domain.outstanding, 0, "a retired registration kept its translation");
        assert_eq!(table.live(), 0, "a retired set is still live");

        let after = any_set();
        assert!(table.resolve(after, kani::any(), kani::any()).is_err(), "a stale id resolved");
        assert!(table.unregister(after, &mut domain).is_err(), "a stale id retired something");
        kani::cover!(true, "a set is retired");
    } else {
        assert_ne!(named, set, "the id this table issued was refused");
        kani::cover!(true, "a stale id is refused");
    }
}

/// The two transports answer the same refusals over every name.
///
/// `registry::Transport` is what the two paths differ in, and RFC 0028 rests a
/// comparison on the claim that everything *else* about them is the same. This
/// is that claim over arbitrary names: a name of the wrong kind is refused by
/// both, and neither panics on one.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn both_transports_refuse_a_name_of_the_wrong_kind() {
    let mut table: Table<SLOTS> = Table::new();
    let mut domain = Domain::empty();
    let len: u32 = kani::any();
    let buffers: u32 = kani::any();
    kani::assume(len <= REGISTERED && buffers <= SETS);
    let _ = table.register(kani::any(), len, buffers, &mut domain);

    let entry = any_sqe();
    let features: u64 = kani::any();
    let Ok(name) = Name::read(&entry, features) else { return };

    let mut registered = registry::Registered::bind(
        f_abi::Negotiated { version: ABI_VERSION, features },
        &mut table,
    )
    .expect("the registered path requires no feature");
    let first = registered.resolve(name, kani::any());
    if matches!(name, Name::Virtual { .. }) {
        assert!(first.is_err(), "an address resolved against a registration table");
        assert!(registered.release(name).is_err(), "an address was released by a table");
    }

    let walk = Walk;
    if let Ok(mut virt) =
        registry::SharedVirtual::bind(f_abi::Negotiated { version: ABI_VERSION, features }, &walk)
    {
        let second = virt.resolve(name, kani::any());
        if matches!(name, Name::Registered { .. }) {
            assert!(second.is_err(), "a set id resolved on a path with no registration");
            assert!(virt.release(name).is_err(), "a set id was released on that path");
        }
        kani::cover!(true, "the virtual path is bound");
    }
}

/// A buffer goes out and comes back, over every completion a service could
/// post.
///
/// RFC 0024's client half, and the one part of it that is not a compile error:
/// `InFlight::complete` decides from a *peer-written* completion whether this
/// buffer is the one being returned, and both `Idle::submit` and `complete`
/// carry an `expect` whose unreachability is an argument in a comment. A proof
/// is where an argument like that becomes a check.
///
/// Two-sided, because *a buffer is never returned* satisfies every safety
/// property here: the buffer comes back **exactly** when the completion carries
/// its token and is the last one for it.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(12))]
fn lending_a_buffer_over_an_arbitrary_completion() {
    // The naming has to come out of a service's completion, because RFC 0028
    // took `Fixed`'s public constructor away. So this is two arbitrary
    // completions: the one that issued the set, and the one that answers.
    let issued = any_cqe();
    let Ok(fixed) = Fixed::from_completion(&issued) else { return };
    let set = fixed.set();

    let mut bytes: [u8; LENT] = kani::any();
    let features: u64 = kani::any();
    let agreed = f_abi::Negotiated { version: ABI_VERSION, features };
    let Ok(mut owned) = BufferSet::bind(fixed, agreed, &mut bytes) else { return };
    let Ok([first, second]) = owned.carve::<2>() else { return };
    assert_eq!(first.index(), 0, "a carve that did not number its buffers in order");
    assert_eq!(second.index(), 1, "a carve that did not number its buffers in order");
    assert_eq!(first.len(), LENT / 2, "a carve that did not divide the region evenly");

    let mut lane = Lane::any();
    let entry = any_sqe();
    let asked = entry.len;

    match first.submit(&mut lane, entry) {
        Ok((lent, _)) => {
            assert!(asked as usize <= LENT / 2, "an entry longer than the buffer was submitted");
            let written = lane.last.expect("a submission the lane did not see");
            // The name is written over whatever the caller put there, which is
            // what *a buffer names memory its set covers* means on this side.
            assert_ne!(
                written.flags & flags::FIXED_BUF,
                0,
                "a registered buffer named as an address"
            );
            assert_eq!(written.buf_set, set.bits(), "a buffer named against another set");
            assert_eq!(written.buf_index, 0, "a buffer named as one it is not");

            let answer = any_cqe();
            let returns =
                answer.user_data == written.user_data && answer.flags & f_abi::cflags::MORE == 0;
            match lent.complete(&answer) {
                Ok(idle) => {
                    assert!(returns, "a completion that answers somebody else returned a buffer");
                    assert_eq!(idle.index(), 0, "the wrong buffer came back");
                }
                Err(still) => {
                    assert!(!returns, "the completion that returns this buffer did not");
                    // Consumed rather than dropped: `InFlight`'s drop is a bomb,
                    // and a harness that let one fall would be proving that the
                    // bomb works rather than that the path does.
                    let gone =
                        PeerGone::of(RingError::EpochChanged).expect("an epoch change is evidence");
                    let _ = still.reclaim(gone);
                    kani::cover!(true, "a buffer is reclaimed from a peer that went");
                }
            }
            kani::cover!(true, "a buffer goes out");
        }
        Err((_, idle)) => {
            assert!(
                asked as usize > LENT / 2 || lane.refuses,
                "a submission was refused for neither length nor a full ring"
            );
            let _ = idle;
            kani::cover!(true, "a submission is handed back with its buffer");
        }
    }
}

// ---------------------------------------------------------------------------
// 5. The granted window, which is the other place a bound is arithmetic.
// ---------------------------------------------------------------------------

/// `Window::slice` and `Region::slice` narrow and never widen.
///
/// RFC 0033's property, and the reason it is in this file: a driver hands a
/// sub-window to another part of itself, so *a sub-window cannot name a byte
/// the whole one did not* is what makes that safe to do at all. It is
/// arithmetic over three symbolic `u64`s and two `u32`s and costs almost
/// nothing to prove, which is the argument for proving it rather than testing
/// three offsets.
#[cfg_attr(kani, kani::proof)]
#[cfg_attr(kani, kani::unwind(20))]
fn narrowing_a_granted_window() {
    let base: u64 = kani::any();
    let len: u32 = kani::any();
    let offset: u32 = kani::any();
    let taken: u32 = kani::any();

    if let Ok(window) = Window::at(base, len) {
        assert_eq!(window.len(), len);
        if let Ok(inner) = window.slice(offset, taken) {
            assert_eq!(inner.len(), taken, "a slice is not the length it was asked for");
            assert!(u64::from(offset) + u64::from(taken) <= u64::from(len), "a slice that widens");
            kani::cover!(true, "a window narrows");
        }
        // Every accessor refuses an offset outside the window rather than
        // reading past it. Not dereferenced here: `Window::at`'s contract is
        // that the frame mapped the address, and a harness that invented one
        // and read it would be proving something about its own pointer.
        assert!(window.read8(len).is_err(), "the byte one past the end was readable");
    }

    let device: u64 = kani::any();
    if let Ok(region) = DeviceRegion::at(base, device, len) {
        if let Ok(inner) = region.slice(offset, taken) {
            assert_eq!(inner.len(), taken, "a slice is not the length it was asked for");
            assert!(u64::from(offset) + u64::from(taken) <= u64::from(len), "a slice that widens");
        }
        if let Ok(address) = region.device_at(offset) {
            assert!(offset < len, "a device address was computed past the end of the grant");
            assert_eq!(address, device.wrapping_add(u64::from(offset)));
            kani::cover!(true, "a device address is answered");
        }
        assert!(region.device_at(len).is_err(), "a device address one past the end was answered");
    }
}
