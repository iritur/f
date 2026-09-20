// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The objects ring entry format: five opcodes, one fixed-width record, and
//! the field an application byte is counted at.
//!
//! # What this is, and what it deliberately is not
//!
//! `user/objects` is the component `E2-B08` builds, and this module is the
//! wire it answers on — the wire and nothing else. **One opcode here is
//! implemented and four are not.** [`op::known`] admits [`op::READ`], which
//! `f_objects::service` answers; the other four are numbers with nothing
//! behind them and are refused with [`error::argument::UNKNOWN_OPCODE`]
//! exactly as any opcode a service does not implement is. Widening
//! [`op::known`] stays a line in this file with a red test beside it rather
//! than a silence, and it stayed one for this diff too.
//!
//! The numbers land before the behaviour because that is the cheapest order.
//! `TODO.md`'s ordering rule 1: a number is free to change while one peer
//! exists and expensive once two do, and this space has *no* peers today. The
//! worked example is one slice back — `user/virtio-blk/src/driver.rs`'s five
//! zone opcodes, which landed carrying no behaviour, with `known` deliberately
//! not widened and a test asserting both the refusal and the non-widening.
//! `docs/rfc/0059-a-generation-is-a-root.md` pre-committed three of the five
//! here: the pin opcodes' *numbers are assigned contiguously when the objects
//! service's `op` module is first written*, so that the space is designed once
//! rather than grown. This is that module.
//!
//! # Why the vocabulary is in the wire crate
//!
//! `lib.rs` says most opcode spaces are the property of whichever component
//! defines them. Five per-service vocabularies live here anyway —
//! [`control`](crate::control), [`input`](crate::input),
//! [`participate`](crate::participate), [`scene`](crate::scene) and
//! [`semantic`](crate::semantic) — each under section 05's *per-service and
//! not global*, which is a rule about numbers not colliding across spaces and
//! not a rule about which file holds them. A sixth follows five precedents.
//!
//! It could not live in `user/objects` in any case, and that crate's own
//! `Cargo.toml` states why: the only use for `#[repr(C)]` over a record is a
//! pointer cast, RFC 0001 puts a pointer cast inside the frame, and that crate
//! is not in the frame. [`Request::decode`]'s whole-envelope comparison is
//! exactly such a cast. The counter-precedent — the block opcodes living in
//! `user/virtio-blk` — states its own reason, which is that `kernel/src/blk.rs`
//! imports that module because the frame is a peer of the blk ring. The frame
//! is not a peer of the objects ring, so that reason does not transfer.
//!
//! # Where an application byte is counted
//!
//! `docs/rfc/0058-an-extent-is-pieces.md` defines claim 0017's denominator as
//! *the sum of the `bytes` fields of the write entries the client submitted on
//! the objects ring, and nothing else*. That sentence names a field, and this
//! module is where the field acquires a location: **it is [`Write::bytes`], in
//! the payload, and it is not [`Sqe::len`]**.
//!
//! The reason is that [`Sqe::len`] is already spoken for. Every entry on this
//! ring frames its payload the way [`scene`](crate::scene) and
//! [`input`](crate::input) frame theirs — `len` is [`PAYLOAD_BYTES`], the one
//! stride, on every opcode — so `len` is a constant of the format and carries
//! no information about how much of a client's data crossed. The data itself
//! never crosses in the entry at all: it is in the registered buffer
//! [`Sqe::buf_set`] and [`Sqe::buf_index`] name, whose length is the
//! registration's and not this request's. Counting at `len` would count forty
//! per write however large the write was; counting at the buffer's own length
//! would count the buffer rather than the transfer.
//!
//! Stated as a decision, because a counter and a client that each picked one
//! would produce a ratio no test could see was wrong: **a `WRITE` entry's
//! `bytes` field is the application bytes that entry submits, whatever the
//! device does afterwards and whether or not the piece was already dirty.**
//! *What would reverse this:* an objects request that stops framing its
//! payload in the arena — a record small enough to sit in [`Sqe::ext`], which
//! would free `len` for a length that means something — at which point the
//! question is open again and must be decided in this one place rather than at
//! whichever end notices first.
//!
//! # One width for all five, and an unread byte is refused
//!
//! Every payload is [`PAYLOAD_BYTES`] long whatever the opcode, so a batch in
//! the inline arena is an array with a stride rather than a list that must be
//! parsed to be walked — [`scene`](crate::scene)'s argument, and it is the
//! same argument. The cost is the padding: [`Collect`] carries nothing at all
//! and still occupies a slot.
//!
//! Neither half of an entry is checked against a list of fields written out by
//! hand. The payload is read through a `Reader` that counts what the decoder
//! consumed and refuses every byte past it; the envelope is rebuilt from the
//! fields actually read and compared in full, so any field of [`Sqe`] this
//! format does not set — including one a later ABI adds — must arrive zero.
//! R04, structurally.
//!
//! # No clock, no floating point, no allocator
//!
//! Nothing here reads a clock: a deadline on one of these entries is a number
//! the caller computed, carried verbatim. Nothing here is binary floating
//! point, nothing here allocates, and every field is encoded by hand through
//! `to_le_bytes`, so nothing inherits the host's word size or byte order.
//! RFC 0004.
//!
//! # A sixth opcode
//!
//! It is an RFC, by the rule that makes every change to this crate an ABI
//! change, and it is also a diff to one list: the `entries!` invocation below
//! emits the opcode constants, the [`Entry`] variants, the dispatch, the
//! specimens every test runs over, and the answer to *does this one move a
//! client's bytes*. There is no second place a sixth opcode could be written.

use crate::{Sqe, error, flags};

/// The routing page the frame and the objects component share.
///
/// A wire like the entry format above and in the same module for that reason,
/// not because a board and an opcode space are the same kind of thing: what
/// they share is that two peers in this tree have to agree on them byte for
/// byte, which is what this crate is for.
#[path = "objects/board.rs"]
pub mod board;

/// Bytes of inline arena one objects entry's payload occupies, whatever its
/// opcode.
///
/// The widest record is [`Read`] at 36 bytes, and this is the next multiple of
/// eight above it: every multi-byte field in every record below is at most
/// eight wide, so a stride that is a multiple of eight gives the same
/// alignment to the same field in every slot of a batch. Nothing here casts a
/// payload to a struct — every record is encoded by hand, field by field — so
/// the alignment buys a reader's arithmetic rather than a compiler's, which is
/// the only kind this crate is allowed to want.
///
/// Written down rather than derived from the records, for
/// [`input`](crate::input)'s reason: this is the wire's stride, and a stride
/// that recomputed itself when a record grew would change what every peer must
/// agree on without anybody typing the new number. The assertion at the foot
/// of this file is what makes a record growing past it — or shrinking and
/// leaving it eight bytes wider than anything uses — a build failure rather
/// than a comment that has gone stale.
/// Unit: bytes.
pub const PAYLOAD_BYTES: usize = 40;

/// The width of a submission entry, which [`Request::decode`] compares in full.
///
/// Stated here as well as in `lib.rs` because the comparison depends on it and
/// on there being no padding inside an [`Sqe`]; the two assertions further
/// down this file are what make both facts rather than hopes.
/// Unit: bytes.
const SQE_BYTES: usize = 64;

/// The submission flags an objects request may carry.
///
/// [`flags::FIXED_BUF`] and nothing else, and every part of that is a decision
/// rather than an oversight:
///
/// - **`FIXED_BUF` is the only payload path.** `TODO.md`'s `E2-B08` line
///   carries the parenthesis *the caller's buffer is a registered one, or the
///   zero-copy count is a copy nobody counted*, and `user/objects/src/lib.rs`
///   is built on it: a destination resolved through `f_ring::registry::Table`
///   is the only one a service can say was registered. An entry that named
///   memory any other way would be an entry whose copy count is taken against
///   the wrong question.
/// - **[`flags::NO_CQE`] is refused.** Every opcode here has an answer the
///   client needs — a count for [`Read`] and [`Write`], a refusal for a
///   [`Pin`] whose root does not resolve, and for [`Collect`] the completion
///   *is* the result, since RFC 0059 says a collect completes when the
///   collector has nothing left to do. Suppressing the completion suppresses
///   the refusal, and R04 is about refusals being visible.
/// - **[`flags::LINK`] and [`flags::DRAIN`] are refused**, for now and
///   loudly. RFC 0060's publish is an ordered sequence — write the blobs,
///   flush, append the root record, flush — and ordering on *this* ring is a
///   design nobody has written down. Accepting a bit whose meaning this format
///   does not define is the unread-field defect wearing an envelope; refusing
///   it is one line to reverse. *What would reverse this:* RFC 0060's barrier
///   expressed as linked entries rather than as a service-side sequence.
///
/// Unit: none — a bitmask of the [`flags`] constants.
pub const FLAGS_ACCEPTED: u8 = flags::FIXED_BUF;

/// Why an entry was not believed.
///
/// A value rather than a packed integer, for
/// [`scene::Refusal`](crate::scene::Refusal)'s reason: a caller compares
/// against these, and a test that spelled the packed integer out itself would
/// pass on a decoder that started returning the right refusal for the wrong
/// reason. [`Refusal::packed`] is the one place the mapping to RFC 0010's
/// domains is written, and no code in it is new — this module adds nothing to
/// [`error::argument`], because four sibling entry formats are written against
/// each other and a ninth `argument` code invented in each of them would be
/// four meanings wearing one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// An opcode outside this service's five. Refused, never skipped: R04.
    ///
    /// This is also what four of the five get from a service today, and the
    /// difference is worth keeping straight: this refusal is the decoder
    /// saying the number is not in the vocabulary at all, and [`op::known`] is
    /// the build saying it does not answer a number that is. They are one
    /// packed code on the wire and two different things to fix, which is why
    /// `f_objects::service` raises the second itself rather than asking this
    /// decoder for it.
    UnknownOpcode,
    /// A submission flag outside [`FLAGS_ACCEPTED`]. Refused rather than
    /// masked off — a bit silently dropped is two peers with different beliefs
    /// about what just happened.
    UnknownFlag,
    /// A field this opcode does not read is not zero: a byte past the record's
    /// own fields, a buffer named by an opcode that moves no bytes, or any
    /// part of the envelope this format never looks at. The refusal this
    /// module is shaped around.
    Reserved,
    /// The entry does not frame a payload: a length that is not
    /// [`PAYLOAD_BYTES`], or an arena offset that is not an arena offset.
    Malformed,
    /// An opcode that moves a client's bytes names no registered buffer:
    /// [`flags::FIXED_BUF`] clear, or a [`Sqe::buf_set`] of zero, which is
    /// slot zero at generation zero and was never issued.
    ///
    /// Separate from [`Refusal::Reserved`] because an otherwise-zeroed entry
    /// carrying a real opcode produces exactly this, and a caller chasing a
    /// producer that forgot to register wants to be told that rather than
    /// *some field is wrong*.
    NoBuffer,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownOpcode => "the opcode is not one of this service's five",
            Self::UnknownFlag => "the entry carries a submission flag an objects request may not",
            Self::Reserved => "a field this opcode does not read is not zero",
            Self::Malformed => "the entry does not frame a payload of the one width there is",
            Self::NoBuffer => "an opcode that moves bytes names no registered buffer",
        }
    }

    /// The refusal as a packed [`error`], for a completion.
    ///
    /// Every one is [`error::ARGUMENT`]: the channel is healthy and the peer
    /// is present, and what is wrong is one entry. [`Refusal::NoBuffer`]
    /// shares [`error::argument::UNKNOWN_FLAG`] with [`Refusal::UnknownFlag`],
    /// which is the code [`scene::Refusal`](crate::scene::Refusal) already
    /// uses for *a field that must name something names nothing*. The two are
    /// distinguishable as values, which is where a caller distinguishes them;
    /// they are one code on the wire, which is where RFC 0010 says the domain
    /// is the stable part.
    #[must_use]
    pub const fn packed(self) -> i32 {
        let code = match self {
            Self::UnknownOpcode => error::argument::UNKNOWN_OPCODE,
            Self::Reserved => error::argument::RESERVED_NOT_ZERO,
            Self::Malformed => error::argument::MALFORMED_HEADER,
            Self::UnknownFlag | Self::NoBuffer => error::argument::UNKNOWN_FLAG,
        };
        error::pack(error::ARGUMENT, code)
    }
}

/// Writes a record's fields into a payload, counting as it goes.
///
/// Little-endian and by hand, [`store`](crate::store)'s discipline: the layout
/// is the encoding rather than the compiler's opinion of a struct, so nothing
/// here depends on the host's word size, its alignment rules or its byte
/// order. Every slot the writer does not reach stays zero, which is what makes
/// a record's tail the same bytes on both sides of the wire.
struct Writer<'a> {
    /// The payload being filled.
    out: &'a mut [u8; PAYLOAD_BYTES],
    /// How much of it has been written.
    at: usize,
}

impl Writer<'_> {
    /// Append four bytes.
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes.
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    /// Append a hash, in the order FIPS 180-4 produces it.
    fn hash(&mut self, value: &[u8; 32]) {
        self.put(value);
    }

    /// The one place bytes land.
    ///
    /// Indexing rather than a checked write, and the bound is a compile-time
    /// fact rather than a runtime hope: `entries!` emits
    /// `assert!(WIDTH <= PAYLOAD_BYTES)` for every record it declares, so the
    /// bound is not a list of five that a sixth joins by somebody remembering.
    /// A record wide enough to overflow this cannot reach a build.
    fn put(&mut self, bytes: &[u8]) {
        self.out[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }
}

/// Reads a record's fields out of a payload, counting as it goes.
///
/// The counting is the point. [`Reader::finish`] refuses every byte the
/// record's decoder did not consume, so *unread* is a property of what the
/// decoder actually did rather than of a width somebody wrote down beside it.
/// A field dropped from a decoder does not become a field that is ignored; it
/// becomes a field in the tail, and the tail is refused.
struct Reader<'a> {
    /// The payload being read.
    raw: &'a [u8; PAYLOAD_BYTES],
    /// How much of it has been consumed.
    at: usize,
}

impl Reader<'_> {
    /// Take four bytes.
    fn u32(&mut self) -> u32 {
        let at = self.take(4);
        u32::from_le_bytes([self.raw[at], self.raw[at + 1], self.raw[at + 2], self.raw[at + 3]])
    }

    /// Take eight bytes.
    fn u64(&mut self) -> u64 {
        let at = self.take(8);
        let mut word = [0u8; 8];
        let mut i = 0;
        while i < 8 {
            word[i] = self.raw[at + i];
            i += 1;
        }
        u64::from_le_bytes(word)
    }

    /// Take a hash.
    fn hash(&mut self) -> [u8; 32] {
        let at = self.take(32);
        let mut value = [0u8; 32];
        let mut i = 0;
        while i < 32 {
            value[i] = self.raw[at + i];
            i += 1;
        }
        value
    }

    /// Advance, and answer where the caller's bytes start. Bounded by the same
    /// compile-time assertion [`Writer::put`] is.
    fn take(&mut self, bytes: usize) -> usize {
        let at = self.at;
        self.at += bytes;
        at
    }

    /// Every byte this record did not read must be zero.
    ///
    /// The whole of *an entry with a non-zero unread field is refused rather
    /// than ignored*, for the payload half, written once so that no record can
    /// be written without it.
    fn finish(self) -> Result<(), Refusal> {
        let mut at = self.at;
        while at < PAYLOAD_BYTES {
            if self.raw[at] != 0 {
                return Err(Refusal::Reserved);
            }
            at += 1;
        }
        Ok(())
    }
}

/// What every opcode's record can do.
///
/// Private, because nothing outside this module should encode a payload
/// without the envelope that frames it — [`Request`] is the door. Its value is
/// what it makes obligatory: a record cannot exist without a width, a
/// specimen, a writer and a reader, so a sixth opcode cannot be added without
/// all four, and [`Entry::SPECIMENS`] picks the specimen up without anybody
/// remembering to add it to a test.
trait Record: Copy + Sized {
    /// The bytes this record's fields occupy, before the padding to
    /// [`PAYLOAD_BYTES`].
    /// Unit: bytes.
    const WIDTH: usize;

    /// One value of this record that is legal on the wire.
    ///
    /// Not a default and not an empty value: every field holds something a
    /// broken decoder would not invent — zero is the value every broken
    /// decoder invents — so that the round trip is an observation rather than
    /// a coincidence. A field added to a record without a value here does not
    /// compile.
    const SPECIMEN: Self;

    /// Write the fields, in order.
    fn write(&self, out: &mut Writer);

    /// Read the fields, in the same order, refusing before any of them is
    /// handed back.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming the disbelief. The tail is not this method's
    /// business: [`Record::from_payload`] holds that rule for every record.
    fn read(raw: &mut Reader) -> Result<Self, Refusal>;

    /// Encode into a whole payload slot. Everything past the fields is zero.
    fn to_payload(&self) -> [u8; PAYLOAD_BYTES] {
        let mut out = [0u8; PAYLOAD_BYTES];
        let mut writer = Writer { out: &mut out, at: 0 };
        self.write(&mut writer);
        out
    }

    /// Decode a whole payload slot, refusing whatever this record did not read.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] from the record's own fields, or [`Refusal::Reserved`]
    /// for a byte past them — including the bytes of a field the record
    /// declares a width for and its decoder does not consume.
    fn from_payload(raw: &[u8; PAYLOAD_BYTES]) -> Result<Self, Refusal> {
        let mut reader = Reader { raw, at: 0 };
        let value = Self::read(&mut reader)?;
        // The decoder stopped where the record says its fields end, or the two
        // disagree and this entry is not believed. Without this line a decoder
        // that dropped its record's *last* field would be invisible: the bytes
        // it left behind are inside the declared width, so `finish` never sees
        // them and `WIDTH` is only ever checked against the writer. A decoder
        // that read *past* its width is caught by the same comparison and is
        // the more serious half: it would be reading a later field's bytes.
        if reader.at != Self::WIDTH {
            return Err(Refusal::Reserved);
        }
        reader.finish()?;
        Ok(value)
    }
}

/// Names an opcode constant where a pattern is expected.
///
/// `macro_rules!` will not take `op::$constant` as a pattern directly — a path
/// followed by a metavariable is ambiguous there — and this is the indirection
/// that resolves it. It exists for no other reason and is declared before
/// `entries!` because a macro must be defined before it is used.
macro_rules! opcode_pattern {
    ($constant:ident) => {
        op::$constant
    };
}

/// The five opcodes, written once.
///
/// # Why a macro, in a tree that mostly refuses them
///
/// Because the alternative is four sequences that have to agree: the opcode
/// constants, the list [`op::ALL`] holds, the [`Entry`] variants, and the
/// specimens the round-trip test iterates. `abi/src/scene.rs` and
/// `abi/src/input.rs` reached this shape after two adversarial reviews found a
/// role that was in an enum and not in an array, and `docs/postmortem/0001` is
/// about four hand-written copies of one list. A loop over a list cannot see
/// what the list omits; an exhaustive match demands an arm.
///
/// The cost is the indirection between a reader and the enum, and a third copy
/// of a macro two other files already carry. Factoring one shared macro out of
/// all three is a diff to two landed wire formats and is not this one's to
/// take: this file has no peer yet and those two have.
macro_rules! entries {
    (
        $(
            $(#[$about:meta])*
            $variant:ident / $record:ident / $constant:ident = $opcode:literal,
            moves_bytes: $moves:literal;
        )*
    ) => {
        /// The objects service's opcode space.
        ///
        /// Per-service and not global: section 05. These numbers mean nothing
        /// on the frame's ring, on the block driver's, or on the compositor's,
        /// and comparing an opcode across two spaces is the mistake a single
        /// global enumeration would invite.
        ///
        /// They start at one, so that a zeroed entry — which is what an
        /// untouched slot of a fresh mapping holds — names no opcode and is
        /// refused rather than read as the first one. RFC 0028 reserves `0xFE`
        /// and `0xFF` at the top of every service's space for buffer
        /// registration, and nothing here approaches them.
        pub mod op {
            $(
                $(#[$about])*
                ///
                /// Unit: none — an opcode is an identifier, not a quantity.
                pub const $constant: u8 = $opcode;
            )*

            /// How many opcodes this service has.
            ///
            /// Counted from the list rather than written down, so it cannot
            /// disagree with what it counts.
            /// Unit: opcodes.
            pub const COUNT: usize = [$(stringify!($constant)),*].len();

            /// Every opcode, in declaration order.
            ///
            /// Emitted from the same list as the constants, so it holds every
            /// opcode there is — not because a test checks it, but because
            /// there is no way to write one this array does not get.
            /// Unit: none — opcodes.
            pub const ALL: [u8; COUNT] = [$($constant),*];

            /// Is this a number this vocabulary has assigned?
            ///
            /// The question a *decoder* asks, and the one [`known`] is not. An
            /// entry whose opcode this answers `false` for is refused
            /// [`super::Refusal::UnknownOpcode`] before any field of it is
            /// read.
            #[must_use]
            pub const fn defined(opcode: u8) -> bool {
                matches!(opcode, $($constant)|*)
            }

            /// Does this build answer `opcode`?
            ///
            /// **[`READ`] and nothing else.** `f_objects::service` is what
            /// answers it: an entry naming a registered buffer, a hash
            /// resolved to a zone and an offset, and the content landing in
            /// the caller's own memory. The other four are assigned numbers
            /// that nothing implements, so a client submitting one is answered
            /// `ARGUMENT`/`UNKNOWN_OPCODE` exactly as it would be for an
            /// opcode no service ever heard of.
            ///
            /// Two predicates rather than one, because the two questions
            /// genuinely differ and this is the diff that made the difference
            /// visible: [`defined`] says all five numbers are in the
            /// vocabulary, this says one of them has a body behind it.
            ///
            /// **Deliberately not emitted from the list**, which is the one
            /// place this module departs from `scene`'s shape and the reason
            /// is the whole point: an opcode must not become implemented by
            /// being declared. This line is the widening `E2-B08` owed,
            /// written by hand, and `known_admits_read_and_nothing_else` went
            /// red in the same diff — which is what makes it a decision
            /// somebody took rather than a silence. The next three are three
            /// more decisions, each with a service behind it, and none of them
            /// is this line growing an arm on its own.
            ///
            /// **`WRITE` is the second, taken for `E2-B09`.** Its service is
            /// `f_objects::write::WritePath`, and the reason it had to be a
            /// decision rather than a declaration is the one `claims/0017`
            /// rests on: that claim's denominator is *a byte the client
            /// submitted on the objects ring*, and until an opcode carried one
            /// across, the denominator was a write through a `Store` in a host
            /// harness wearing a client's name.
            ///
            /// It also cannot outlive its own opcode: `READ` here is the
            /// constant the list above emits, so deleting that line from the
            /// list is a build failure here rather than a predicate quietly
            /// answering for a number nobody declares any more.
            #[must_use]
            pub const fn known(opcode: u8) -> bool {
                matches!(opcode, READ | WRITE)
            }

            /// A word for a log or a trace.
            #[must_use]
            pub const fn label(opcode: u8) -> &'static str {
                match opcode {
                    $($constant => stringify!($variant),)*
                    _ => "unknown",
                }
            }

            /// Does this opcode move a client's bytes through a registered
            /// buffer?
            ///
            /// `None` for an opcode outside this space, which is a different
            /// answer from *no* and is kept different on purpose: a caller
            /// that collapsed the two would treat an unknown opcode as one
            /// that names no buffer, and go on to read its payload.
            ///
            /// The answer is on the line that declares the opcode, so a sixth
            /// cannot be added without deciding it — and
            /// [`super::Request::decode`] turns the decision into a refusal in
            /// both directions: an opcode that moves bytes and names no buffer
            /// is refused, and an opcode that moves none and names one is
            /// refused for the reason any unread field is.
            #[must_use]
            pub const fn moves_bytes(opcode: u8) -> Option<bool> {
                match opcode {
                    $($constant => Some($moves),)*
                    _ => None,
                }
            }
        }

        // Every record fits the one payload width, emitted from the list that
        // declares the records rather than written out beside them.
        // `Writer::put` indexes on the strength of this, so a sixth record
        // that did not fit would be a panic in a service's drain loop;
        // emitting the bound means there is no sixth record that can exist
        // without it.
        $(
            const _: () = assert!(
                <$record as Record>::WIDTH <= PAYLOAD_BYTES,
                concat!(stringify!($record), " is wider than one payload slot"),
            );
        )*

        /// The widest record's own fields, over every record there is.
        ///
        /// Computed from the list rather than named, because naming the widest
        /// record is a fact that stops being true when somebody adds a wider
        /// one — quietly, since the assertion would still pass about the
        /// record it names.
        /// Unit: bytes.
        const WIDEST_RECORD_BYTES: usize = {
            let widths = [$(<$record as Record>::WIDTH),*];
            let mut widest = 0;
            let mut at = 0;
            while at < widths.len() {
                if widths[at] > widest {
                    widest = widths[at];
                }
                at += 1;
            }
            widest
        };

        /// What a request asks for, once its payload has been believed.
        ///
        /// One variant per opcode, emitted from the same list, so the set of
        /// things a request can be is the set of opcodes there are.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Entry {
            $(
                $(#[$about])*
                $variant($record),
            )*
        }

        impl Entry {
            /// One legal value of every opcode's record.
            ///
            /// Every test's corpus, and it is derived rather than written: a
            /// sixth opcode is covered by the tests that already exist, on the
            /// day it is declared, without anybody remembering.
            /// Unit: none — one entry per opcode, in declaration order.
            pub const SPECIMENS: [Self; op::COUNT] =
                [$(Self::$variant(<$record as Record>::SPECIMEN)),*];

            /// The opcode that names this entry.
            #[must_use]
            pub const fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant(_) => op::$constant,)*
                }
            }

            /// The bytes this record's own fields occupy, before the padding
            /// to [`PAYLOAD_BYTES`].
            /// Unit: bytes.
            #[must_use]
            pub const fn width(&self) -> usize {
                match self {
                    $(Self::$variant(_) => <$record as Record>::WIDTH,)*
                }
            }

            /// The payload as it crosses: the record's fields, then zero to
            /// [`PAYLOAD_BYTES`].
            #[must_use]
            pub fn payload(&self) -> [u8; PAYLOAD_BYTES] {
                match self {
                    $(Self::$variant(record) => record.to_payload(),)*
                }
            }

            /// The entry an opcode and a payload name.
            fn read(opcode: u8, raw: &[u8; PAYLOAD_BYTES]) -> Result<Self, Refusal> {
                match opcode {
                    $(
                        opcode_pattern!($constant) =>
                            Ok(Self::$variant(<$record as Record>::from_payload(raw)?)),
                    )*
                    _ => Err(Refusal::UnknownOpcode),
                }
            }
        }
    };
}

entries! {
    /// Read the content a hash names into the caller's registered buffer.
    ///
    /// The opcode `E2-B08` exists for, and one whose completion carries a
    /// count: a short read is *stated* in `Cqe::result` rather than inferred
    /// from bytes that did not change.
    Read / Read / READ = 1, moves_bytes: true;

    /// Write the caller's registered buffer into an object at an offset.
    ///
    /// The entry RFC 0058's denominator is a sum over. See the module's *where
    /// an application byte is counted*: the field is [`Write::bytes`] and not
    /// [`Sqe::len`].
    Write / Write / WRITE = 2, moves_bytes: true;

    /// Keep a generation reachable, by its root hash. RFC 0059.
    ///
    /// Its own opcode rather than a flag or a field, because RFC 0059 says a
    /// pin is *an opcode on the objects ring with a capability behind it* —
    /// the capability being [`Sqe::cap`], which every opcode here reads.
    Pin / Pin / PIN = 3, moves_bytes: false;

    /// Release a pin, by the same root hash. Schedules the re-mark. RFC 0059.
    Unpin / Unpin / UNPIN = 4, moves_bytes: false;

    /// Start a collection cycle, completing when there is nothing left to do.
    ///
    /// RFC 0059's third pin opcode, and the one that carries nothing at all: a
    /// collect names no generation, because the set it works over is every
    /// generation that is not pinned.
    Collect / Collect / COLLECT = 5, moves_bytes: false;
}

/// Read the content a hash names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Read {
    /// The content address of what to read.
    /// Unit: bytes, exactly 32 of them — a content address, in the order
    /// FIPS 180-4 produces it and printed as 64 hexadecimal characters. All
    /// zero is a hash like any other and is not a sentinel: what refuses a
    /// zeroed payload is the opcode, not this field.
    pub hash: [u8; 32],
    /// How much of that content to read into the registered buffer.
    /// Unit: bytes. Zero is a zero-length read, which is valid and distinct
    /// from an absent one — it is how a client asks whether a hash resolves
    /// without moving anything.
    pub bytes: u32,
}

impl Record for Read {
    const WIDTH: usize = 32 + 4;
    // A hash whose bytes are all distinct and a length that is neither zero
    // nor a power of two: a decoder that read the hash one byte off, or the
    // length at the wrong width, produces a different value from this one.
    const SPECIMEN: Self = Self { hash: hash_specimen(0x10), bytes: 4_095 };

    fn write(&self, out: &mut Writer) {
        out.hash(&self.hash);
        out.u32(self.bytes);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let hash = raw.hash();
        let bytes = raw.u32();
        // Whether the hash resolves is a question about a device, and whether
        // `bytes` fits the caller's buffer is a question about a registration
        // this format deliberately does not repeat — RFC 0024 puts the set and
        // the index in the entry itself. `store::code::SHORT_BUFFER` is the
        // refusal for the second, at the layer that knows the answer.
        Ok(Self { hash, bytes })
    }
}

/// Write the caller's buffer into an object.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Write {
    /// Where in the object the write lands.
    /// Unit: bytes from the start of the object, not from the start of any
    /// device or zone. Zero is the first byte of the object.
    pub offset: u64,
    /// How many bytes this entry submits.
    ///
    /// **Claim 0017's denominator is a sum of this field.** RFC 0058: a 4 KiB
    /// write is 4096 whether it lands aligned or straddling, whether the piece
    /// was already dirty, and whatever the device does afterwards. The
    /// module's *where an application byte is counted* is why it is this field
    /// and not [`Sqe::len`], and what would reverse that.
    /// Unit: bytes. Zero is a zero-length write, which is valid, contributes
    /// zero to the denominator, and is distinct from an absent one.
    pub bytes: u32,
}

impl Record for Write {
    const WIDTH: usize = 8 + 4;
    // An offset past four gibibytes, so that a decoder which narrowed this
    // field to the width of `bytes` cannot round-trip it, and a length that is
    // one past a round number.
    const SPECIMEN: Self = Self { offset: 0x0000_0007_1234_5678, bytes: 262_145 };

    fn write(&self, out: &mut Writer) {
        out.u64(self.offset);
        out.u32(self.bytes);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let offset = raw.u64();
        let bytes = raw.u32();
        Ok(Self { offset, bytes })
    }
}

/// Keep a generation reachable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    /// The generation root this pin names.
    /// Unit: bytes, exactly 32 of them — a generation root hash, in the order
    /// FIPS 180-4 produces it. RFC 0059 refuses a pin whose root does not
    /// resolve, which is a question about a device and not about this record.
    pub root: [u8; 32],
}

impl Record for Pin {
    const WIDTH: usize = 32;
    const SPECIMEN: Self = Self { root: hash_specimen(0x40) };

    fn write(&self, out: &mut Writer) {
        out.hash(&self.root);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        Ok(Self { root: raw.hash() })
    }
}

/// Release a pin.
///
/// The same one field as [`Pin`], and a separate record rather than a flag on
/// one opcode: a flag would make *pin* and *unpin* two values of a field a
/// decoder has to read before it knows which operation it is looking at, and
/// the opcode is the field this format already reads first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unpin {
    /// The generation root whose pin is released.
    /// Unit: bytes, exactly 32 of them — a generation root hash, in the order
    /// FIPS 180-4 produces it.
    pub root: [u8; 32],
}

impl Record for Unpin {
    const WIDTH: usize = 32;
    // Deliberately not `Pin::SPECIMEN`'s bytes: two records that round-trip
    // through one corpus should not be able to pass by being confused with
    // each other.
    const SPECIMEN: Self = Self { root: hash_specimen(0x80) };

    fn write(&self, out: &mut Writer) {
        out.hash(&self.root);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        Ok(Self { root: raw.hash() })
    }
}

/// Start a collection cycle.
///
/// A record with no fields, which is a statement rather than a placeholder: a
/// collect names nothing, so its payload is [`PAYLOAD_BYTES`] of zero and a
/// single non-zero byte anywhere in it is refused. That is the whole of what
/// this record buys, and it is not nothing — it is what stops a later diff
/// from smuggling a parameter into a collect without widening the format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Collect;

impl Record for Collect {
    const WIDTH: usize = 0;
    const SPECIMEN: Self = Self;

    fn write(&self, _out: &mut Writer) {}

    fn read(_raw: &mut Reader) -> Result<Self, Refusal> {
        Ok(Self)
    }
}

/// A hash whose every byte differs from its neighbours, from one seed.
///
/// A specimen and nothing else — it is not a hash of anything. Written as a
/// function rather than as three 32-byte literals because the literals are
/// noise a reader skips, and what matters about them is the one property this
/// states in a line: no two adjacent bytes alike, so a decoder reading a hash
/// one byte off produces a different value.
const fn hash_specimen(first: u8) -> [u8; 32] {
    let mut out = [0u8; 32];
    let mut at = 0;
    while at < 32 {
        out[at] = first.wrapping_add(at as u8);
        at += 1;
    }
    out
}

/// One objects request as it crosses: part I's envelope, and the body its
/// opcode names.
///
/// Every field here is either read by this format or refused, and there is no
/// third category — which is the sentence the whole module is arranged to make
/// true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Request {
    /// Returned verbatim in the completion, and opaque here.
    /// Unit: none — chosen by the submitter and never interpreted. Zero is a
    /// legal token; a client that wants to match completions chooses
    /// otherwise.
    pub user_data: u64,
    /// The capability this request is made under.
    ///
    /// Read by every opcode, which is why there is no per-opcode answer for
    /// it: RFC 0059 says a pin is an opcode *with a capability behind it*, and
    /// a read or a write with no authority behind it is not a thing this ring
    /// offers. Nothing here refuses a value — what an index names is the
    /// service's question, and a forged one fails the frame's bounds check.
    /// Unit: capability-table slots, zero-based, in the submitter's own table.
    /// Zero is a valid slot and not a null.
    pub cap: u32,
    /// The scheduling class, and the depth its urgency has already crossed.
    ///
    /// Carried and not re-decided. It is the ring's field and not this
    /// format's: the service that drains the ring orders by it, and
    /// [`deadline::inherit`](crate::deadline::inherit) is what decides what it
    /// means.
    /// Unit: none — a `class` field: an ordinal in the low byte, a depth in
    /// the high one. Zero is `class::HARD` at depth zero.
    pub class: u16,
    /// The deadline the entry was scheduled against.
    ///
    /// Carried on every opcode rather than reserved to one, and that is a
    /// decision: storage requests are latency-bearing — claim 0020 is about a
    /// read path's — so a format that refused a deadline here would be
    /// refusing the field the scheduler already reads. This module neither
    /// re-decides it nor writes a second copy of it, which is the mistake
    /// [`scene`](crate::scene)'s *the commit* is about.
    /// Unit: nanoseconds, monotonic, in this channel's epoch — RFC 0009.
    /// [`NO_DEADLINE`](crate::NO_DEADLINE) is zero and is an unscheduled
    /// request rather than one that is already late.
    pub deadline: u64,
    /// Where this request's payload is.
    /// Unit: bytes from the first byte of the channel's inline arena, not from
    /// the start of the mapping — [`crate::op::WRITE_SERIAL`]'s origin, and
    /// for its reason. Zero is the first byte of the arena.
    pub payload_offset: u32,
    /// The registered buffer set the client's bytes cross in.
    /// Unit: registered-set identifiers, packed as
    /// [`buf::SetId`](crate::buf::SetId). Zero is slot zero at generation
    /// zero, which was never issued — so it names no set, and an opcode that
    /// moves bytes is refused [`Refusal::NoBuffer`] for it.
    pub buf_set: u32,
    /// Which buffer of that set.
    /// Unit: buffers within `buf_set`, zero-based. Zero is the first buffer
    /// and is legal.
    pub buf_index: u32,
    /// Submission flags.
    /// Unit: none — a bitmask, and a subset of [`FLAGS_ACCEPTED`]. Zero is
    /// legal only on an opcode that moves no bytes; the other two require
    /// [`flags::FIXED_BUF`].
    pub flags: u8,
    /// What the request asks for.
    /// Unit: none — one opcode's record.
    pub body: Entry,
}

impl Request {
    /// The opcode this request submits under.
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        self.body.opcode()
    }

    /// Is this a request a peer would accept?
    ///
    /// The value-level half of [`Request::decode`]'s rules that does not need
    /// the payload's bytes, exposed so that a client can refuse its own entry
    /// before submitting it rather than learning about it in a completion. It
    /// is the same code path, not a second statement of the same rules.
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownFlag`] for a flag outside [`FLAGS_ACCEPTED`];
    /// [`Refusal::NoBuffer`] for an opcode that moves bytes and names no
    /// registered buffer; [`Refusal::Reserved`] for a buffer named by an
    /// opcode that moves none.
    pub const fn check(&self) -> Result<(), Refusal> {
        envelope_rules(self.opcode(), self.flags, self.buf_set, self.buf_index)
    }

    /// The submission entry this request crosses in.
    ///
    /// Written field by field, with no `..Sqe::ZERO`, so that a field added to
    /// [`Sqe`] stops this build here and asks whether an objects request reads
    /// it. [`Request::decode`] then compares an arriving entry against exactly
    /// this, which turns whatever is answered into something a peer cannot get
    /// wrong.
    #[must_use]
    pub const fn envelope(&self) -> Sqe {
        Sqe {
            opcode: self.opcode(),
            flags: self.flags,
            class: self.class,
            cap: self.cap,
            user_data: self.user_data,
            deadline: self.deadline,
            offset: self.payload_offset as u64,
            buf_set: self.buf_set,
            buf_index: self.buf_index,
            len: PAYLOAD_BYTES as u32,
            _reserved: 0,
            // Not a spare pair of words. A field that belongs to an opcode
            // belongs in that opcode's record, where the record's own width
            // states it and `Reader::finish` polices it; a field in the
            // envelope would be a field every opcode shares whether it reads
            // it or not. These are also the eight bytes the module's *where an
            // application byte is counted* names as its reversal condition: a
            // record that fitted here would free `len`.
            ext: [0, 0],
        }
    }

    /// Encode: the entry, and the payload it points at.
    ///
    /// Total, on purpose. It writes what it was given, including a value
    /// [`Request::decode`] would refuse — which is what lets a test build a
    /// malformed entry without a second encoder. A producer that wants the
    /// check calls [`Request::check`] first.
    #[must_use]
    pub fn encode(&self) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        (self.envelope(), self.body.payload())
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The order is the order a refusal is distinguishable in: the opcode
    /// first, because every later check depends on which opcode this is; the
    /// flags and the buffer next, because they are what a client acts on
    /// differently; the framing; then the payload; then the whole-envelope
    /// comparison last, because it is the catch-all nothing can be added past.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]. [`Refusal::Reserved`] is the one worth naming: it is
    /// what an entry gets for any byte, anywhere, that this build does not
    /// read.
    pub fn decode(entry: &Sqe, payload: &[u8; PAYLOAD_BYTES]) -> Result<Self, Refusal> {
        envelope_rules(entry.opcode, entry.flags, entry.buf_set, entry.buf_index)?;
        if entry.len as usize != PAYLOAD_BYTES {
            return Err(Refusal::Malformed);
        }
        // A channel mapping is a `u32` of bytes — `layout::MAX_ENTRIES` is
        // chosen so that every offset in one fits with room to spare — so an
        // offset past that is not an offset into any arena. Refused here
        // rather than truncated into the field below, because a truncation
        // would make the comparison that follows fail as *a reserved field is
        // not zero*, which is a true sentence about the wrong field.
        if entry.offset > u64::from(u32::MAX) {
            return Err(Refusal::Malformed);
        }

        let body = Entry::read(entry.opcode, payload)?;
        let request = Self {
            user_data: entry.user_data,
            cap: entry.cap,
            class: entry.class,
            deadline: entry.deadline,
            payload_offset: entry.offset as u32,
            buf_set: entry.buf_set,
            buf_index: entry.buf_index,
            flags: entry.flags,
            body,
        };

        // The check this module exists for, on the envelope half. Not a list
        // of the fields an objects request ignores — such a list is correct on
        // the day it is written and silent afterwards — but the entry this
        // build would have produced from the fields it just read, compared in
        // full. Any field `envelope` does not set, including one a later ABI
        // adds, must arrive zero.
        if sqe_bytes(&request.envelope()) != sqe_bytes(entry) {
            return Err(Refusal::Reserved);
        }
        Ok(request)
    }
}

/// The envelope rules that do not need the payload.
///
/// One function, called by [`Request::check`] before submitting and by
/// [`Request::decode`] after receiving, so that a producer and a consumer
/// cannot hold different opinions about which entries are legal.
const fn envelope_rules(
    opcode: u8,
    flags: u8,
    buf_set: u32,
    buf_index: u32,
) -> Result<(), Refusal> {
    let Some(moves) = op::moves_bytes(opcode) else {
        return Err(Refusal::UnknownOpcode);
    };
    if flags & !FLAGS_ACCEPTED != 0 {
        return Err(Refusal::UnknownFlag);
    }
    if moves {
        // Both halves, because either one alone is an entry whose destination
        // nobody can resolve: the flag is what makes the two words a set
        // identifier rather than half of an address (RFC 0024), and a set of
        // zero was never issued.
        if flags & flags::FIXED_BUF == 0 || buf_set == 0 {
            return Err(Refusal::NoBuffer);
        }
    } else if flags & flags::FIXED_BUF != 0 || buf_set != 0 || buf_index != 0 {
        // A buffer named by an opcode that moves nothing is a field the opcode
        // does not read, refused for the reason every such field is: the peer
        // that set it believes something happened that did not.
        return Err(Refusal::Reserved);
    }
    Ok(())
}

/// The sixty-four bytes of a submission entry.
///
/// A byte view rather than a field-by-field comparison, and the difference is
/// the whole of [`Request::decode`]'s guarantee: a comparison written out by
/// hand covers the fields somebody listed, and this one covers the fields
/// there are.
///
/// The fifth copy of this function in this crate — `scene`, `input`,
/// `participate` and `semantic` each carry one. Copied rather than shared
/// because sharing it is an edit to four landed wire formats and belongs in a
/// diff of its own; the count is written here so that the diff which pays it
/// can find all five.
fn sqe_bytes(entry: &Sqe) -> &[u8; SQE_BYTES] {
    // SAFETY: `Sqe` is `#[repr(C, align(64))]`, so its fields are laid out in
    // declaration order at fixed offsets; `size_of::<Sqe>()` is `SQE_BYTES`
    // and `SQE_FIELD_BYTES` below adds up the width of each of its fields as
    // the field's own type states it, which the assertion beside it requires
    // to be the same number — so the type carries no padding and every one of
    // its bytes is an initialised byte of an integer. `[u8; SQE_BYTES]` has
    // alignment one, which `Sqe`'s sixty-four satisfies. The reference
    // produced borrows `entry` for its own lifetime and is shared, so nothing
    // is mutated through it and nothing outlives it.
    unsafe { &*core::ptr::from_ref(entry).cast::<[u8; SQE_BYTES]>() }
}

/// The width of the field handed in, taken from the field's own type.
///
/// The type parameter is inferred at the call site, so the answer is the
/// declaration's and not a number somebody typed next to it.
const fn field_bytes<T: Copy>(_field: T) -> usize {
    core::mem::size_of::<T>()
}

/// What [`Sqe`]'s fields occupy, added up from the fields themselves.
///
/// Every term is `field_bytes(<the field>)`, so a field that changes width
/// changes this number without anybody editing it.
/// Unit: bytes.
const SQE_FIELD_BYTES: usize = {
    let entry = Sqe::ZERO;
    field_bytes(entry.opcode)
        + field_bytes(entry.flags)
        + field_bytes(entry.class)
        + field_bytes(entry.cap)
        + field_bytes(entry.user_data)
        + field_bytes(entry.deadline)
        + field_bytes(entry.offset)
        + field_bytes(entry.buf_set)
        + field_bytes(entry.buf_index)
        + field_bytes(entry.len)
        + field_bytes(entry._reserved)
        + field_bytes(entry.ext)
};

// The two facts `sqe_bytes` rests on, asserted here rather than trusted from
// another file: this is the code that would be unsound without them.
const _: () = assert!(core::mem::size_of::<Sqe>() == SQE_BYTES);
const _: () = assert!(
    SQE_FIELD_BYTES == SQE_BYTES,
    "Sqe carries padding, so sqe_bytes would read uninitialised memory"
);

// And the stride is the widest record's width rounded to eight. A record that
// grows past it fails the per-record assertion `entries!` emits; this one is
// what makes a *narrowing* — a field deleted, leaving the stride eight bytes
// wider than anything uses — visible rather than free.
const _: () = assert!(PAYLOAD_BYTES == WIDEST_RECORD_BYTES.next_multiple_of(8));

// The flags this format accepts are flags the ABI defines. A bit removed from
// `flags::KNOWN` and left here would be a bit this build accepts on an entry
// and the ring's own envelope check refuses.
const _: () = assert!(FLAGS_ACCEPTED & !flags::KNOWN == 0);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{buf, class, deadline};

    /// A request around a body, with an envelope that is legal for it.
    ///
    /// Derived from [`op::moves_bytes`] rather than written per opcode, so
    /// that a sixth opcode gets a legal envelope from the list that declared
    /// it, and every test below covers it without being edited.
    fn request(body: Entry) -> Request {
        let moves = op::moves_bytes(body.opcode()) == Some(true);
        Request {
            user_data: 0x0102_0304_0506_0708,
            cap: 9,
            class: deadline::pack(class::SOFT, 1),
            deadline: 0x0000_0002_1871_1A00,
            payload_offset: 0x0E00,
            buf_set: if moves { 0x0001_0002 } else { 0 },
            buf_index: if moves { 3 } else { 0 },
            flags: if moves { flags::FIXED_BUF } else { 0 },
            body,
        }
    }

    #[test]
    fn every_opcode_round_trips_through_its_bytes() {
        // The corpus is `Entry::SPECIMENS`, emitted from the same list as the
        // opcodes, so a sixth opcode is in it the day it is declared. It runs
        // on x86-64 and, through `cargo xtask test` on the arm runner, on
        // AArch64 — which is what the encoding being written by hand is for:
        // every field goes out through `to_le_bytes` and comes back through
        // `from_le_bytes`, so nothing here can inherit the host's word size,
        // its alignment or its byte order.
        //
        // The edit that makes this fail: drop a field from a record's `write`
        // or its `read`, give the two different field orders, or narrow
        // `Write::offset` to a `u32`. `Collect` is the one member of the
        // corpus this proves nothing about — a record with no fields
        // round-trips whatever the writer does — and
        // `a_non_zero_byte_past_a_record_is_refused` is what covers it
        // instead.
        assert_eq!(Entry::SPECIMENS.len(), op::COUNT);
        for body in Entry::SPECIMENS {
            let original = request(body);
            let label = op::label(original.opcode());
            let (entry, payload) = original.encode();
            assert_eq!(entry.len as usize, PAYLOAD_BYTES, "{label}");
            assert_eq!(original.check(), Ok(()), "{label}");
            let back = Request::decode(&entry, &payload)
                .unwrap_or_else(|refusal| panic!("{label}: {}", refusal.message()));
            assert_eq!(back, original, "{label}");

            // And the encoding is a function of the value alone: the same
            // request encodes to the same bytes, which is what makes two
            // architectures produce one wire image.
            let (again, again_payload) = back.encode();
            assert_eq!(sqe_bytes(&again), sqe_bytes(&entry), "{label}");
            assert_eq!(again_payload, payload, "{label}");
        }
    }

    #[test]
    fn an_opcode_this_vocabulary_does_not_name_is_refused() {
        // R04, and the case that matters most: a zeroed slot of a fresh
        // mapping is not opcode zero doing nothing, it is an entry nobody
        // wrote. Opcode zero is outside this space, which is why the five
        // start at one.
        //
        // The edit that makes this fail: renumber an opcode to zero, or give
        // `op::moves_bytes` a wildcard arm answering `Some`. The second is the
        // one worth naming, because it is where the refusal actually lives —
        // `envelope_rules` reads `moves_bytes` first and refuses `None` before
        // any field of the entry is looked at, so the wildcard arm in
        // `Entry::read` is unreachable from `Request::decode` and a mutation
        // of *it* does not turn this test red. It was tried. The arm stays
        // because `Entry::read` is reachable from inside this module and a
        // decoder with no total dispatch is a decoder waiting for a second
        // caller.
        assert_eq!(Request::decode(&Sqe::ZERO, &[0; PAYLOAD_BYTES]), Err(Refusal::UnknownOpcode));
        assert!(!op::defined(0));
        assert_eq!(op::moves_bytes(0), None);

        // And the top of the space is RFC 0028's, not this service's.
        let mut foreign = request(Entry::Collect(Collect)).envelope();
        foreign.opcode = buf::opcode::REGISTER;
        assert_eq!(
            Request::decode(&foreign, &[0; PAYLOAD_BYTES]),
            Err(Refusal::UnknownOpcode),
            "the top of the space is RFC 0028's, not this service's"
        );

        for opcode in 0..=u8::MAX {
            assert_eq!(op::defined(opcode), op::ALL.contains(&opcode));
            assert_eq!(op::defined(opcode), op::moves_bytes(opcode).is_some());
        }
    }

    #[test]
    fn known_admits_read_and_write_and_nothing_else() {
        // The assertion that makes this module honest rather than decorative,
        // and it is the one this diff rewrote rather than deleted. It used to
        // say *none of the five*, which was true while the numbers were
        // assigned and nothing answered them; `f_objects::service` answers
        // `READ`, so the sentence is now *one of the five* and the other four
        // are held exactly where the first one was.
        //
        // Written as a partition rather than as one `assert!(known(READ))`,
        // because the interesting failure is not the opcode that is answered.
        // It is a second arm appearing beside it with no service behind it —
        // which is what `op::known`'s own comment says must not be possible by
        // accident, and which this loop is what catches.
        //
        // The edit that makes this fail: widen `known` to admit a further
        // opcode, or narrow it back. Both are decisions and both are red here.
        //
        // **That happened, exactly as this comment said it would.** The text
        // above read *a diff that answers `WRITE` fixes this test by naming
        // `WRITE` beside `READ` — one line, in the same diff as the body*, and
        // that is the diff `E2-B09` is: `f_objects::write::WritePath` is the
        // body, and this is the line. The sentence is kept rather than edited
        // away because a prediction that came true is worth more standing than
        // rewritten, and the same sentence now governs `PIN`, `UNPIN` and
        // `COLLECT` — three numbers with no service behind them.
        const ANSWERED: [u8; 2] = [op::READ, op::WRITE];

        for opcode in op::ALL {
            assert!(op::defined(opcode), "{} is not in its own vocabulary", op::label(opcode));
            assert_eq!(
                op::known(opcode),
                ANSWERED.contains(&opcode),
                "{} is answered by something this test does not name",
                op::label(opcode)
            );
        }
        // And nothing outside the space, which is the half that says `known`
        // is a predicate over this vocabulary rather than a catch-all: an
        // opcode no service ever heard of gets the same refusal as one of the
        // four that are declared and unanswered.
        for opcode in 0..=u8::MAX {
            if !op::defined(opcode) {
                assert!(!op::known(opcode), "{opcode} is outside this space and is answered");
            }
        }
        assert_eq!(
            Refusal::UnknownOpcode.packed(),
            error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE)
        );
    }

    #[test]
    fn a_non_zero_byte_past_a_record_is_refused() {
        // The payload half of *an unread field is refused rather than
        // ignored*, over every record there is — including `Collect`, whose
        // declared width is zero, so the whole payload is its tail.
        //
        // The edit that makes this fail: stop `Reader::finish` walking to
        // `PAYLOAD_BYTES`, stop `Record::from_payload` calling it, or widen a
        // record's `WIDTH` past what its decoder consumes.
        for body in Entry::SPECIMENS {
            let original = request(body);
            let label = op::label(original.opcode());
            let (entry, payload) = original.encode();
            for at in original.body.width()..PAYLOAD_BYTES {
                let mut tampered = payload;
                tampered[at] = 0xA5;
                assert_eq!(
                    Request::decode(&entry, &tampered),
                    Err(Refusal::Reserved),
                    "{label}: byte {at} past the record was ignored"
                );
            }
        }
    }

    #[test]
    fn the_five_numbers_are_contiguous_and_collide_with_nothing() {
        // The numbers are what this diff exists to fix in place, and they are
        // free to move only until a second peer reads them. Contiguous from
        // one, per RFC 0059's *assigned contiguously when the objects
        // service's `op` module is first written*; away from the top of the
        // byte, which RFC 0028 reserves in every service's space.
        //
        // The edit that makes this fail: renumber an opcode, add one out of
        // sequence, or give a future opcode `0xFE` or `0xFF`.
        // `abi/src/store.rs`'s `code` module already apologises once for two
        // files holding one number space, which is why this is checked rather
        // than assumed.
        assert_eq!(op::COUNT, 5);
        assert_eq!(op::ALL, [op::READ, op::WRITE, op::PIN, op::UNPIN, op::COLLECT]);
        for (index, opcode) in op::ALL.iter().enumerate() {
            assert_eq!(usize::from(*opcode), index + 1);
            assert!(!buf::opcode::is_registration(*opcode));
        }
    }
}
