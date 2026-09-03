// SPDX-License-Identifier: Apache-2.0 OR MIT
//! A hostile peer, generated rather than written down.
//!
//! `E1-P04`. `ring/tests/headers.rs` drives fifteen hand-written hostile
//! headers into a real region and requires each to be refused with the domain
//! and code RFC 0010 names. This is the same idea at nine orders of magnitude
//! more attempts, and the difference is not the count: a hand-written case is a
//! case somebody thought of, and the bugs that survive review are the ones
//! nobody thought of. So the peer's behaviour is *drawn* — arbitrary bytes in
//! the header and the reserved words, cursors that run backwards, jump, exceed
//! the ring, wrap or point into the arena, an epoch that is stale or from the
//! future or changes between two halves of one operation, a peer that stops
//! answering in the middle of a batch and restarts, and entries whose lengths
//! and offsets address outside the arena.
//!
//! # The three properties, and why they are three
//!
//! **No panic.** `f_ring`'s module comment promises that nothing a peer writes
//! produces one. It is caught here per episode, so a failure names a seed
//! rather than a backtrace.
//!
//! **No memory unsafety.** Not observable from inside the program at all, so it
//! is not asserted here: it is Miri's verdict over the same code and the same
//! generator at a much smaller count. RFC 0046 states the split and both
//! numbers, because one figure covering both would be a claim about the
//! property Miri checked and the count it never reached.
//!
//! **No hang.** Deliberately not a timeout. A timeout in a test is a flake
//! generator, and one that fires on a slow runner says nothing about the code.
//! What is asserted instead is a *work bound*: every loop on the peer-facing
//! path has a ceiling that is a function of the channel's geometry and never of
//! anything the peer wrote, and this file counts the work each operation
//! actually does and refuses it against that ceiling. See [`Bounded`].
//!
//! # One thread, and what that costs
//!
//! The peer and the honest end are the same thread, so a hostile write lands at
//! an operation boundary and not inside a call — with one deliberate exception,
//! [`Peer::batch`], which restarts the peer while a batch is staged and
//! unpublished. That is `PEER_GAP` and RFC 0046 declares it rather than leaving
//! it to be noticed: the window inside `Consumer::pop` is empty because the
//! entry is copied out before any field is read, which is a property of the
//! code and not something asserted here. Closing it means a second thread, and
//! the property would then be *no data race* and the instrument Miri's
//! preemption rather than a counter.
//!
//! # The shape of a run, and why it is episodes
//!
//! A run is `episodes * STEPS` operations. An episode is a freshly zeroed
//! region, a sound header, and [`STEPS`] drawn operations against it; its
//! stream is derived from `(seed, episode index)` by identity, so episode
//! 700 000 is the same episode whatever ran before it. That is RFC 0026's
//! split-by-identity spent on the one thing a fuzzer needs and a chained
//! generator cannot give: **a finding at operation 999 999 999 reproduces in
//! one episode's worth of work** rather than in the whole run. The
//! reproduction a finding prints is `--seed <base> --episode <k>`, and it
//! stands alone.
//!
//! What an episode boundary costs is stated rather than hidden: a peer cannot
//! carry a corruption across one, so a bug needing more than [`STEPS`]
//! operations of accumulated damage is not reachable here. That is the price of
//! the reproduction being short, and `STEPS` is the knob.
//!
//! # Why the counters are half the file
//!
//! Because a fuzzer that reached nothing reports the same two words as one that
//! reached everything. [`Reach`] counts each path the run touched, and
//! `claims/0008-hostile-peer-operations.toml` puts a **minimum** on the ones
//! that say the interesting paths were reached — so a generator that stopped
//! producing malformed headers, or a region that spent every episode refused
//! with nothing behind it, fails exactly as a panic does. That is the answer to
//! *what input would make this green while the property was false*.
//!
//! # No clock
//!
//! Nothing here reads one. `cargo xtask lint-determinism` scans `ring/` with no
//! allow-list entry, so it could not — and the wall-clock cost of a run is
//! `xtask`'s to print, beside the report and never inside it. `f-sim` and
//! `cargo xtask sweep` split the same way and RFC 0040 is where that split is
//! argued.

use std::panic::AssertUnwindSafe;

use f_abi::layout::Layout;
use f_abi::{CHANNEL_MAGIC, ChannelHeader, Cqe, Sqe, error, feature};
use f_env::split::{Stream, derive};
use f_ring::{
    Adopted, Client, Consumer, Drained, Mapping, Poster, Producer, RingError, Server, Service,
    Sink, execute,
};

/// Entries in each ring of the fuzzed channel. Unit: entries.
///
/// Eight, for `ring/tests/headers.rs`'s reason and one more. A 4 KiB region
/// holds both rings and an arena at that size and the arithmetic stays small
/// enough to check by hand; and a ring this small means a drawn cursor lands
/// *inside* it often rather than almost never, so the in-range hostile cases
/// are exercised as hard as the wild ones. A ring of 4096 entries would make
/// every drawn cursor a wild one, and the wild ones are the easy half.
const ENTRIES: u32 = 8;

/// The region a channel is bound in. Unit: bytes. One frame, which is the unit
/// the kernel shares.
const LEN: u32 = 4096;

/// Operations in one episode. Unit: operations.
///
/// A thousand and twenty-four, and the number is a trade with two ends. Longer
/// episodes let a corruption accumulate further before anything looks at it;
/// shorter ones make a reproduction cheaper. This is where a reproduction costs
/// about a millisecond and a peer still gets to interleave hundreds of hostile
/// writes with hundreds of honest operations against the wreckage.
const STEPS: u32 = 1024;

/// How many entries one drain is allowed to take. Unit: entries.
///
/// The budget is the whole reason [`Service::drain`] has one — it is what makes
/// the time a drain takes a property of the caller rather than of the peer — so
/// this is also the bound the hang property is asserted against.
///
/// **Below [`ENTRIES`] on purpose.** A budget at or above the ring's capacity is
/// never the binding constraint: a full ring holds eight entries, so a drain
/// would stop because it ran out of work rather than because it ran out of
/// budget, and the assertion would be about the geometry instead of about the
/// loop. `mutate-unbounded-drain` is the defect that says so — with the budget
/// at sixteen it cannot be caught at all, and with it at four it is caught on
/// the first full ring.
const BUDGET: u32 = 4;

/// The seed a run uses when it is not told one.
///
/// The tree's own, so a run named in a commit message and a run named in
/// `xtask` are the same run. `TRACE_SEED` in `xtask/src/main.rs` is this number
/// for this reason.
const DEFAULT_SEED: u64 = 0xf00d_beef_cafe_1234;

/// Operations one run performs when it is not told a count. Unit: operations.
///
/// The per-commit gate, and it is deliberately not the exit's billion:
/// `cargo test --workspace` builds this unoptimised, and
/// `claims/0008-hostile-peer-operations.toml` carries both numbers and the
/// argument for why the larger one is a nightly rather than a gate.
const DEFAULT_OPS: u64 = 1 << 21;

// ---------------------------------------------------------------------------
// The shared region.
// ---------------------------------------------------------------------------

/// The bytes both ends reach, and neither owns exclusively.
///
/// # Why this is an allocation and not a Rust value
///
/// Because a channel region is not one. It is a page the frame mapped, and the
/// two ends reach it through raw addresses; a `Box<[u8; 4096]>` would give the
/// test a *unique* handle to memory whose whole purpose is to be shared, and
/// every access through it would invalidate the pointer the honest end is
/// holding. `f_ring::Mapping` hands out atomics and `UnsafeCell`s rather than
/// references for exactly that reason, and this is the far side of the same
/// arrangement: one allocation, one pointer, and no Rust reference to the bytes
/// anywhere.
///
/// It is what makes the interesting case reachable at all — a peer writing
/// *during* an operation the honest end has already begun.
struct Region {
    /// The one pointer everything reaches these bytes through, taken once from
    /// the allocator.
    ///
    /// # Why once, and why this is not a micro-optimisation
    ///
    /// Deriving a fresh pointer from a `&Page` on every access is what the
    /// first version of this file did, and it is correct — and it made the Miri
    /// run of one episode take **three minutes**. Each derivation retags the
    /// whole four-kibibyte allocation under Stacked Borrows, so a byte poke
    /// cost four thousand bookkeeping operations and `clear` cost sixteen
    /// million. The unsafety property is checked by a tool that costs six
    /// orders of magnitude, so what that tool can reach in a CI job is decided
    /// by how many *aliasing events* this file generates and not by how many
    /// operations it performs. One pointer for the life of the run is the
    /// difference between one episode under Miri and a hundred.
    base: *mut u8,
}

/// The allocation one region is: 4 KiB at a page alignment.
///
/// A page rather than the cache line [`Mapping`] needs, deliberately: the fixed
/// regions are placed on lines measured from the first byte, so a base that is
/// not itself line-aligned makes every one of those offsets wrong — and a page
/// is the unit the frame actually grants.
fn page_layout() -> std::alloc::Layout {
    std::alloc::Layout::from_size_align(LEN as usize, 4096).expect("4 KiB at a page alignment")
}

impl Region {
    /// A zeroed region, at an address that outlives every mapping over it.
    ///
    /// Allocated rather than boxed, and freed rather than leaked. Both halves
    /// are load-bearing. Boxed, the pointer would be derived from a box's
    /// unique tag and every later move of that box would invalidate it under
    /// Miri — which is the tool this file exists to be run under. Leaked,
    /// Miri's own leak check would report the page on every run, and a suite
    /// that has to be told to ignore leaks cannot notice one in the code under
    /// test.
    fn new() -> Self {
        // SAFETY: `page_layout` has a non-zero size and a power-of-two
        // alignment, which is the whole of `alloc_zeroed`'s obligation.
        let base = unsafe { std::alloc::alloc_zeroed(page_layout()) };
        assert!(!base.is_null(), "the host could not spare one page for a channel");
        Self { base }
    }

    /// The first byte, as the address a mapping is stated against.
    const fn base(&self) -> *mut u8 {
        self.base
    }

    /// The base as the integer [`Adopted::at`] takes.
    ///
    /// # The one assumption a reproduction rests on
    ///
    /// This is an allocator address: it differs between two processes, and
    /// nothing in a seed determines it. So this file's central promise — *a
    /// finding is a seed and reproduces* — holds only while no observable
    /// behaviour depends on the numeric value of the base. Today none does:
    /// what `Adopted::at` reads out of it is alignment and length, and both are
    /// properties of the page rather than of where the page landed, which is
    /// why the only refusals `Peer::adopt_askew` reaches are those two.
    ///
    /// It is written down rather than left as a fact about today, because it is
    /// exactly the sort of dependency a later change adds without noticing. The
    /// reversal condition: a finding that does not reproduce in a second process
    /// at the same seed and episode. The fix would then be a base drawn from the
    /// stream and mapped, rather than one taken from the allocator.
    fn address(&self) -> u64 {
        self.base as u64
    }

    /// Write one byte, the way a peer writes one.
    ///
    /// Volatile, because this is the far end of memory something else is
    /// reading and the compiler may not decide the write is dead.
    fn poke(&self, at: u32, byte: u8) {
        let at = (at % LEN) as usize;
        // `wrapping_add` and not `add`, so the offset arithmetic is safe code
        // and the one unsafe operation here is the write itself. It is also
        // exactly equivalent: `at` was masked into the allocation on the line
        // above, so no wrap can happen.
        //
        // SAFETY: `at` is inside the region, which is `LEN` bytes this value
        // owns from `new` until `drop`. No Rust reference to these bytes exists
        // — the honest end's `Mapping` hands out `UnsafeCell` and atomics,
        // never a plain `&` — so a write through the base pointer cannot alias
        // one.
        unsafe { self.base.wrapping_add(at).write_volatile(byte) };
    }

    /// Read four bytes back, little-endian, the way the peer sees a cursor.
    fn read32(&self, at: u32) -> u32 {
        let mut bytes = [0u8; 4];
        for (i, byte) in bytes.iter_mut().enumerate() {
            let at = ((at as usize + i) % LEN as usize) as u32;
            // SAFETY: masked into the region, which is `LEN` initialised bytes
            // this value owns. `wrapping_add` for `poke`'s reason. Volatile
            // because the honest end writes these words through an atomic, and
            // this read may not be merged with another or elided.
            *byte = unsafe { self.base.wrapping_add(at as usize).read_volatile() };
        }
        u32::from_le_bytes(bytes)
    }

    /// Write four bytes little-endian, which is how every cursor and every
    /// index-ring slot is laid out.
    fn poke32(&self, at: u32, value: u32) {
        for (i, byte) in value.to_le_bytes().into_iter().enumerate() {
            #[allow(clippy::cast_possible_truncation)]
            self.poke(at + i as u32, byte);
        }
    }

    /// Place a header the way a peer does: straight into the first cache line,
    /// without going through [`Mapping::describe`], which by construction can
    /// only produce sound ones.
    fn place(&self, header: ChannelHeader) {
        // SAFETY: the region is page-aligned, which is stronger than the 64
        // bytes a `ChannelHeader` needs, and the header is the first 64 bytes
        // of `LEN`. Volatile and unaliased for `poke`'s reasons.
        unsafe { self.base.cast::<ChannelHeader>().write_volatile(header) };
    }

    /// Zero every byte, which is what a freshly granted frame looks like.
    ///
    /// One write of the whole page rather than four thousand of one byte, for
    /// the reason [`Region::base`] gives: what this file costs under Miri is
    /// the number of memory events it generates, and an episode boundary should
    /// not be the most expensive thing in an episode.
    fn clear(&self) {
        // SAFETY: `LEN` bytes from the base is exactly the allocation, and an
        // episode boundary is between two mappings rather than inside one, so
        // nothing derived from it is in use here.
        unsafe { self.base.write_bytes(0, LEN as usize) };
    }
}

impl Drop for Region {
    fn drop(&mut self) {
        // SAFETY: `base` came from `alloc_zeroed` with this exact layout and
        // has not been freed. Every `Mapping` and every `Adopted` over it is
        // gone by now: the run's last episode ends before `main` returns, and
        // neither type owns anything that outlives its own call.
        unsafe { std::alloc::dealloc(self.base, page_layout()) };
    }
}

/// A header a well-behaved peer at `epoch` writes: this build's own layout,
/// described.
///
/// Derived from [`Layout`] rather than written out by hand, so a change to the
/// wire layout moves the sound header with it instead of leaving a stale
/// literal that every hostile case is measured against. `headers.rs` takes the
/// same care and for the same reason.
fn sound(epoch: u32) -> ChannelHeader {
    Layout::new(ENTRIES, 0).expect("eight entries is a layout").describe(epoch, 0, 0)
}

/// Where the believed layout puts each region of the mapping.
///
/// Computed once from this build's own arithmetic rather than read back out of
/// the header, because the header is the thing under test: a fuzzer that aimed
/// its pokes using the peer's numbers would stop aiming at the entry array the
/// moment the peer lied about where it was.
struct Where {
    /// First byte of the submission index ring. Unit: bytes from the base.
    index: u32,
    /// First byte of the submission entry array. Unit: bytes from the base.
    entries: u32,
    /// First byte of the inline arena. Unit: bytes from the base.
    arena: u32,
    /// Bytes of inline arena. Unit: bytes.
    arena_len: u32,
}

impl Where {
    /// The one this build computes for [`ENTRIES`] in [`LEN`] bytes.
    fn here() -> Self {
        let layout = Layout::adopt(&sound(0), LEN).expect("this build's own header");
        Self {
            index: layout.sq_index_offset(),
            entries: layout.sqe_offset(),
            arena: layout.arena_offset(),
            arena_len: layout.arena_len(),
        }
    }
}

/// The four cursor lines, as byte offsets from the base.
///
/// `f_abi::layout` states them and they are restated here as a list because
/// what this file needs is something to *draw from*, and a draw over a list is
/// the one operation the constants themselves do not offer.
const CURSOR_LINES: [u32; 4] = [64, 128, 192, 256];

/// The consumer's flag word: four bytes into the consumer's own cursor line.
const FLAGS_WORD: u32 = CURSOR_LINES[1] + 4;

// ---------------------------------------------------------------------------
// The hang bound.
// ---------------------------------------------------------------------------

/// A sink that counts what it is asked to do, and stops when the geometry says
/// it has been asked for too much.
///
/// # Why a counter and not a timeout
///
/// There are exactly three loops on the peer-facing path of this crate, and
/// each has a ceiling that is a function of the channel's geometry rather than
/// of anything a peer wrote:
///
/// - [`Service::drain`] runs `budget` times and no more. Asserted against
///   [`Drained::executed`].
/// - `write_serial` copies the arena in fixed-size pieces, and the range it may
///   copy was already refused unless it lies wholly inside the arena — so it
///   offers at most `arena_len` bytes, in at most `arena_len` calls, because a
///   piece is at least one byte. Asserted here.
/// - `Arena::copy_out` walks the slice it was handed, which is one of those
///   pieces.
///
/// So *stuck* is a count and not a duration: an operation that did more work
/// than the geometry permits is a finding with a seed, on a fast machine and a
/// slow one alike. And the refusal is load-bearing rather than decorative — a
/// short answer is what makes `write_serial` stop, so a sink that stops at the
/// bound turns a hypothetical unbounded loop into a terminating one that
/// reports itself, instead of into a job somebody kills at the timeout.
///
/// What this does not catch is a loop that never calls out and never returns.
/// No counter in the caller can, and RFC 0046 declares that residual rather
/// than leaving it to be discovered.
struct Bounded {
    /// Bytes accepted. Unit: bytes.
    taken: u64,
    /// Calls taken. Unit: calls.
    calls: u64,
    /// What the geometry permits: bytes, and also calls, because a call takes
    /// at least one byte.
    bound: u64,
    /// Set when the bound was passed. Read by the caller, which turns it into a
    /// finding.
    over: bool,
}

impl Bounded {
    /// A sink armed for one operation's worth of work.
    const fn armed(bound: u64) -> Self {
        Self { taken: 0, calls: 0, bound, over: false }
    }
}

impl Sink for Bounded {
    fn write(&mut self, bytes: &[u8]) -> usize {
        self.calls += 1;
        if self.taken >= self.bound || self.calls > self.bound {
            // Past the ceiling. Answering zero is a short write, which
            // `write_serial` reports as a partial completion and stops on — so
            // the loop terminates here rather than in whatever kills the job.
            self.over = true;
            return 0;
        }
        #[allow(clippy::cast_possible_truncation)]
        let take = bytes.len().min((self.bound - self.taken) as usize);
        self.taken += take as u64;
        take
    }
}

// ---------------------------------------------------------------------------
// What a run reports.
// ---------------------------------------------------------------------------

/// Every path this run reached, and how often.
#[derive(Default)]
struct Reach {
    /// Arbitrary bytes written into the 64-byte header. Unit: writes.
    header_bytes: u64,
    /// Named header fields set to drawn values. Unit: writes.
    header_fields: u64,
    /// Cursors set to a drawn value — backwards, jumped, past the ring, near
    /// the `u32` wrap, or arbitrary. Unit: writes.
    cursors: u64,
    /// Index-ring slots set to a drawn value. Unit: writes.
    index_slots: u64,
    /// Entry slots overwritten with arbitrary bytes. Unit: writes.
    entry_slots: u64,
    /// Arena bytes overwritten. Unit: writes.
    arena_bytes: u64,
    /// The consumer's flag word overwritten. Unit: writes.
    flag_words: u64,
    /// Peer restarts: a new epoch and a rewritten header. Unit: restarts.
    restarts: u64,
    /// Restarts that landed between a batch being staged and published, which
    /// is the exit's *restarts mid-operation*. Unit: restarts.
    restarts_mid_batch: u64,
    /// Re-adoptions that found an epoch other than the bound one — the peer
    /// lying about its epoch, noticed. Unit: observations.
    epoch_changes: u64,

    /// Adoptions that succeeded. Unit: adoptions.
    adopts_ok: u64,
    /// Adoptions refused with `ARGUMENT/MALFORMED_HEADER`. Unit: refusals.
    refused_header: u64,
    /// Adoptions refused with `ARGUMENT/BAD_ADDRESS`. Unit: refusals.
    refused_address: u64,
    /// Adoptions refused with `PEER/VERSION_UNSUPPORTED`. Unit: refusals.
    refused_version: u64,
    /// Adoptions refused with `PEER/FEATURE_REQUIRED`. Unit: refusals.
    refused_feature: u64,

    /// Submissions the ring accepted. Unit: entries.
    submitted: u64,
    /// Submissions refused because the ring was full. Unit: refusals.
    ring_full: u64,
    /// Operations that ended in `RingError::Corrupt` — an impossible cursor or
    /// a forged slot number, caught rather than followed. Unit: refusals.
    corrupt: u64,
    /// Submissions drained off the ring. Unit: entries.
    popped: u64,
    /// Completions reaped. Unit: entries.
    reaped: u64,

    /// Entries `execute` ran to a success. Unit: entries.
    executed: u64,
    /// Entries `execute` refused. Unit: entries.
    refused_entry: u64,
    /// Entries refused for a non-zero reserved word. Unit: entries.
    refused_reserved: u64,
    /// Entries refused for a flag bit this build does not know. Unit: entries.
    refused_flag: u64,
    /// Entries refused for an opcode this build does not implement.
    /// Unit: entries.
    refused_opcode: u64,
    /// Entries refused for a range outside the arena. Unit: entries.
    refused_entry_address: u64,
    /// Bytes `write_serial` actually copied out of the arena. Unit: bytes.
    arena_copied: u64,
    /// The most work one armed bound ever saw. Unit: bytes.
    work_high: u64,
}

impl Reach {
    /// Fold one adoption refusal into the counter that names it.
    fn refusal(&mut self, code: i32) {
        match error::unpack(code) {
            Some((error::ARGUMENT, error::argument::MALFORMED_HEADER)) => self.refused_header += 1,
            Some((error::ARGUMENT, error::argument::BAD_ADDRESS)) => self.refused_address += 1,
            Some((error::PEER, error::peer::VERSION_UNSUPPORTED)) => self.refused_version += 1,
            Some((error::PEER, error::peer::FEATURE_REQUIRED)) => self.refused_feature += 1,
            // A refusal in a domain this file has not been taught about is
            // still a refusal, and it is counted as nothing rather than
            // mis-attributed. The minimums in `claims/0008` are on the four
            // above, so a code that stopped being produced goes red there.
            _ => {}
        }
    }

    /// Fold one completion into the counters that name its refusal.
    fn completion(&mut self, cqe: &Cqe) {
        match cqe.error() {
            None => self.executed += 1,
            Some((error::ARGUMENT, code)) => {
                self.refused_entry += 1;
                match code {
                    error::argument::RESERVED_NOT_ZERO => self.refused_reserved += 1,
                    error::argument::UNKNOWN_FLAG => self.refused_flag += 1,
                    error::argument::UNKNOWN_OPCODE => self.refused_opcode += 1,
                    error::argument::BAD_ADDRESS => self.refused_entry_address += 1,
                    _ => {}
                }
            }
            Some(_) => self.refused_entry += 1,
        }
        if cqe.result > 0 {
            self.arena_copied += cqe.result as u64;
        }
    }
}

/// What a run found, if it found anything.
///
/// Two kinds, because the two properties this binary asserts fail differently
/// and a reader told only *it failed* has to go and find out which.
enum Finding {
    /// Something panicked on bytes a peer wrote, which the whole crate promises
    /// cannot happen.
    Panicked {
        /// The episode it happened in.
        episode: u64,
        /// What the panic said, as far as a caught payload carries it.
        what: String,
    },
    /// An operation did more work than the channel's geometry permits, which is
    /// the deterministic form of *stuck*.
    Stuck {
        /// The episode it happened in.
        episode: u64,
        /// The operation that overran.
        what: &'static str,
        /// What it actually did. Unit: as `what` states.
        did: u64,
        /// What the geometry permits. Unit: the same.
        bound: u64,
    },
}

impl Finding {
    /// The episode a finding reproduces from.
    const fn episode(&self) -> u64 {
        match self {
            Self::Panicked { episode, .. } | Self::Stuck { episode, .. } => *episode,
        }
    }

    /// The one line that says what broke.
    fn describe(&self) -> String {
        match self {
            Self::Panicked { what, .. } => format!("panic — {what}"),
            Self::Stuck { what, did, bound, .. } => {
                format!("stuck — {what} did {did} against a bound of {bound}")
            }
        }
    }
}

/// An operation that overran its bound: what it was, what it did, what it was
/// allowed. Turned into a [`Finding::Stuck`] by the episode runner, which is
/// the only place that knows which episode this is.
type Overrun = (&'static str, u64, u64);

// ---------------------------------------------------------------------------
// The peer.
// ---------------------------------------------------------------------------

/// One episode: a region, a stream, and whatever this end currently believes.
struct Peer<'r> {
    /// The bytes. Shared with the hostile half of this same struct, which is
    /// the arrangement being tested.
    region: &'r Region,
    /// Where this build puts each region of a mapping.
    at: &'r Where,
    /// This episode's draws.
    stream: Stream,
    /// The channel this end has believed, if it has believed one. [`Adopted`]
    /// holds the layout it was validated at and rebuilds the mapping per call —
    /// RFC 0037 — so this is the half of the run where the peer's header goes
    /// on lying after the honest end has stopped reading it.
    bound: Option<Adopted>,
    /// The epoch the binding above was taken at. Unit: restarts of the peer.
    believed_epoch: u32,
    /// The epoch the peer is actually at. Unit: restarts of the peer.
    peer_epoch: u32,
}

impl<'r> Peer<'r> {
    /// A peer at the start of an episode: a zeroed region with a sound header,
    /// which is what the frame hands a component.
    fn new(region: &'r Region, at: &'r Where, seed: u64) -> Self {
        region.clear();
        region.place(sound(0));
        Self {
            region,
            at,
            stream: Stream::from_seed(seed),
            bound: None,
            believed_epoch: 0,
            peer_epoch: 0,
        }
    }

    /// The next draw.
    fn draw(&mut self) -> u64 {
        self.stream.next_u64()
    }

    /// A drawn value below `n`.
    #[allow(clippy::cast_possible_truncation)]
    fn below(&mut self, n: u32) -> u32 {
        (self.draw() % u64::from(n.max(1))) as u32
    }

    /// A drawn cursor value, from the family a hostile peer actually writes.
    ///
    /// The families are named rather than left as *an arbitrary `u32`*, because
    /// a uniform draw over four billion values lands outside the ring
    /// essentially always — and what is hard to get right is the value just
    /// inside the bound and the value just outside it, not the one a mile away.
    /// The two arbitrary cases are still there, because a list of families is a
    /// guess about where bugs live and a guess is not a proof.
    #[allow(clippy::cast_possible_truncation)]
    fn hostile_cursor(&mut self, current: u32) -> u32 {
        let value = self.draw();
        match self.below(8) {
            // Backwards, which is what a cursor may never run.
            0 => current.wrapping_sub(1),
            1 => current.wrapping_sub(value as u32 & 0xFF),
            // A jump to and just past the ring's capacity, which is the
            // boundary the occupancy check is written against.
            2 => current.wrapping_add(ENTRIES + 1),
            3 => current.wrapping_add(ENTRIES),
            // Near the `u32` wrap, where the wrapping subtraction is the whole
            // of the arithmetic.
            4 => u32::MAX.wrapping_sub(value as u32 & 0x3),
            // A value that would address the arena if a mask were forgotten.
            5 => self.at.arena + (value as u32 % self.at.arena_len),
            _ => value as u32,
        }
    }

    /// Bump the epoch and rewrite the header: the peer restarted.
    fn restart(&mut self, reset_cursors: bool) {
        self.peer_epoch = self.peer_epoch.wrapping_add(1);
        self.region.place(sound(self.peer_epoch));
        if reset_cursors {
            // A restarting peer usually comes back to a fresh region, so the
            // cursors go to zero while the honest end still holds tokens
            // against the old ones. That is the state `PEER/EPOCH_CHANGED`
            // exists for, seen from the side that causes it.
            for line in CURSOR_LINES {
                self.region.poke32(line, 0);
            }
        }
    }

    /// Adopt the region through the safe path a component uses, and notice a
    /// restart if the epoch moved.
    fn adopt(&mut self, reach: &mut Reach) {
        let offers = if self.draw() & 1 == 0 { 0 } else { feature::CONTROL_EVENTS };
        match Adopted::at(self.region.address(), LEN, offers, 0) {
            Ok(adopted) => {
                reach.adopts_ok += 1;
                if self.bound.is_some() && adopted.epoch() != self.believed_epoch {
                    // The peer restarted under a channel this end had already
                    // bound. Every token outstanding on the old one is stale.
                    reach.epoch_changes += 1;
                }
                self.believed_epoch = adopted.epoch();
                self.bound = Some(adopted);
            }
            Err(code) => {
                reach.refusal(code);
                // Fail closed: a refused adoption does not leave the previous
                // binding usable under a header that has since been scribbled.
                // It is dropped, and the next successful adoption brings the
                // channel back.
                self.bound = None;
            }
        }
    }

    /// Adopt a region the frame described askew — unaligned, or too short for a
    /// header.
    ///
    /// Not something the *peer* writes, and here anyway: it is the one refusal
    /// that cannot come from the header, because it is what makes reading the
    /// header defined at all. Without it `refused_address` would be a counter
    /// no run can move, and `claims/0008` would carry a minimum on a number
    /// that is structurally zero — which is the shape of false green this whole
    /// file is arranged against.
    fn adopt_askew(&mut self, reach: &mut Reach) {
        let (base, len) = if self.draw() & 1 == 0 {
            (self.region.address() + 1, LEN - 1)
        } else {
            (self.region.address(), 63)
        };
        match Adopted::at(base, len, 0, 0) {
            // A refusal is the only correct answer, and an acceptance is not a
            // panic — so it is recorded as an adoption and the minimum on
            // `refused_address` is what goes red.
            Ok(_) => reach.adopts_ok += 1,
            Err(code) => reach.refusal(code),
        }
    }

    /// The believed channel's client end, if there is one.
    fn client(&self) -> Option<Client> {
        self.bound.map(Adopted::client)
    }

    /// The believed channel's server end, if there is one.
    fn server(&self) -> Option<Server> {
        self.bound.map(Adopted::server)
    }

    /// A drawn submission: mostly well-formed, sometimes not, never sanitised.
    #[allow(clippy::cast_possible_truncation)]
    fn entry(&mut self) -> Sqe {
        let mut sqe = Sqe::ZERO;
        sqe.user_data = self.draw();
        let a = self.draw();
        match self.below(6) {
            // A well-formed nop, which is what keeps the ring moving so the
            // hostile cases have something to interfere with.
            0 => {}
            // A well-formed write of a range inside the arena.
            1 => {
                let len = self.below(self.at.arena_len);
                let offset = self.below(self.at.arena_len - len);
                sqe.opcode = 1;
                sqe.offset = u64::from(offset);
                sqe.len = len;
            }
            // A write whose range is not.
            2 => {
                sqe.opcode = 1;
                sqe.offset = self.draw();
                sqe.len = self.draw() as u32;
            }
            // An entry drawn with no structure at all. Its reserved word is
            // drawn too, so it is refused by the *first* envelope check almost
            // always — which is why the two cases below exist.
            3 => {
                sqe.opcode = a as u8;
                sqe.flags = (a >> 8) as u8;
                sqe.class = (a >> 16) as u16;
                sqe.cap = (a >> 32) as u32;
                sqe.deadline = self.draw();
                sqe.offset = self.draw();
                sqe.len = self.draw() as u32;
                sqe._reserved = self.draw() as u32;
            }
            // The envelope is checked in order — reserved word, then flags,
            // then opcode — so a generator that always drew a reserved word
            // would reach the first check and never the other two, and
            // `refused_flag` and `refused_opcode` would sit at zero while the
            // run reported a billion clean operations. They did, in the first
            // version of this file. These two cases are what moved them, and
            // the minimums in `claims/0008` are what stop them going back.
            4 => {
                sqe.opcode = a as u8;
                sqe.flags = (a >> 8) as u8;
                sqe.len = self.draw() as u32;
            }
            _ => {
                sqe.opcode = a as u8;
                sqe.offset = self.draw();
                sqe.len = self.draw() as u32;
            }
        }
        sqe
    }

    /// The peer scribbles one named header field, or a consistent pair of them.
    #[allow(clippy::cast_possible_truncation)]
    fn scribble_header(&mut self) {
        let mut header = sound(self.peer_epoch);
        let value = self.draw();
        match self.below(12) {
            0 => header.magic = value,
            1 => header.magic = CHANNEL_MAGIC ^ (1 << (value % 64)),
            2 => header.ring_size = value as u32,
            3 => header.sqe_offset = value as u32,
            4 => header.cqe_offset = value as u32,
            5 => header.abi_version = value as u32,
            6 => header.abi_version_min = value as u32,
            7 => header.features = value,
            8 => header.features_required = value | 1,
            // A peer from the future: a floor above anything this build speaks,
            // stated consistently so it survives structural validation and is
            // refused by negotiation instead. The pair is drawn together
            // because the two fields are refused by different code when they
            // disagree, and `PEER/VERSION_UNSUPPORTED` is only reachable when
            // they agree.
            9 => {
                header.abi_version = (value as u32).max(3);
                header.abi_version_min = header.abi_version - 1;
            }
            // A peer from before there was a version.
            10 => {
                header.abi_version = 0;
                header.abi_version_min = 0;
            }
            _ => header._reserved[(value % 4) as usize] = value as u32,
        }
        self.region.place(header);
    }

    /// One drawn operation. The whole of what a hostile peer and an honest end
    /// do to each other.
    ///
    /// Returns an [`Overrun`] for the hang property only; a panic is caught by
    /// the caller, which is the only place it can be.
    fn step(&mut self, reach: &mut Reach) -> Option<Overrun> {
        // Weighted by repetition rather than by a table of probabilities: the
        // honest operations appear more than once because a run in which the
        // ring is always wrecked exercises the refusal path and nothing behind
        // it, and the counters in `Reach` are what make that visible.
        match self.below(26) {
            // --- the peer scribbles --------------------------------------
            0 => {
                let at = self.below(64);
                #[allow(clippy::cast_possible_truncation)]
                let byte = self.draw() as u8;
                self.region.poke(at, byte);
                reach.header_bytes += 1;
            }
            1 => {
                self.scribble_header();
                reach.header_fields += 1;
            }
            2 => {
                // A sound header again. Without this the region would spend
                // most of an episode refused and everything behind adoption
                // would be nearly unreachable — the shape of vacuous green the
                // `Reach` minimums exist to make visible.
                self.region.place(sound(self.peer_epoch));
            }
            3 | 4 => {
                let line = CURSOR_LINES[self.below(4) as usize];
                let current = self.region.read32(line);
                let value = self.hostile_cursor(current);
                self.region.poke32(line, value);
                reach.cursors += 1;
            }
            5 => {
                let slot = self.below(ENTRIES);
                #[allow(clippy::cast_possible_truncation)]
                let value = match self.below(3) {
                    0 => self.draw() as u32,
                    1 => ENTRIES + (self.draw() as u32 % 16),
                    _ => self.below(ENTRIES),
                };
                self.region.poke32(self.at.index + slot * 4, value);
                reach.index_slots += 1;
            }
            6 => {
                let slot = self.below(ENTRIES);
                let base = self.at.entries + slot * 64;
                for word in 0..16u32 {
                    #[allow(clippy::cast_possible_truncation)]
                    let value = self.draw() as u32;
                    self.region.poke32(base + word * 4, value);
                }
                reach.entry_slots += 1;
            }
            7 => {
                let at = self.below(self.at.arena_len);
                #[allow(clippy::cast_possible_truncation)]
                let byte = self.draw() as u8;
                self.region.poke(self.at.arena + at, byte);
                reach.arena_bytes += 1;
            }
            8 => {
                #[allow(clippy::cast_possible_truncation)]
                let value = self.draw() as u32;
                self.region.poke32(FLAGS_WORD, value);
                reach.flag_words += 1;
            }
            9 => {
                let reset = self.draw() & 1 == 0;
                self.restart(reset);
                reach.restarts += 1;
            }

            // --- the honest end works ------------------------------------
            10 | 11 => self.adopt(reach),
            12 => self.adopt_askew(reach),
            13..=15 => {
                let entry = self.entry();
                if let Some(client) = self.client() {
                    match client.submit(entry) {
                        Ok(_) => reach.submitted += 1,
                        Err(RingError::Full) => reach.ring_full += 1,
                        Err(RingError::Corrupt | RingError::EpochChanged) => reach.corrupt += 1,
                    }
                }
            }
            16 => {
                if let Some(client) = self.client()
                    && client.queued().is_err()
                {
                    reach.corrupt += 1;
                }
            }
            17 | 18 => {
                if let Some(server) = self.server() {
                    match server.pop() {
                        Ok(Some(_)) => reach.popped += 1,
                        Ok(None) => {}
                        Err(_) => reach.corrupt += 1,
                    }
                }
            }
            19 => {
                let token = self.draw();
                if let Some(server) = self.server() {
                    let _ = server.free();
                    match server.post(f_ring::completion(token, 0, 0)) {
                        Ok(()) | Err(RingError::Full) => {}
                        Err(_) => reach.corrupt += 1,
                    }
                }
            }
            20 | 21 => {
                if let Some(client) = self.client() {
                    match client.take() {
                        Ok(Some(_)) => reach.reaped += 1,
                        Ok(None) => {}
                        Err(_) => reach.corrupt += 1,
                    }
                }
            }
            22 => return self.batch(reach),
            23 | 24 => return self.drain(reach),
            _ => return self.execute_one(reach),
        }
        None
    }

    /// Stage a batch, let the peer restart in the middle of it, and publish.
    ///
    /// This is the exit's *restarts mid-operation*, and it is only expressible
    /// because both ends hold the region by shared reference: the peer writes
    /// while the batch is staged and unpublished, which is exactly the window a
    /// `&mut [u8]` model would have made unreachable.
    fn batch(&mut self, reach: &mut Reach) -> Option<Overrun> {
        // A `Batch` needs a `Producer`, which needs a `Channel`, which borrows
        // a `Mapping` — so this operation reads the header rather than using
        // the believed layout. That is the frame's discipline rather than a
        // component's: the frame adopts at the moment a channel is handed over.
        let base = self.region.base();
        // SAFETY: the region is 4096 aligned, initialised bytes owned by this
        // test for the whole run, every one of them inside an `UnsafeCell`; the
        // only references into the range are the ones this mapping hands out,
        // and it is dropped before this function returns. Its *contents* are
        // hostile, which is the subject and not a safety obligation.
        let Ok(mapping) = (unsafe { Mapping::adopt(base, LEN, 0, 0) }) else { return None };
        let mut producer = Producer::new(mapping.channel())?;

        let count = self.below(4) + 1;
        let interrupt = self.draw() & 3 == 0;
        let mut staged = 0u64;
        let mut entries = [Sqe::ZERO; 4];
        for slot in entries.iter_mut().take(count as usize) {
            *slot = self.entry();
        }

        let mut batch = producer.batch();
        for entry in entries.into_iter().take(count as usize) {
            if batch.push(entry).is_ok() {
                staged += 1;
            }
        }
        if interrupt {
            // The peer stops answering half way through and comes back as a
            // different instance, with the batch still invisible to anybody.
            self.peer_epoch = self.peer_epoch.wrapping_add(1);
            self.region.place(sound(self.peer_epoch));
            reach.restarts += 1;
            reach.restarts_mid_batch += 1;
        }
        if batch.publish().is_err() {
            reach.corrupt += 1;
        }
        reach.submitted += staged;
        None
    }

    /// Drain the ring the way a service does, and check the budget held.
    fn drain(&mut self, reach: &mut Reach) -> Option<Overrun> {
        let now = self.draw();
        let base = self.region.base();
        // SAFETY: as `batch`.
        let Ok(mapping) = (unsafe { Mapping::adopt(base, LEN, 0, 0) }) else { return None };
        let (Some(consumer), Some(poster)) =
            (Consumer::new(mapping.channel()), Poster::new(mapping.completions()))
        else {
            return None;
        };

        // The bound: a drain of `BUDGET` entries may offer the sink at most one
        // arena's worth of bytes per entry, because `write_serial` refuses any
        // range not wholly inside the arena.
        let bound = u64::from(BUDGET) * u64::from(self.at.arena_len);
        let mut service = Service::new(consumer, poster, mapping.arena(), Bounded::armed(bound));
        let done = service.drain(BUDGET, now);
        let sink = service.sink();
        let (over, taken) = (sink.over, sink.taken);
        reach.work_high = reach.work_high.max(taken);

        if over {
            return Some(("a drain's bytes to the sink", taken, bound));
        }
        match done {
            Ok(Drained { executed, completed, refused }) => {
                if executed > BUDGET {
                    return Some((
                        "a drain's executed entries",
                        u64::from(executed),
                        BUDGET.into(),
                    ));
                }
                reach.popped += u64::from(executed);
                reach.executed += u64::from(completed.saturating_sub(refused));
                reach.refused_entry += u64::from(refused);
                reach.arena_copied += taken;
            }
            Err(_) => reach.corrupt += 1,
        }
        None
    }

    /// Pop one entry and run it through `execute`, which is where the bound is
    /// tightest: one entry may offer the sink one arena's worth of bytes and no
    /// more.
    fn execute_one(&mut self, reach: &mut Reach) -> Option<Overrun> {
        let now = self.draw();
        let base = self.region.base();
        // SAFETY: as `batch`.
        let Ok(mapping) = (unsafe { Mapping::adopt(base, LEN, 0, 0) }) else { return None };
        let consumer = Consumer::new(mapping.channel())?;
        let entry = match consumer.pop() {
            Ok(Some(entry)) => entry,
            Ok(None) => return None,
            Err(_) => {
                reach.corrupt += 1;
                return None;
            }
        };
        reach.popped += 1;

        let bound = u64::from(self.at.arena_len);
        let mut sink = Bounded::armed(bound);
        let arena = mapping.arena();
        if let Some(cqe) = execute(&entry, &arena, &mut sink, now) {
            reach.completion(&cqe);
        }
        reach.work_high = reach.work_high.max(sink.taken);
        if sink.over {
            return Some(("one entry's bytes to the sink", sink.taken, bound));
        }
        if sink.calls > bound {
            return Some(("one entry's calls to the sink", sink.calls, bound));
        }
        None
    }
}

// ---------------------------------------------------------------------------
// The run.
// ---------------------------------------------------------------------------

/// The identity an episode's stream is derived at.
///
/// The ASCII of `hep`, for *hostile episode*. It is here so that adding another
/// derived stream later — a second generator, a per-episode geometry — does not
/// move the episodes a recorded finding names. RFC 0026.
const EPISODE_IDENTITY: u64 = 0x0068_6570;

/// Run one episode, catching a panic so the failure names a seed rather than a
/// backtrace.
///
/// `catch_unwind` works here because cargo ignores a profile's `panic` setting
/// for test targets, and this is one. In a build where it did not, a panic
/// would abort the process — still a red run, and one that says less.
fn one_episode(
    region: &Region,
    at: &Where,
    seed: u64,
    episode: u64,
    reach: &mut Reach,
) -> Option<Finding> {
    let stream_seed = derive(seed, EPISODE_IDENTITY ^ episode.rotate_left(21));
    let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| {
        let mut peer = Peer::new(region, at, stream_seed);
        for _ in 0..STEPS {
            if let Some((what, did, bound)) = peer.step(reach) {
                return Some(Finding::Stuck { episode, what, did, bound });
            }
        }
        None
    }));

    match outcome {
        Ok(found) => found,
        Err(payload) => {
            let what = payload
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| payload.downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "a panic carrying no message".to_string());
            Some(Finding::Panicked { episode, what })
        }
    }
}

/// The name this target answers to when cargo's harness protocol asks for one.
///
/// `harness = false` means the binary *is* the test, so there is exactly one and
/// this is what it is called. It matters because a filter — `cargo test -p
/// f-ring litmus`, or anything nextest does — is handed to **every** test target
/// in the package, including the ones the filter is not about.
const TEST_NAME: &str = "hostile";

/// What one invocation was asked for.
///
/// The episode arithmetic is done in [`parse`] and carried here already checked,
/// rather than recomputed in [`run`]. A fuzzer whose whole subject is arithmetic
/// on numbers somebody else supplied is the last place to leave its own argument
/// arithmetic able to overflow: `--ops` within 1024 of `u64::MAX` rounds up past
/// the end of the range, and in the debug build `cargo test --workspace` uses,
/// that is a panic in the harness reported as a finding about the ring.
struct Asked {
    /// The base the episodes derive from.
    seed: u64,
    /// The first episode this run performs.
    first: u64,
    /// How many episodes it performs. Unit: episodes.
    episodes: u64,
    /// Answer the harness protocol's `--list` and run nothing.
    list: bool,
    /// A name filter that selected nothing. Carried rather than refused, so
    /// [`main`] can answer it the way a harness does.
    unselected: Option<String>,
}

/// Parse the command line.
///
/// # Two kinds of argument, and why only one of them fails closed
///
/// The fuzzer's own options — `--seed`, `--ops`, `--episode` — fail closed, R04:
/// a misspelling that quietly ran a *different* run than the one somebody asked
/// for is what a gate binary cannot afford.
///
/// Cargo's harness protocol is not that. `cargo test` hands every test target in
/// a package the same arguments, including a bare **name filter** meant for some
/// other target, and `--list`, which is how cargo and nextest enumerate tests.
/// Refusing those made `cargo test -q --release -p f-ring litmus` fail the whole
/// package *after* the six litmus tests had passed, and would break any CI step
/// or agent that narrowed a run. So the protocol's arguments are answered the
/// way libtest answers them: a filter that does not match [`TEST_NAME`] selects
/// nothing and exits zero, `--list` names the one test, and the flags libtest
/// owns are accepted and ignored. R04 is about fields a peer writes; the
/// harness's own argv is not one of them.
fn parse(args: &[String]) -> Result<Asked, String> {
    let mut seed = DEFAULT_SEED;
    let mut ops = DEFAULT_OPS;
    let mut episode: Option<u64> = None;
    let mut list = false;
    let mut filter: Option<String> = None;

    let mut walk = args.iter();
    while let Some(arg) = walk.next() {
        let mut value = || walk.next().cloned().ok_or_else(|| format!("{arg} needs a value"));
        match arg.as_str() {
            "--seed" => seed = number(&value()?)?,
            "--ops" => ops = number(&value()?)?,
            "--episode" => episode = Some(number(&value()?)?),
            "--list" => list = true,
            // Flags libtest owns that take no value.
            "--nocapture"
            | "--quiet"
            | "-q"
            | "--exact"
            | "--show-output"
            | "--ignored"
            | "--include-ignored"
            | "--force-run-in-process"
            | "--report-time"
            | "--test"
            | "--bench" => {}
            // The same, taking one.
            "--test-threads" | "--color" | "--format" | "--logfile" | "--skip" | "-Z" => {
                let _ = value()?;
            }
            other if other.starts_with('-') => return Err(format!("unknown argument: {other}")),
            // A bare word is a name filter. The last one wins, which is
            // libtest's behaviour and not a decision worth making differently.
            other => filter = Some(other.to_string()),
        }
    }

    if let Some(name) = filter.filter(|name| !TEST_NAME.contains(name.as_str())) {
        return Ok(Asked { seed, first: 0, episodes: 0, list: false, unselected: Some(name) });
    }

    if episode.is_none() && ops == 0 {
        return Err("--ops 0 asks for a run with no operations in it, which is a result \
                    that is green because it asserted nothing. R04."
            .to_string());
    }

    let steps = u64::from(STEPS);
    let first = episode.unwrap_or(0);
    let episodes = if episode.is_some() { 1 } else { ops.div_ceil(steps) };
    if episodes.checked_mul(steps).is_none() {
        return Err(format!(
            "--ops {ops} rounds up to {episodes} episode(s) of {STEPS}, which is more \
             operations than a u64 counts. The largest --ops this binary accepts is {}.",
            (u64::MAX / steps) * steps
        ));
    }
    if first.checked_add(episodes).is_none() {
        return Err(format!(
            "--episode {first} plus {episodes} episode(s) runs past the last episode a u64 \
             numbers. The largest --episode this binary accepts is {}.",
            u64::MAX - episodes
        ));
    }
    Ok(Asked { seed, first, episodes, list, unselected: None })
}

/// A decimal or `0x`-prefixed hexadecimal number.
fn number(text: &str) -> Result<u64, String> {
    let parsed = text
        .strip_prefix("0x")
        .map_or_else(|| text.parse::<u64>().ok(), |hex| u64::from_str_radix(hex, 16).ok());
    parsed.ok_or_else(|| format!("`{text}` is not a number"))
}

/// One run, and the status it earns.
///
/// Split from [`main`] so the region is dropped before the process exits.
/// `std::process::exit` runs no destructor, and a run that left its page
/// allocated would be reported as a leak by the one tool this file exists to be
/// run under — on the *failing* runs only, which is the worst place to add a
/// second message.
fn run(asked: &Asked) -> i32 {
    let at = Where::here();
    let region = Region::new();
    let mut reach = Reach::default();
    let mut found: Option<Finding> = None;

    // Neither of these can overflow: `parse` refused the arguments that would
    // make them, which is where an argument is judged rather than here.
    let (first, episodes) = (asked.first, asked.episodes);
    let ops = episodes * u64::from(STEPS);

    println!(
        "hostile — {ops} operation(s) in {episodes} episode(s) of {STEPS}, \
         from seed {:#018x}",
        asked.seed
    );

    for episode in first..first + episodes {
        found = one_episode(&region, &at, asked.seed, episode, &mut reach);
        if found.is_some() {
            // One finding is enough to act on and the run stops there: a fuzzer
            // that carries on past a panic is reporting behaviour measured
            // after the thing it was measuring broke.
            break;
        }
    }

    report(&reach, ops);

    let Some(found) = found else {
        println!("\nfindings   none");
        return 0;
    };
    println!("\nfinding 1  {}", found.describe());
    println!(
        "  repro      cargo test -q --release -p f-ring --test hostile -- \
         --seed {:#018x} --episode {}",
        asked.seed,
        found.episode()
    );
    1
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let asked = match parse(&args) {
        Ok(asked) => asked,
        Err(why) => {
            eprintln!(
                "hostile: {why}\n\n\
                 usage: --seed <n> [--ops <n> | --episode <n>]\n\
                 \x20      cargo's own harness arguments — a name filter, --list,\n\
                 \x20      --nocapture and the rest of libtest's flags — are answered\n\
                 \x20      rather than refused. RFC 0046."
            );
            std::process::exit(2);
        }
    };
    // The harness protocol's two answers, before any work is done: cargo asks a
    // test target what it holds, and hands it filters aimed at other targets.
    if asked.list {
        println!("{TEST_NAME}: test");
        println!();
        println!("1 test, 0 benchmarks");
        return;
    }
    if let Some(name) = &asked.unselected {
        println!(
            "hostile — 0 operation(s): the filter `{name}` selects no test in this target,\n\
             \x20         so this run performed none. That is a harness answering a filter\n\
             \x20         and it asserts nothing about the ring."
        );
        return;
    }
    std::process::exit(run(&asked));
}

/// Print what the run reached.
///
/// Every line is a count and none of them is a duration: this binary reads no
/// clock, and the wall-clock cost of a run belongs beside the report rather
/// than inside it. `cargo xtask hostile` is what prints that.
fn report(reach: &Reach, ops: u64) {
    let line = |name: &str, value: u64| println!("  {name:<24}{value:>14}");

    println!("\nthe peer");
    line("header bytes", reach.header_bytes);
    line("header fields", reach.header_fields);
    line("cursors", reach.cursors);
    line("index slots", reach.index_slots);
    line("entry slots", reach.entry_slots);
    line("arena bytes", reach.arena_bytes);
    line("flag words", reach.flag_words);
    line("restarts", reach.restarts);
    line("restarts mid-batch", reach.restarts_mid_batch);

    println!("\nthe channel");
    line("adopted", reach.adopts_ok);
    line("refused malformed", reach.refused_header);
    line("refused address", reach.refused_address);
    line("refused version", reach.refused_version);
    line("refused feature", reach.refused_feature);
    line("epoch changes seen", reach.epoch_changes);

    println!("\nthe rings");
    line("submitted", reach.submitted);
    line("ring full", reach.ring_full);
    line("corrupt, reported", reach.corrupt);
    line("popped", reach.popped);
    line("reaped", reach.reaped);

    println!("\nthe entries");
    line("executed", reach.executed);
    line("refused", reach.refused_entry);
    line("refused reserved", reach.refused_reserved);
    line("refused flag", reach.refused_flag);
    line("refused opcode", reach.refused_opcode);
    line("refused bad address", reach.refused_entry_address);
    line("arena bytes copied", reach.arena_copied);
    line("most work in one bound", reach.work_high);

    println!("\noperations {ops}");
}
