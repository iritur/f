// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The input entry format: six opcodes, one fixed-width payload, and the stamp
//! that every one of them carries.
//!
//! # What crosses, and what carries it
//!
//! `docs/design/ring-scene-boot.html` section 09 makes the input path a chain
//! of stages with one number running through it — the event happened at `t = 0`
//! and the screen showed it later — and this module is the wire form of one
//! link in that chain. There is no second transport and no second framing: an
//! event is an [`Sqe`] whose opcode names what the device did, and a
//! fixed-width record in the channel's inline arena that the entry points at.
//! [`layout`](crate::layout) is where the arena is; this module is what is in
//! it.
//!
//! [`scene`](crate::scene) is the sibling format this one is deliberately
//! shaped against — same envelope, same stride discipline, same two halves of
//! *an unread field is refused*. Two entry formats in one crate that disagreed
//! about how a non-zero unread field is refused would be one rule with two
//! implementations, which is the defect R04 exists to prevent rather than an
//! instance of following it. Where this format diverges — the deadline, below —
//! it diverges out loud and says why.
//!
//! # Why one width for all six
//!
//! Every payload is [`PAYLOAD_BYTES`] long whatever the opcode, so a queue of
//! events in the arena is an array with a stride rather than a list that must
//! be parsed to be walked. Section 09's *late-latch* is what buys it: at the
//! last moment before GPU submit the compositor wants the **newest** pointer or
//! stylus position in the queue and nothing else, and with a stride it finds
//! that by walking backwards from the producer's index. With a
//! length-prefixed stream it would have to parse forwards through every event
//! it is trying to skip — paying, on the hottest path in the system, exactly in
//! proportion to how far behind it had fallen. A late-latch that gets slower the
//! later it is is a late-latch that amplifies the problem it exists to solve.
//!
//! Two lesser reasons, both real. A consumer draining a driver's arena must be
//! able to bound its walk before it believes any field in it — section 05's
//! *shared memory is untrusted input, always* — and a bound that depends on a
//! length field is a bound the producer chooses. And a producer at interrupt
//! time writes slot *n* without having finished slot *n − 1*, which is what lets
//! a driver stamp and enqueue in the interrupt path rather than in a loop that
//! has to run in order.
//!
//! The cost is the padding: the widest records are [`TouchPoint`] and
//! [`StylusPoint`] at 14 bytes, and a [`Key`] is six, so a keystroke carries
//! eight bytes of nothing on top of the stamp. That is a fraction of one cache line per event
//! on a path whose budget is stated in milliseconds, and the arithmetic is not
//! close. *What would reverse this:* an input corpus dominated by one narrow
//! opcode on a channel whose arena is the binding constraint, measured rather
//! than supposed, at which point the stride becomes per-opcode and the
//! backwards walk stops being arithmetic — which is to say the late-latch
//! argument above is the thing being sold.
//!
//! # The stamp, and why its scale is in its name
//!
//! Every payload begins with [`Event::stamp_nanos`], and the unit is in the
//! field's name rather than in a comment beside it for the reason `E3-B04a`
//! spends a module arguing: the number this path exists to bound is
//! `presented_at - stamped_at`, and a stamp that is nanoseconds in the driver
//! and microseconds in the compositor is a latency figure that is wrong by
//! three orders of magnitude and looks plausible. Nothing in the language will
//! find that. A name will, at every site that reads it, which is why `nanos`
//! is also the name of the accessor on `f_input::StampNanos` and of the
//! constructor `f_input::StampNanos::from_wire_nanos` that rebuilds one from
//! this field. The scale is restated at every boundary the number crosses.
//!
//! The stamp is in the **payload** rather than in [`Sqe::ext`], and the two are
//! not equally good. A field in the envelope is a field every opcode of every
//! service shares whether it reads it or not, and `ext`'s own contract is that
//! an opcode reading it states what it holds — six statements of one fact. In
//! the payload it is read in exactly one place, [`Event::decode`], before the
//! opcode is dispatched at all, so a seventh opcode is stamped on the day it is
//! declared and there is no decoder that could forget. It is also the reason
//! there is no seventh opcode for *these events happened together*: two entries
//! bearing one stamp are one instant, because there is only one clock reading
//! per instant to bear.
//!
//! # An unread field is refused, and it is refused structurally
//!
//! R04 is the rule — a thing this build does not know is refused and never
//! ignored — and the hard half of it is the *field* rather than the opcode: a
//! driver that sets something this build does not read has a belief about what
//! the user just did that this build does not share, and the two of them will
//! not find out. Neither half of an entry is checked against a list of fields
//! written out by hand:
//!
//! - **The payload.** The `Reader` an event decodes through counts what was
//!   consumed — the stamp, then the record's own fields — and `Reader::finish`
//!   requires every byte past that to be zero. A field a record does not read
//!   is a field in the tail, and the tail is refused. The rule is written once,
//!   so a seventh record inherits it by existing.
//! - **The envelope.** [`Event::decode`] rebuilds the [`Sqe`] this build would
//!   have written from the fields it actually read, and compares all sixty-four
//!   bytes. Any field of `Sqe` the encoder does not set — today `cap`,
//!   `buf_set`, `buf_index`, `deadline`, `len`'s other values, `_reserved`,
//!   `ext`, and anything a later ABI adds — must arrive zero, because the
//!   canonical entry has it zero.
//!
//! [`Event::envelope`] is written field by field rather than with
//! `..Sqe::ZERO`, so a field added to `Sqe` stops *this* build here and asks
//! whether an input event reads it. The byte comparison then makes whatever is
//! answered enforceable instead of documentary.
//!
//! # No input entry carries a deadline
//!
//! Flatly, for every opcode, with no per-opcode exception — which is where this
//! format and [`scene`](crate::scene) part company, and the difference is the
//! two formats' subjects rather than an oversight. A scene delta's commit *is*
//! scheduled work: it names a frame the compositor has promised to finish by a
//! time. An input event is a thing that already happened. Asking the ring to
//! order it against a deadline would be asking a scheduler to hurry up the
//! past, and the deadline that actually matters — the one section 09's chain is
//! computed backwards from — belongs to the frame that will show this event,
//! is set by `E3-B01h`, and has not been decided yet at the moment this entry
//! is written.
//!
//! So [`Sqe::deadline`] is a field no opcode here reads, and it goes through
//! the same refusal as any other unread field: [`NO_DEADLINE`] or
//! [`Refusal::Reserved`]. Stating it once rather than per opcode is deliberate.
//! A seventh opcode inherits the rule by existing, and there is no line on
//! which one could claim otherwise. *What would reverse this:* an input opcode
//! that is a request rather than a report — a driver asking to be told
//! something by a time — which is a different kind of entry and should say so
//! with an RFC rather than by widening this one.
//!
//! # No clock, no floating point, no allocator
//!
//! Nothing here reads a clock, and this crate could not: `f-abi` has no
//! dependency on `f-env`, and `f-input` depends on `f-abi` rather than the
//! other way round, which is what puts the one clock reading in
//! `input/src/stamp.rs` and a plain `u64` here. Encoding and decoding are pure
//! functions of the values they are handed, which is stronger than drawing one
//! deterministically: a function that takes its time as a parameter has nothing
//! to seed.
//!
//! Nothing here is binary floating point either. A pointer position is 16.16
//! fixed point in an `i32` and a pressure is a `u16`, both with the scale in the
//! name, because the two architectures do not agree on that arithmetic and a
//! cursor that landed on a different pixel on one of them would be the whole
//! thesis failing quietly. RFC 0004.
//!
//! # Why the refusals are local
//!
//! [`Refusal`] is this module's own enum, in the shape
//! [`scene::Refusal`](crate::scene::Refusal) and
//! [`manifest::Refusal`](crate::manifest::Refusal) already use, and it packs
//! into the domains RFC 0010 fixes without adding a code to
//! [`error::argument`]. Adding one was the obvious move and is the wrong one
//! this week: several sibling entry formats are being written at once, and a
//! ninth `argument` code invented in each of four places is four meanings
//! wearing one number. Every refusal below reuses a code that already means
//! what it says.
//!
//! # A seventh opcode
//!
//! It is an RFC, by the rule that makes every change to this crate an ABI
//! change, and it is also a diff to one list: the `events!` invocation below
//! emits the opcode constants, the [`Entry`] variants, the dispatch and the
//! specimens every test in this file runs over. There is no second place a
//! seventh opcode could be written, and no way to add one that the round-trip
//! test, the byte-flip test and the tail test do not pick up.
//!
//! The opcode space is the input service's own. Section 05 makes an opcode
//! space per-service, so these numbers are not [`crate::op`]'s, not
//! [`scene::op`](crate::scene::op)'s and not the semantic ring's; nothing
//! should compare an opcode across two of them.
//!
//! # Which device this came from
//!
//! No field says. The channel says: a driver component holds one ring per
//! device — `E3-B04d` builds it on virtio-input, where a device *is* a virtio
//! device — so a device identifier on the entry would be a second answer to
//! *which device* that can disagree with the first, and the one that wins would
//! be whichever the consumer happened to read. [`TouchPoint::contact`] is not
//! an exception: it distinguishes fingers on one device, which is a question
//! the channel cannot answer. *What would reverse this:* a seat service that
//! multiplexes several devices onto one ring, at which point the identifier
//! becomes a field in the shared header beside the stamp and every consumer has
//! to read it — a diff to this file, reviewed as an ABI change.

use crate::{NO_DEADLINE, Sqe, error, flags};

/// Bytes of inline arena one event's payload occupies, whatever its opcode.
///
/// The stamp plus the widest record, rounded up to the next multiple of eight
/// — arithmetic `events!` asserts from the list of records rather than a number
/// this comment is trusted to have kept up with: every multi-byte field in every record below
/// is at most eight wide, so a stride that is a multiple of eight gives the same
/// alignment to the same field in every slot of a queue. Nothing here casts a
/// payload to a struct — every field is encoded by hand — so the alignment buys
/// a reader's arithmetic rather than a compiler's, which is the only kind this
/// crate is allowed to want.
/// Unit: bytes.
pub const PAYLOAD_BYTES: usize = 24;

/// Bytes of every payload that are the stamp, before the record's own fields.
///
/// The shared header, and the whole of it. A second shared field would be a
/// field every opcode pays for, which is the argument the module's *the stamp*
/// makes against putting even this one in the envelope; it is here because
/// there is no input event that is not stamped.
/// Unit: bytes.
pub const STAMP_BYTES: usize = 8;

/// The width of a submission entry, which [`Event::decode`] compares in full.
///
/// Stated here as well as in `lib.rs` because the comparison depends on it and
/// on there being no padding inside an [`Sqe`]; the two assertions further down
/// this file are what make both facts rather than hopes.
/// Unit: bytes.
const SQE_BYTES: usize = 64;

/// No stamp. Zero, so that a zeroed payload is not an event.
///
/// A page of zeroes is what an untouched arena holds and what a faulted driver
/// leaves behind, and [`PointerMotion`] has no closed field to refuse one with:
/// without this rule a fresh mapping would decode as the pointer sitting at the
/// origin at the beginning of time, which is a scene the compositor would draw.
/// So the stamp is what makes a zeroed slot nothing at all, for every opcode
/// there is and every opcode there will be.
///
/// The cost is exactly one nanosecond of the channel's epoch: an event stamped
/// at the epoch's own zero cannot be expressed. That instant is the channel's
/// creation, before any device was attached to it to raise an interrupt, so
/// nothing real is lost. *What would reverse this:* a device that genuinely
/// stamps at the epoch's zero — for which the repair is to move the epoch
/// earlier than the device, not to make zero a time, because the alternative is
/// that a page of zeroes is a pointer at the origin.
/// Unit: nanoseconds — the absence of a stamp, not a stamp of zero.
pub const NOT_STAMPED: u64 = 0;

/// The submission flags an input event may carry.
///
/// [`flags::NO_CQE`] and nothing else, which is what every driver in the
/// shipped tree will set: a completion per mouse motion at a thousand events a
/// second is a cache line per event of traffic on the hottest path in the
/// system, carrying information nobody reads. RFC 0028's asymmetry is what
/// makes that safe — a *refused* event completes whatever this flag says — so
/// suppressing the success completion costs a driver nothing it needs.
///
/// Permitted rather than required, and the difference is one entry: a driver
/// that has just attached and wants to know its first event was believed has no
/// other way to ask, and a rule that forbade it would be a rule with no
/// refusal that honestly names it.
///
/// [`flags::LINK`] and [`flags::DRAIN`] are refused rather than honoured,
/// because input order is not negotiable and is already decided: the ring's
/// order is the order in which the user did things, and a second ordering
/// mechanism on the same entries is two answers to *what happened before what*
/// about the one sequence in the system where the user can see the answer.
/// [`flags::FIXED_BUF`] is refused because the payload is in the arena at
/// [`Event::payload_offset`]; an entry naming a registered buffer instead would
/// be an entry this format does not read.
/// Unit: none — a bitmask of the [`flags`] constants.
pub const FLAGS_ACCEPTED: u8 = flags::NO_CQE;

/// Which way a switch went.
///
/// A module of constants rather than an `enum`, which is this crate's shape
/// wherever a value arrives from a peer: an `enum` makes the known values a
/// type and leaves every decoder to invent what an unknown one is. Zero is
/// neither, so a zeroed payload does not decode as a release.
pub mod edge {
    /// The switch closed: a button or key went down.
    /// Unit: none — a transition, not a quantity.
    pub const PRESSED: u16 = 1;
    /// The switch opened: a button or key came up.
    /// Unit: none — a transition, not a quantity.
    pub const RELEASED: u16 = 2;

    /// Is this a transition this build knows? Zero is not one.
    ///
    /// The negative answer is the one that matters. A repeat is deliberately
    /// not a third value: key repeat is a timer, the timer belongs to whichever
    /// stage knows the user's repeat settings, and a driver that synthesised
    /// repeats onto the wire would make the rate un-turn-off-able by anything
    /// downstream.
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, PRESSED | RELEASED)
    }
}

/// Where a contact is in its life.
pub mod touch {
    /// A new contact: this identifier was not on the surface a moment ago.
    /// Unit: none — a phase, not a quantity.
    pub const DOWN: u16 = 1;
    /// An existing contact moved, or reported again without moving.
    /// Unit: none — a phase, not a quantity.
    pub const MOVE: u16 = 2;
    /// The contact was lifted. The identifier may be reused after this.
    /// Unit: none — a phase, not a quantity.
    pub const UP: u16 = 3;
    /// The contact is withdrawn rather than lifted: a palm the device rejected
    /// on second thoughts, or a gesture the compositor took for itself.
    ///
    /// Distinct from [`UP`] because a consumer must undo what the contact did
    /// rather than complete it, and a format that could not say which would
    /// make every stolen gesture also a stray tap.
    /// Unit: none — a phase, not a quantity.
    pub const CANCEL: u16 = 4;

    /// Is this a phase this build knows? Zero is not one.
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, DOWN | MOVE | UP | CANCEL)
    }
}

/// What produced a scroll.
pub mod axis_source {
    /// A detented wheel: the distance is one or more discrete steps, converted
    /// by the driver, which is the stage that knows the device's detent.
    /// Unit: none — a source, not a quantity.
    pub const WHEEL: u16 = 1;
    /// A finger on a surface: the distance is continuous, and the gesture has
    /// an end the consumer may want to glide out of.
    /// Unit: none — a source, not a quantity.
    pub const FINGER: u16 = 2;

    /// Is this a source this build knows? Zero is not one.
    ///
    /// The field exists because the two are handled differently and the
    /// difference is visible to a user: snapping a finger scroll to detents
    /// feels broken, and gliding a wheel click feels broken, and a consumer
    /// that could not tell them apart would have to pick one and be wrong half
    /// the time.
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, WHEEL | FINGER)
    }
}

/// Why an entry was not believed.
///
/// A value rather than a packed integer, for
/// [`store::refusal`](crate::store::refusal)'s reason: a caller compares
/// against these, and a test that spelled the packed integer out itself would
/// pass on a decoder that started returning the right refusal for the wrong
/// reason. [`Refusal::packed`] is the one place the mapping to RFC 0010's
/// domains is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// An opcode outside this service's six. Refused, never skipped: R04.
    UnknownOpcode,
    /// A submission flag outside [`FLAGS_ACCEPTED`]. Refused rather than masked
    /// off — a bit silently dropped is two peers with different beliefs about
    /// what just happened.
    UnknownFlag,
    /// A field this opcode does not read is not zero: a byte past the record's
    /// own fields, a deadline on an entry that reports rather than requests, or
    /// any part of the envelope this format never looks at. The refusal this
    /// module is shaped around.
    Reserved,
    /// The entry does not frame a payload: a length that is not
    /// [`PAYLOAD_BYTES`], or an arena offset that is not an arena offset.
    Malformed,
    /// A closed field carries a value outside its set — a transition, a touch
    /// phase or a scroll source.
    Value,
    /// The payload carries [`NOT_STAMPED`]. Separate from [`Refusal::Value`]
    /// because a zeroed slot of a fresh mapping produces exactly this, and
    /// because it is the one refusal that says *this event has no place in the
    /// latency chain* rather than *this field is wrong*.
    NotStamped,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownOpcode => "the opcode is not one of this service's six",
            Self::UnknownFlag => "the entry carries a submission flag an input event may not",
            Self::Reserved => "a field this opcode does not read is not zero",
            Self::Malformed => "the entry does not frame a payload of the one width there is",
            Self::Value => "a closed field carries a value outside its set",
            Self::NotStamped => "the event carries no timestamp, so its latency is unknowable",
        }
    }

    /// The refusal as a packed [`error`], for a completion.
    ///
    /// Every one is [`error::ARGUMENT`]: the channel is healthy and the peer is
    /// present, and what is wrong is one entry. No code here is new — see the
    /// module's *why the refusals are local* — so [`Refusal::Value`] and
    /// [`Refusal::NotStamped`] share [`error::argument::UNKNOWN_FLAG`], which is
    /// the code [`scene::Refusal`](crate::scene::Refusal) and
    /// [`store::refusal`](crate::store::refusal) already use for *a closed field
    /// carries a value this build does not accept*. They are distinguishable as
    /// values, which is where a caller distinguishes them; they are one code on
    /// the wire, which is where RFC 0010 says the domain is the stable part.
    #[must_use]
    pub const fn packed(self) -> i32 {
        let code = match self {
            Self::UnknownOpcode => error::argument::UNKNOWN_OPCODE,
            Self::Reserved => error::argument::RESERVED_NOT_ZERO,
            Self::Malformed => error::argument::MALFORMED_HEADER,
            Self::UnknownFlag | Self::Value | Self::NotStamped => error::argument::UNKNOWN_FLAG,
        };
        error::pack(error::ARGUMENT, code)
    }
}

/// Writes a payload's fields, counting as it goes.
///
/// Little-endian and by hand, [`store`](crate::store)'s discipline: the layout
/// is the encoding rather than the compiler's opinion of a struct, so nothing
/// here depends on the host's word size, its alignment rules or its byte order.
/// Every slot the writer does not reach stays zero, which is what makes a
/// record's tail the same bytes on both sides of the wire — and on both
/// architectures, which is the clause this file is accepted on.
struct Writer<'a> {
    /// The payload being filled.
    out: &'a mut [u8; PAYLOAD_BYTES],
    /// How much of it has been written.
    at: usize,
}

impl Writer<'_> {
    /// Append two bytes.
    fn u16(&mut self, value: u16) {
        self.put(&value.to_le_bytes());
    }

    /// Append two bytes of a signed quantity, two's complement.
    fn i16(&mut self, value: i16) {
        self.put(&value.to_le_bytes());
    }

    /// Append four bytes.
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    /// Append four bytes of a signed quantity, two's complement.
    fn i32(&mut self, value: i32) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes.
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    /// The one place bytes land.
    ///
    /// Indexing rather than a checked write, and the bound is a compile-time
    /// fact rather than a runtime hope: `events!` emits
    /// `STAMP_BYTES + WIDTH <= PAYLOAD_BYTES` for every record it declares, and
    /// `the_width_each_record_declares_is_the_width_it_writes` requires the
    /// writer to stop at `WIDTH` for every record there is. A record wide
    /// enough to overflow this cannot reach a build.
    fn put(&mut self, bytes: &[u8]) {
        self.out[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }
}

/// Reads a payload's fields, counting as it goes.
///
/// The counting is the point. [`Reader::finish`] refuses every byte the
/// decoder did not consume, so *unread* is a property of what the decoder
/// actually did rather than of a width somebody wrote down beside it. A field
/// dropped from a decoder does not become a field that is ignored; it becomes a
/// field in the tail, and the tail is refused.
struct Reader<'a> {
    /// The payload being read.
    raw: &'a [u8; PAYLOAD_BYTES],
    /// How much of it has been consumed.
    at: usize,
}

impl Reader<'_> {
    /// Take two bytes.
    fn u16(&mut self) -> u16 {
        let at = self.take(2);
        u16::from_le_bytes([self.raw[at], self.raw[at + 1]])
    }

    /// Take two bytes of a signed quantity, two's complement.
    fn i16(&mut self) -> i16 {
        self.u16().cast_signed()
    }

    /// Take four bytes.
    fn u32(&mut self) -> u32 {
        let at = self.take(4);
        u32::from_le_bytes([self.raw[at], self.raw[at + 1], self.raw[at + 2], self.raw[at + 3]])
    }

    /// Take four bytes of a signed quantity, two's complement.
    fn i32(&mut self) -> i32 {
        self.u32().cast_signed()
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

    /// Advance, and answer where the caller's bytes start. Bounded by the same
    /// compile-time assertion [`Writer::put`] is.
    fn take(&mut self, bytes: usize) -> usize {
        let at = self.at;
        self.at += bytes;
        at
    }

    /// Every byte this payload did not read must be zero.
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
/// Private, because nothing outside this module should encode a record without
/// the stamp and the envelope that frame it — [`Event`] is the door. Its value
/// is what it makes obligatory: a record cannot exist without a width, a
/// specimen, a writer and a reader, so a seventh opcode cannot be added without
/// all four, and [`Entry::SPECIMENS`] picks the specimen up without anybody
/// remembering to add it to a test.
trait Record: Copy + Sized {
    /// The bytes this record's own fields occupy, after the stamp and before
    /// the padding to [`PAYLOAD_BYTES`].
    /// Unit: bytes.
    const WIDTH: usize;

    /// One value of this record that is legal on the wire.
    ///
    /// Not a default and not an empty value: every closed field holds something
    /// inside its set, so that decoding it succeeds and every test below has
    /// something to work on. A field added to a record without a value here
    /// does not compile.
    const SPECIMEN: Self;

    /// Write the fields, in order, after the stamp the caller has written.
    fn write(&self, out: &mut Writer);

    /// Read the fields, in the same order, refusing before any of them is
    /// handed back.
    ///
    /// # Errors
    ///
    /// A [`Refusal`] naming the disbelief. Neither the stamp nor the tail is
    /// this method's business: [`Event::decode`] holds both rules for every
    /// record there is.
    fn read(raw: &mut Reader) -> Result<Self, Refusal>;
}

/// Names an opcode constant where a pattern is expected.
///
/// `macro_rules!` will not take `op::$constant` as a pattern directly — a path
/// followed by a metavariable is ambiguous there — and this is the indirection
/// that resolves it. It exists for no other reason.
macro_rules! opcode_pattern {
    ($constant:ident) => {
        op::$constant
    };
}

/// The six opcodes, written once.
///
/// # Why a macro, in a tree that mostly refuses them
///
/// Because the alternative is four sequences that have to agree: the opcode
/// constants, the list `known` matches against, the [`Entry`] variants and the
/// specimens every test here iterates. `interface/src/node.rs`'s `vocabulary!`
/// was written after two adversarial reviews found a role that was in an enum
/// and not in an array, and the second review found the hole still open under
/// the test that was supposed to have closed it — because a loop over a list
/// cannot see what the list omits, and an exhaustive match demands an arm
/// rather than an arm that says anything.
///
/// The clauses this file is accepted on are *fixed-width on both
/// architectures* and *unread fields refused when non-zero*, and both are
/// asserted by looping over a corpus of one specimen per opcode. A hand-written
/// corpus is precisely how those sentences stop being true while every test
/// stays green: a seventh opcode is added, it is not in the table, and the loop
/// keeps passing. One list closes it. Everything the rest of this module asks
/// of an opcode — its number, its record, its variant and a legal value of it —
/// is on the line that declares it.
///
/// The cost is the indirection between a reader and the enum, which is why
/// nothing else in this file is written this way. It is paid here because here
/// the second copy was the defect.
macro_rules! events {
    (
        $(
            $(#[$about:meta])*
            $variant:ident / $record:ident / $constant:ident = $opcode:literal;
        )*
    ) => {
        /// The input service's opcode space.
        ///
        /// Per-service and not global: section 05. These numbers mean nothing
        /// on the compositor's scene ring or the control ring, and comparing an
        /// opcode across two spaces is the mistake a single global enumeration
        /// would invite.
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

            /// Is this an opcode this build implements?
            ///
            /// R04: the negative answer is what turns an unknown opcode into a
            /// refusal instead of a silently skipped entry.
            #[must_use]
            pub const fn known(opcode: u8) -> bool {
                matches!(opcode, $($constant)|*)
            }

            /// A word for a log or a trace.
            #[must_use]
            pub const fn label(opcode: u8) -> &'static str {
                match opcode {
                    $($constant => stringify!($variant),)*
                    _ => "unknown",
                }
            }
        }

        // Every record fits the one payload width behind the stamp, asserted
        // from the list that declares the records rather than from a list
        // beside it. `Writer::put` indexes on the strength of this, so a
        // seventh record that did not fit would be a panic in a driver's
        // interrupt path; emitting the bound means there is no seventh record
        // that can exist without it. The hand-written version of this list was
        // deleted rather than kept beside it: a bound a new record can simply
        // not appear in is not a bound.
        $(
            const _: () = assert!(STAMP_BYTES + $record::WIDTH <= PAYLOAD_BYTES);
        )*

        /// The widest record's own fields, over every record there is.
        ///
        /// Computed from the list rather than named, because naming the widest
        /// record is a fact that stops being true when somebody adds a wider
        /// one — quietly, since the assertion would still pass about the record
        /// it names.
        /// Unit: bytes.
        const WIDEST_RECORD_BYTES: usize = {
            let widths = [$($record::WIDTH),*];
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

        // And the stride is the stamp plus that, rounded to eight. `PAYLOAD_BYTES`
        // stays a number written down rather than one derived from this, because
        // it is the wire's stride: a stride that recomputed itself when a record
        // grew would change what every peer must agree on without anybody typing
        // the new number. This is what makes changing it deliberate — a record
        // that grows past the stride, and a record that shrinks and leaves the
        // stride eight bytes wider than anything uses, are both a build failure
        // here rather than a comment that has gone stale.
        const _: () =
            assert!(PAYLOAD_BYTES == (STAMP_BYTES + WIDEST_RECORD_BYTES).next_multiple_of(8));

        /// What an event says, once its payload has been believed.
        ///
        /// One variant per opcode, emitted from the same list, so the set of
        /// things an event can be is the set of opcodes there are.
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
            /// seventh opcode is covered by the tests that already exist, on
            /// the day it is declared, without anybody remembering.
            /// Unit: none — one entry per opcode, in declaration order.
            pub const SPECIMENS: [Self; op::COUNT] = [$(Self::$variant($record::SPECIMEN)),*];

            /// The opcode that names this entry.
            #[must_use]
            pub const fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant(_) => op::$constant,)*
                }
            }

            /// The bytes this record's own fields occupy, after the stamp.
            /// Unit: bytes.
            #[must_use]
            pub const fn width(&self) -> usize {
                match self {
                    $(Self::$variant(_) => $record::WIDTH,)*
                }
            }

            /// Write the record's fields, after the stamp the caller wrote.
            fn write(&self, out: &mut Writer) {
                match self {
                    $(Self::$variant(record) => record.write(out),)*
                }
            }

            /// The entry an opcode and a reader positioned past the stamp name.
            fn read(opcode: u8, raw: &mut Reader) -> Result<Self, Refusal> {
                match opcode {
                    $(
                        opcode_pattern!($constant) =>
                            Ok(Self::$variant($record::read(raw)?)),
                    )*
                    _ => Err(Refusal::UnknownOpcode),
                }
            }
        }
    };
}

events! {
    /// Where the pointer is now.
    ///
    /// A position and not a displacement, which is the decision this opcode is
    /// really about; [`PointerMotion`] argues it.
    PointerMotion / PointerMotion / POINTER_MOTION = 0x01;

    /// A pointer button went down or came up.
    PointerButton / PointerButton / POINTER_BUTTON = 0x02;

    /// A wheel turned or a finger dragged on a scrolling surface.
    Scroll / Scroll / SCROLL = 0x03;

    /// A key went down or came up.
    Key / Key / KEY = 0x04;

    /// A contact appeared, moved, lifted or was withdrawn.
    TouchPoint / TouchPoint / TOUCH_POINT = 0x05;

    /// A stylus reported a position, a pressure and a tilt.
    ///
    /// Its own opcode rather than a pointer with extra fields, for the reason
    /// [`scene::SetPaint`](crate::scene::SetPaint) refuses a sixth field: a
    /// field that is meaningful only under some values of another is a record
    /// whose width is a function of its content wearing a fixed stride.
    StylusPoint / StylusPoint / STYLUS_POINT = 0x06;
}

/// Where the pointer is now.
///
/// # Why a position and not a displacement
///
/// A mouse reports displacement; this carries position, because the stage that
/// accumulates displacement into position must be the stage that holds the
/// device, and there is exactly one of those. If the wire carried deltas, every
/// consumer would keep its own accumulator, the compositor's and the
/// application's would start from different origins, and a single event dropped
/// on a full ring would move the pointer apart on the two sides **permanently**
/// rather than for one frame. Section 09's late-latch then re-reads a position
/// that is wrong by the sum of everything ever lost.
///
/// It is also what makes late-latch cheap: the newest [`PointerMotion`] in the
/// queue is the answer, and the compositor does not have to replay the ones
/// behind it. *What would reverse this:* pointer-lock and camera control, which
/// genuinely want raw displacement with no origin at all — a different quantity,
/// so a different opcode and an RFC, not a wider field here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerMotion {
    /// Position along x.
    ///
    /// 16.16 fixed point in an `i32` rather than 16.16 in an `i64`, because a
    /// coordinate is not a product: nothing multiplies two positions together,
    /// and the consumer that composes this with a transform widens it there,
    /// where the composition happens and the overflow would be. Signed, because
    /// a pointer that has left the surface has a position outside it and
    /// clamping on the wire would lose the direction it left in.
    /// Unit: device pixels from the surface origin, scaled by 65 536. Zero is
    /// the origin, which is a real place the pointer can be.
    pub x_x65536: i32,
    /// Position along y, on [`PointerMotion::x_x65536`]'s terms.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub y_x65536: i32,
}

impl Record for PointerMotion {
    const WIDTH: usize = 4 + 4;
    const SPECIMEN: Self = Self { x_x65536: 41_943_040, y_x65536: 23_592_960 };

    fn write(&self, out: &mut Writer) {
        out.i32(self.x_x65536);
        out.i32(self.y_x65536);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        // Every bit pattern of both fields is a position. There is nothing to
        // refuse here, and inventing a plausibility check — a coordinate
        // "outside the display" — would be this format deciding where surfaces
        // are, which it does not know.
        Ok(Self { x_x65536: raw.i32(), y_x65536: raw.i32() })
    }
}

/// A pointer button went down or came up.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PointerButton {
    /// Which button.
    ///
    /// The device's own code, as the driver reports it, and opaque here. This
    /// format does not enumerate buttons: a mouse with nine of them is not a
    /// wire-format change, and a fixed enumeration would make it one. What the
    /// numbers mean is the device's affair and the consumer's, and the two
    /// already have to agree about it — `E3-B04d` is where a real device's
    /// codes arrive.
    /// Unit: none — a button identifier, not a quantity. Zero is a legal
    /// identifier: the zeroed-payload hazard is closed by the stamp and by
    /// [`PointerButton::transition`], both of which refuse zero.
    pub button: u32,
    /// Which way it went.
    /// Unit: none — an [`edge`] constant. Zero is neither.
    pub transition: u16,
}

impl Record for PointerButton {
    const WIDTH: usize = 4 + 2;
    const SPECIMEN: Self = Self { button: 1, transition: edge::PRESSED };

    fn write(&self, out: &mut Writer) {
        out.u32(self.button);
        out.u16(self.transition);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let button = raw.u32();
        let transition = raw.u16();
        if !edge::known(transition) {
            return Err(Refusal::Value);
        }
        Ok(Self { button, transition })
    }
}

/// A wheel turned or a finger dragged on a scrolling surface.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Scroll {
    /// How far, along x. Positive is the direction the content moves, not the
    /// direction the finger moved: two conventions exist, they differ by a sign,
    /// and a format that did not pick one would make every consumer guess.
    /// Unit: device pixels, scaled by 65 536. Zero is no movement on this axis,
    /// which is the usual value on a wheel.
    pub dx_x65536: i32,
    /// How far, along y, on [`Scroll::dx_x65536`]'s terms.
    /// Unit: device pixels, scaled by 65 536.
    pub dy_x65536: i32,
    /// What produced it.
    ///
    /// The distance is in pixels whichever the source is, because a consumer
    /// that had to convert detents to distance would need the device's detent
    /// resolution, which only the driver has. The source is carried so the
    /// consumer can snap or glide — see [`axis_source::known`] — not so it can
    /// do the conversion the driver already did.
    /// Unit: none — an [`axis_source`] constant. Zero is neither.
    pub source: u16,
}

impl Record for Scroll {
    const WIDTH: usize = 4 + 4 + 2;
    const SPECIMEN: Self = Self { dx_x65536: 0, dy_x65536: -983_040, source: axis_source::WHEEL };

    fn write(&self, out: &mut Writer) {
        out.i32(self.dx_x65536);
        out.i32(self.dy_x65536);
        out.u16(self.source);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let dx_x65536 = raw.i32();
        let dy_x65536 = raw.i32();
        let source = raw.u16();
        if !axis_source::known(source) {
            return Err(Refusal::Value);
        }
        Ok(Self { dx_x65536, dy_x65536, source })
    }
}

/// A key went down or came up.
///
/// A key and not a character. What this carries is which key on the keyboard
/// moved; which character that produces is a function of the layout, the
/// modifier state and the composition sequence in progress, all of which live
/// in a stage that can be reconfigured while the machine is running — and a
/// driver that resolved characters here would bake the layout into the wire at
/// interrupt time, where it cannot be changed and cannot be seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// Which key.
    ///
    /// The device's own code, as the driver reports it, and opaque here for
    /// [`PointerButton::button`]'s reason.
    /// Unit: none — a key identifier, not a quantity. Zero is a legal
    /// identifier.
    pub code: u32,
    /// Which way it went.
    /// Unit: none — an [`edge`] constant. Zero is neither.
    pub transition: u16,
}

impl Record for Key {
    const WIDTH: usize = 4 + 2;
    const SPECIMEN: Self = Self { code: 30, transition: edge::PRESSED };

    fn write(&self, out: &mut Writer) {
        out.u32(self.code);
        out.u16(self.transition);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let code = raw.u32();
        let transition = raw.u16();
        if !edge::known(transition) {
            return Err(Refusal::Value);
        }
        Ok(Self { code, transition })
    }
}

/// A contact appeared, moved, lifted or was withdrawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TouchPoint {
    /// Which contact.
    ///
    /// Distinguishes fingers on one device, which is the one identity question
    /// the channel cannot answer — see the module's *which device this came
    /// from*. The identifier is live from [`touch::DOWN`] to [`touch::UP`] or
    /// [`touch::CANCEL`] and may be reused afterwards, which is why a consumer
    /// must key on the phase and not only on the number.
    /// Unit: none — a contact identifier, not a quantity. Zero is a legal
    /// identifier: hardware numbers slots from zero, and the zeroed-payload
    /// hazard is closed by the stamp and by [`TouchPoint::phase`].
    pub contact: u32,
    /// Position along x, on [`PointerMotion::x_x65536`]'s terms.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub x_x65536: i32,
    /// Position along y, on [`PointerMotion::x_x65536`]'s terms.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub y_x65536: i32,
    /// Where this contact is in its life.
    /// Unit: none — a [`touch`] constant. Zero is no phase.
    pub phase: u16,
}

impl Record for TouchPoint {
    const WIDTH: usize = 4 + 4 + 4 + 2;
    const SPECIMEN: Self =
        Self { contact: 0x0A0B_0C0D, x_x65536: 41_943_040, y_x65536: -196_608, phase: touch::MOVE };

    fn write(&self, out: &mut Writer) {
        out.u32(self.contact);
        out.i32(self.x_x65536);
        out.i32(self.y_x65536);
        out.u16(self.phase);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let contact = raw.u32();
        let x_x65536 = raw.i32();
        let y_x65536 = raw.i32();
        let phase = raw.u16();
        if !touch::known(phase) {
            return Err(Refusal::Value);
        }
        Ok(Self { contact, x_x65536, y_x65536, phase })
    }
}

/// A stylus reported a position, a pressure and a tilt.
///
/// There is no tip-down transition: the tip is down when the pressure is above
/// zero and hovering when it is zero, and one field that is always read beats
/// two that can disagree about the same instant. *What would reverse this:* a
/// device whose tip switch is a separate sensor from its pressure ramp and
/// whose two disagree near the threshold, which is a real thing on cheap
/// digitisers — at which point the switch is a field of its own and the
/// question *which wins* has to be answered here rather than by each consumer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StylusPoint {
    /// Position along x, on [`PointerMotion::x_x65536`]'s terms.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub x_x65536: i32,
    /// Position along y, on [`PointerMotion::x_x65536`]'s terms.
    /// Unit: device pixels from the surface origin, scaled by 65 536.
    pub y_x65536: i32,
    /// How hard the tip is pressed.
    /// Unit: none — a fraction of the device's full scale, scaled so that
    /// 65 535 is full. Zero is not touching: the stylus is hovering, and a
    /// hovering stylus still reports a position, which is what a cursor
    /// preview is drawn from.
    pub pressure_x65535: u16,
    /// Tilt away from vertical, in the x direction.
    /// Unit: degrees, scaled by 100, signed. Zero is upright.
    pub tilt_x_degrees_x100: i16,
    /// Tilt away from vertical, in the y direction.
    /// Unit: degrees, scaled by 100, signed. Zero is upright.
    pub tilt_y_degrees_x100: i16,
}

impl Record for StylusPoint {
    const WIDTH: usize = 4 + 4 + 2 + 2 + 2;
    const SPECIMEN: Self = Self {
        x_x65536: 65_536,
        y_x65536: 131_072,
        pressure_x65535: 32_768,
        tilt_x_degrees_x100: -1_575,
        tilt_y_degrees_x100: 4_500,
    };

    fn write(&self, out: &mut Writer) {
        out.i32(self.x_x65536);
        out.i32(self.y_x65536);
        out.u16(self.pressure_x65535);
        out.i16(self.tilt_x_degrees_x100);
        out.i16(self.tilt_y_degrees_x100);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        // No closed field and nothing to refuse: every pressure is a pressure,
        // and a tilt past vertical is a device reporting what it measured
        // rather than a peer lying. A range check here would be this format
        // deciding what a digitiser can do.
        Ok(Self {
            x_x65536: raw.i32(),
            y_x65536: raw.i32(),
            pressure_x65535: raw.u16(),
            tilt_x_degrees_x100: raw.i16(),
            tilt_y_degrees_x100: raw.i16(),
        })
    }
}

/// One input event as it crosses: part I's envelope, the stamp, and the body
/// its opcode names.
///
/// Every field here is either read by this format or refused, and there is no
/// third category — which is the sentence the whole module is arranged to make
/// true. See *an unread field is refused* for how each half of it is enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Event {
    /// Returned verbatim in the completion, and opaque here.
    /// Unit: none — chosen by the submitter and never interpreted. Zero is a
    /// legal token; a driver that wants to match refusals chooses otherwise.
    pub user_data: u64,
    /// The scheduling class, and the depth its urgency has already crossed.
    ///
    /// Read on every opcode, because it is the ring's field and not this
    /// format's: the service that drains the ring orders by it, and
    /// [`deadline::inherit`](crate::deadline::inherit) is what decides what it
    /// means. Nothing here re-decides it, and nothing here may discard it.
    /// Unit: none — a `class` field: an ordinal in the low byte, a depth in the
    /// high one. Zero is `class::HARD` at depth zero.
    pub class: u16,
    /// Where this event's payload is.
    /// Unit: bytes from the first byte of the channel's inline arena, not from
    /// the start of the mapping. Zero is the first byte of the arena.
    pub payload_offset: u32,
    /// Submission flags.
    /// Unit: none — a bitmask, and a subset of [`FLAGS_ACCEPTED`]. Zero is no
    /// flags, which is always legal.
    pub flags: u8,
    /// When the event happened, on the clock of the `Env` the driver stamped it
    /// against.
    ///
    /// **The scale is in the name and not in this comment**, which is the
    /// clause this file is accepted on: the field is `stamp_nanos` wherever it
    /// is spelled, the accessor that produces it is `f_input::StampNanos::nanos`
    /// and the one that consumes it is `f_input::StampNanos::from_wire_nanos`,
    /// so a stage that read this as microseconds would have had to rename it to
    /// do so. A bare `t` or `time` here is the defect `E3-B04a` describes, one
    /// crate later.
    ///
    /// Taken once, in the driver's interrupt path, and never again — the whole
    /// of `input/src/stamp.rs`'s argument, of which this field is the wire half.
    /// A consumer that re-stamps on arrival is not adding noise to the latency
    /// measurement, it is subtracting its own queueing delay out of it.
    /// Unit: nanoseconds, monotonic, in this channel's epoch — the clock
    /// `f_env::Instant` reports, which is the only clock in the system with
    /// ordering authority, and the same epoch [`Sqe::deadline`] states. RFC
    /// 0009. [`NOT_STAMPED`] is zero and is refused.
    pub stamp_nanos: u64,
    /// What the device did.
    /// Unit: none — one opcode's record.
    pub body: Entry,
}

impl Event {
    /// The opcode this event submits under.
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        self.body.opcode()
    }

    /// How much of the payload this event's opcode reads: the stamp, then the
    /// record's own fields. Everything from here to [`PAYLOAD_BYTES`] must be
    /// zero, and [`Event::decode`] is what requires it.
    /// Unit: bytes.
    #[must_use]
    pub const fn payload_used(&self) -> usize {
        STAMP_BYTES + self.body.width()
    }

    /// Is this an event a peer would accept?
    ///
    /// The value-level half of [`Event::decode`]'s rules that does not need the
    /// payload's bytes, exposed so that a driver can refuse its own entry
    /// before submitting it rather than learning about it in a completion. It
    /// is the same code path, not a second statement of the same rules.
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownFlag`] for a flag outside [`FLAGS_ACCEPTED`];
    /// [`Refusal::Reserved`] for a deadline, which no input opcode reads;
    /// [`Refusal::NotStamped`] for [`NOT_STAMPED`].
    pub const fn check(&self) -> Result<(), Refusal> {
        // Spelled out rather than with `?`, which `Try` is not const enough
        // for. The order is `decode`'s order, so that a producer refusing its
        // own entry is told the same thing its consumer would have told it.
        if let Err(refusal) = envelope_rules(self.opcode(), self.flags, NO_DEADLINE) {
            return Err(refusal);
        }
        if self.stamp_nanos == NOT_STAMPED {
            return Err(Refusal::NotStamped);
        }
        Ok(())
    }

    /// The submission entry this event crosses in.
    ///
    /// Written field by field, with no `..Sqe::ZERO`, so that a field added to
    /// [`Sqe`] stops this build here and asks whether an input event reads it.
    /// [`Event::decode`] then compares an arriving entry against exactly this,
    /// which turns whatever is answered into something a peer cannot get wrong.
    #[must_use]
    pub const fn envelope(&self) -> Sqe {
        Sqe {
            opcode: self.opcode(),
            flags: self.flags,
            class: self.class,
            // An input event names no capability. Authority here is the ring's
            // — the consumer holds a channel to one device's driver, and what
            // that driver may report is what that device does — so a capability
            // index on an entry would be an authority nobody granted and a
            // field nobody reads.
            cap: 0,
            user_data: self.user_data,
            // Never. The module's *no input entry carries a deadline* is the
            // argument, and it is one line here rather than six because there
            // is then no line on which an opcode could claim otherwise.
            deadline: NO_DEADLINE,
            offset: self.payload_offset as u64,
            // The payload is in the arena, at `offset`. `FIXED_BUF` is outside
            // `FLAGS_ACCEPTED`, so these two never name a registered set.
            buf_set: 0,
            buf_index: 0,
            len: PAYLOAD_BYTES as u32,
            _reserved: 0,
            // Not a spare pair of words, and not where the stamp goes — the
            // module's *the stamp* is why. A field that belongs to an opcode
            // belongs in that opcode's record, where the record's own width
            // states it and `Reader::finish` polices it.
            ext: [0, 0],
        }
    }

    /// The payload as it crosses: the stamp, the record's fields, then zero to
    /// [`PAYLOAD_BYTES`].
    ///
    /// The stamp is written here, once, for every opcode there is — which is
    /// the same sentence [`Event::decode`] makes on the way back, and the
    /// reason a seventh opcode cannot be unstamped.
    #[must_use]
    pub fn payload(&self) -> [u8; PAYLOAD_BYTES] {
        let mut out = [0u8; PAYLOAD_BYTES];
        let mut writer = Writer { out: &mut out, at: 0 };
        writer.u64(self.stamp_nanos);
        self.body.write(&mut writer);
        out
    }

    /// Encode: the entry, and the payload it points at.
    ///
    /// Total, on purpose. It writes what it was given, including a value
    /// [`Event::decode`] would refuse — which is what lets a test build a
    /// malformed entry without a second encoder, and lets a hostile-peer
    /// harness produce the entries it exists to produce. A producer that wants
    /// the check calls [`Event::check`] first.
    #[must_use]
    pub fn encode(&self) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        (self.envelope(), self.payload())
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The order is the order a refusal is distinguishable in: the opcode
    /// first, because every later check depends on which opcode this is; the
    /// flags and the deadline next; the framing; the stamp, because an event
    /// with no place in the latency chain is not worth decoding the body of;
    /// then the body; then the tail; and the whole-envelope comparison last,
    /// because it is the catch-all nothing can be added past.
    ///
    /// # Errors
    ///
    /// A [`Refusal`]. [`Refusal::Reserved`] is the one worth naming: it is what
    /// an entry gets for any byte, anywhere, that this build does not read.
    pub fn decode(entry: &Sqe, payload: &[u8; PAYLOAD_BYTES]) -> Result<Self, Refusal> {
        envelope_rules(entry.opcode, entry.flags, entry.deadline)?;
        if entry.len as usize != PAYLOAD_BYTES {
            return Err(Refusal::Malformed);
        }
        // A channel mapping is a `u32` of bytes — `layout::MAX_ENTRIES` is
        // chosen so that every offset in one fits with room to spare — so an
        // offset past that is not an offset into any arena. Refused here rather
        // than truncated into the field below, because a truncation would make
        // the comparison that follows fail as *a reserved field is not zero*,
        // which is a true sentence about the wrong field.
        if entry.offset > u64::from(u32::MAX) {
            return Err(Refusal::Malformed);
        }

        // The stamp, before the dispatch and outside it. This is the one place
        // it is read, which is what makes *every input event is stamped* a
        // property of the format rather than of six decoders agreeing.
        let mut reader = Reader { raw: payload, at: 0 };
        let stamp_nanos = reader.u64();
        if stamp_nanos == NOT_STAMPED {
            return Err(Refusal::NotStamped);
        }
        let body = Entry::read(entry.opcode, &mut reader)?;
        reader.finish()?;

        let event = Self {
            user_data: entry.user_data,
            class: entry.class,
            payload_offset: entry.offset as u32,
            flags: entry.flags,
            stamp_nanos,
            body,
        };

        // The check this module exists for, on the envelope half. Not a list of
        // the fields an input event ignores — such a list is correct on the day
        // it is written and silent afterwards — but the entry this build would
        // have produced from the fields it just read, compared in full. Any
        // field `envelope` does not set, including one a later ABI adds, must
        // arrive zero.
        if sqe_bytes(&event.envelope()) != sqe_bytes(entry) {
            return Err(Refusal::Reserved);
        }
        Ok(event)
    }
}

/// The envelope rules that do not need the payload.
///
/// One function, called by [`Event::check`] before submitting and by
/// [`Event::decode`] after receiving, so that a producer and a consumer cannot
/// hold different opinions about which entries are legal.
const fn envelope_rules(opcode: u8, flags: u8, deadline: u64) -> Result<(), Refusal> {
    if !op::known(opcode) {
        return Err(Refusal::UnknownOpcode);
    }
    if flags & !FLAGS_ACCEPTED != 0 {
        return Err(Refusal::UnknownFlag);
    }
    // Flat, for every opcode. There is no `carries_deadline` to consult here
    // and deliberately no line on which a seventh opcode could claim one.
    if deadline != NO_DEADLINE {
        return Err(Refusal::Reserved);
    }
    Ok(())
}

/// The sixty-four bytes of a submission entry.
///
/// A byte view rather than a field-by-field comparison, and the difference is
/// the whole of [`Event::decode`]'s guarantee: a comparison written out by hand
/// covers the fields somebody listed, and this one covers the fields there are.
///
/// `scene` carries a private copy of this function and its two assertions, and
/// so will every entry format written this week. That duplication should be one
/// `crate::sqe_bytes` — four copies of an `unsafe` block is four places a
/// SAFETY argument can rot independently — but hoisting it is a diff to
/// `lib.rs`, which is not this file's to make.
fn sqe_bytes(entry: &Sqe) -> &[u8; SQE_BYTES] {
    // SAFETY: `Sqe` is `#[repr(C, align(64))]`, so its fields are laid out in
    // declaration order at fixed offsets; `size_of::<Sqe>()` is `SQE_BYTES` and
    // the assertion below this function shows that width to be the exact sum of
    // its field widths, so the type carries no padding and every one of its
    // bytes is an initialised byte of an integer. `[u8; SQE_BYTES]` has
    // alignment one, which `Sqe`'s sixty-four satisfies. The reference produced
    // borrows `entry` for its own lifetime and is shared, so nothing is mutated
    // through it and nothing outlives it.
    unsafe { &*core::ptr::from_ref(entry).cast::<[u8; SQE_BYTES]>() }
}

// The two facts `sqe_bytes` rests on. The first is also asserted in `lib.rs`,
// beside the type; it is asserted again here because this is the code that
// would be unsound without it, and an assertion in another file is a fact this
// one is trusting rather than stating.
const _: () = assert!(core::mem::size_of::<Sqe>() == SQE_BYTES);
// No padding: a `#[repr(C)]` type whose size equals the sum of its field widths
// has none anywhere. Written as the sum rather than as `64`, so that the
// arithmetic is in front of whoever changes a field.
const _: () = assert!(SQE_BYTES == 1 + 1 + 2 + 4 + 8 + 8 + 8 + 4 + 4 + 4 + 4 + 16);

// The stamp is a `u64` and is written as one. A stamp narrowed to 32 bits would
// wrap about four seconds into a channel's epoch, which is a bug that looks
// like the input path stalling rather than like a width being wrong.
const _: () = assert!(STAMP_BYTES == core::mem::size_of::<u64>());
// The flags this format accepts are flags the ABI defines. A bit removed from
// `flags::KNOWN` and left here would be a bit this build accepts on an entry
// and the ring's own envelope check refuses.
const _: () = assert!(FLAGS_ACCEPTED & !flags::KNOWN == 0);

/// Everything that crossed, folded into one word, so that the stage that sent
/// it and the stage that received it can say the same thing about it without
/// either one holding the other's copy.
///
/// # What [`Event::decode`] establishes, and the one thing it cannot
///
/// A decoded entry *arrived stamped*: a payload whose first eight bytes are
/// [`NOT_STAMPED`] is refused before the body is read at all, so a consumer
/// holding an [`Event`] is holding a reading somebody took. What no decoder can
/// establish, because it sees one entry and has nothing to compare it against,
/// is that the reading is **the one the driver took**. A stage in between that
/// re-stamped on arrival, handed the entries on in a different order, dropped
/// one out of the middle, or moved a coordinate by one produces entries that
/// every decoder accepts — and `input/src/stamp.rs` spends a page on what the
/// first of those four does to the number this path exists to publish: it does
/// not add noise to the latency, it subtracts the relay's own delay out of it,
/// in the direction that makes the figure improve as the system gets slower.
///
/// So the producer folds every entry it submits, the consumer folds every entry
/// it drains, and the two words are compared. Neither is computed from the
/// other and neither travels with the entries: the producer publishes its word
/// where it reports what it did, and the consumer builds its own out of what
/// arrived. What is shared is this implementation and not the instance, which
/// is the distinction `E3-B04c`'s seam draws and the reason this is evidence
/// rather than a tautology — a defect *inside* this fold moves both words the
/// same way and is caught by this module's own corpus, and a defect in a relay
/// moves one of them.
///
/// # Why the whole payload and not the stamp alone
///
/// Because a relay that preserved every stamp and rewrote a coordinate has
/// invented what the user did, which is the same class of defect wearing a
/// different field, and a fold that watched one field would have to be widened
/// by hand every time a record gained one. The stamp is the first
/// [`STAMP_BYTES`] of every payload for every opcode there is —
/// [`Event::payload`] writes it before it dispatches — so folding the payload
/// folds the stamp structurally, and a seventh opcode is watched on the day it
/// is declared for the same reason it is stamped on that day.
///
/// The cost is that a disagreement does not say *which* field moved. That is
/// worth stating rather than engineering around: the boot comparing these two
/// words prints both sides' counts beside them, and the question a reader then
/// asks — was it a stamp or a body — is answered by the corpus in this module
/// rather than by a second word on a routing page.
///
/// # What is folded, and what is deliberately left out
///
/// The opcode and the payload. **Not** [`Event::payload_offset`], which says
/// where in an arena the bytes were parked; **not** [`Event::user_data`] or
/// [`Event::flags`], which say what the submission wanted of the ring; and
/// **not** [`Event::class`], which the module above calls the ring's field and
/// not this format's. All four are properties of the carriage rather than of
/// the event, and a fold that included them would go red the first time an
/// entry is carried in a second channel — which is the crossing this
/// attestation exists to survive. What is folded is what the device did and
/// when, and nothing about how it was delivered.
///
/// *What would reverse this:* a relay entitled to rewrite what the device said
/// — a coalescer, or the router `E3-B04`'s parent owes, which may legitimately
/// hand on one motion where two arrived. Such a stage cannot carry this word
/// through and must publish its own, with the arithmetic relating the two
/// written down; the repair is a second attestation with a stated relation, not
/// a fold that stops watching the body.
///
/// # Why this is not `f-hash`
///
/// `f-hash` is SHA-256 and it is what *names* things — a blob, a release, a
/// generation — where a collision an adversary can choose is the whole risk.
/// This word names nothing and is looked up by nobody: it is a checksum across
/// one boot between two stages of one machine, and what it has to do is change
/// when any byte of the crossing changes. `abi` additionally has no
/// dependencies, which its own manifest calls a property rather than an
/// omission, so a digest here would be a fourth copy of one rather than a use
/// of the one there is.
///
/// *What would reverse this:* a peer that is not this machine's own driver, at
/// which point the adversary chooses the entries and a fold anybody can invert
/// stops being evidence. That is a content address, belongs in `f-hash`, is
/// taken over a transcript, and costs what `claims/` would have to measure —
/// not a wider constant here.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Crossing {
    /// The fold so far. Private, and that is the point: a word that could be
    /// assigned is a consumer that can agree with a producer it never listened
    /// to.
    /// Unit: none — a checksum.
    word: u64,
    /// How many entries have gone into it. Unit: entries.
    absorbed: u64,
}

/// Where a fold starts, before anything has crossed.
///
/// FNV-1a's offset basis, and the value matters in exactly one way: it is not
/// zero. A page nobody wrote reads as zero, so a fold that started there would
/// make *nothing crossed and nobody published* indistinguishable from *the
/// producer published a fold of nothing*.
/// Unit: none — a checksum.
const CROSSING_BASIS: u64 = 0xCBF2_9CE4_8422_2325;

/// The multiplier the fold mixes with. FNV-1a's 64-bit prime.
/// Unit: none.
const CROSSING_PRIME: u64 = 0x0000_0100_0000_01B3;

impl Crossing {
    /// A fold with nothing in it.
    #[must_use]
    pub const fn new() -> Self {
        Self { word: CROSSING_BASIS, absorbed: 0 }
    }

    /// Fold one entry in, in the order it crossed.
    ///
    /// Order-sensitive, because the byte stream is: two entries swapped are two
    /// different streams unless they are byte-identical, in which case they are
    /// two events nothing downstream could have told apart either.
    pub fn absorb(&mut self, event: &Event) {
        self.word = mixed(self.word, event.opcode());
        for byte in event.payload() {
            self.word = mixed(self.word, byte);
        }
        self.absorbed = self.absorbed.saturating_add(1);
    }

    /// The fold, for a routing page or a boot log.
    /// Unit: none — a checksum.
    #[must_use]
    pub const fn word(self) -> u64 {
        self.word
    }

    /// How many entries went into it. Unit: entries.
    #[must_use]
    pub const fn absorbed(self) -> u64 {
        self.absorbed
    }

    /// Does what crossed match what the other side says it sent?
    ///
    /// The emptiness guard is the clause worth reading. Two stages that each
    /// folded nothing hold [`CROSSING_BASIS`] and agree, which is a green
    /// answer about a crossing that never happened — the vacuity this workspace
    /// keeps finding one layer down from where it was looking. So a fold with
    /// nothing in it agrees with nobody, and a caller that genuinely means *no
    /// entries crossed* says so with a count rather than with this.
    #[must_use]
    pub const fn agrees_with(self, published: u64) -> bool {
        self.absorbed != 0 && self.word == published
    }
}

impl Default for Crossing {
    fn default() -> Self {
        Self::new()
    }
}

/// One byte into the fold.
fn mixed(word: u64, byte: u8) -> u64 {
    (word ^ u64::from(byte)).wrapping_mul(CROSSING_PRIME)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{class, deadline};

    /// The stamp every event in these tests carries.
    ///
    /// Distinctive in every byte and far enough from zero that no single byte
    /// flipped in it lands back on [`NOT_STAMPED`]. Not a measurement of
    /// anything and not a claim about a machine: it is a bit pattern chosen so
    /// that the byte-order assertion below is readable.
    const STAMPED_AT: u64 = 0x0000_00A1_B2C3_D4E5;

    /// A deadline far enough from zero that no single byte flipped in it lands
    /// back on [`NO_DEADLINE`]. No input entry may carry one; this is what is
    /// used to try.
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// An event around a body, with an envelope that is legal for it.
    ///
    /// Legal for every opcode without a per-opcode branch, because the envelope
    /// rules here do not vary by opcode — which is the point of the module's
    /// *no input entry carries a deadline*, restated as a test helper that
    /// could not have been written if they did.
    fn event(body: Entry) -> Event {
        Event {
            user_data: 0x0102_0304_0506_0708,
            class: deadline::pack(class::SOFT, 1),
            payload_offset: 0x0E00,
            flags: flags::NO_CQE,
            stamp_nanos: STAMPED_AT,
            body,
        }
    }

    #[test]
    fn every_opcode_round_trips_through_its_bytes() {
        // The exit's first clause. It runs on x86-64 and, through `cargo xtask
        // test` on the arm runner, on AArch64 — which is what the encoding
        // being written by hand is for: every field goes out through
        // `to_le_bytes` and comes back through `from_le_bytes`, so nothing here
        // can inherit the host's word size, its alignment or its byte order.
        //
        // The corpus is `Entry::SPECIMENS`, emitted from the same list as the
        // opcodes, so a seventh opcode is in it the day it is declared.
        assert_eq!(Entry::SPECIMENS.len(), op::COUNT);
        for body in Entry::SPECIMENS {
            let original = event(body);
            let label = op::label(original.opcode());
            assert_eq!(original.check(), Ok(()), "{label}");
            let (entry, payload) = original.encode();
            assert_eq!(entry.len as usize, PAYLOAD_BYTES, "{label}");
            let back = Event::decode(&entry, &payload)
                .unwrap_or_else(|refusal| panic!("{label}: {}", refusal.message()));
            assert_eq!(back, original, "{label}");
            assert_eq!(back.stamp_nanos, STAMPED_AT, "{label}");

            // And the encoding is a function of the value alone: the same event
            // encodes to the same bytes, which is what makes two architectures
            // produce one wire image.
            let (again, again_payload) = back.encode();
            assert_eq!(sqe_bytes(&again), sqe_bytes(&entry), "{label}");
            assert_eq!(again_payload, payload, "{label}");
        }
    }

    #[test]
    fn the_bytes_are_the_ones_written_down() {
        // The exit's *fixed-width on both architectures*, as a real byte-layout
        // assertion rather than a `size_of`: every byte of one event, in the
        // position it occupies, with a negative fixed-point coordinate whose
        // sign extension and byte order are both visible. This is the test the
        // AArch64 cross-check runs: a `to_ne_bytes` that slipped into the
        // writer would pass on a little-endian host and be caught here the
        // moment either architecture disagreed, and an encoding that had
        // borrowed the host's alignment or word size would put these bytes in
        // different places.
        let original = event(Entry::TouchPoint(TouchPoint::SPECIMEN));
        let (entry, payload) = original.encode();

        #[rustfmt::skip]
        let expected_entry: [u8; SQE_BYTES] = [
            // opcode, flags, class (SOFT at depth one).
            0x05, 0x08, 0x01, 0x01,
            // cap: an input event names no capability.
            0x00, 0x00, 0x00, 0x00,
            // user_data, little-endian.
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
            // deadline: no input entry carries one.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // offset: 0x0E00 bytes into the arena.
            0x00, 0x0E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // buf_set, buf_index: the payload is in the arena.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // len: one payload slot, 24 bytes.
            0x18, 0x00, 0x00, 0x00,
            // _reserved.
            0x00, 0x00, 0x00, 0x00,
            // ext: not where the stamp lives.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(sqe_bytes(&entry), &expected_entry);

        #[rustfmt::skip]
        let expected_payload: [u8; PAYLOAD_BYTES] = [
            // stamp_nanos, the shared header every opcode carries.
            0xE5, 0xD4, 0xC3, 0xB2, 0xA1, 0x00, 0x00, 0x00,
            // contact = 0x0A0B_0C0D.
            0x0D, 0x0C, 0x0B, 0x0A,
            // x = 41 943 040, which is 640.0 in 16.16.
            0x00, 0x00, 0x80, 0x02,
            // y = -196 608, which is -3.0: two's complement, sign-extended
            // across four bytes and not eight.
            0x00, 0x00, 0xFD, 0xFF,
            // phase = MOVE.
            0x02, 0x00,
            // The padding to the stride, which decoding requires to be zero.
            0x00, 0x00,
        ];
        assert_eq!(payload, expected_payload);
        assert_eq!(original.payload_used(), PAYLOAD_BYTES - 2);
    }

    /// Flip one byte of an entry, in place.
    fn flip(entry: &mut Sqe, at: usize) {
        // SAFETY: the same two facts `sqe_bytes` rests on, and the assertions
        // below it establish — `Sqe` is `#[repr(C)]`, `SQE_BYTES` wide, and
        // that width is the exact sum of its field widths, so it has no padding
        // and every byte of it is an initialised byte of an integer that any
        // bit pattern is a value of. The slice borrows `entry` mutably and
        // exclusively for the length of the statement and is not held across
        // anything.
        let bytes = unsafe {
            core::slice::from_raw_parts_mut(core::ptr::from_mut(entry).cast::<u8>(), SQE_BYTES)
        };
        bytes[at] ^= 0xFF;
    }

    #[test]
    fn nothing_in_an_entry_is_ignored() {
        // The exit's second clause, stated as strongly as it can be: there is
        // no byte of an event — envelope or payload — that can be changed
        // without either changing what the event means or being refused. A
        // field this format read and then dropped would show up here as a byte
        // that can be flipped and still decode to the value it started as.
        //
        // The corpus is `Entry::SPECIMENS` and the positions are every position
        // there is, so neither is a list a seventh opcode or a seventh field
        // could be left out of.
        for body in Entry::SPECIMENS {
            let original = event(body);
            let (entry, payload) = original.encode();
            let label = op::label(original.opcode());

            for at in 0..SQE_BYTES {
                let mut damaged = entry;
                flip(&mut damaged, at);
                assert_ne!(
                    Event::decode(&damaged, &payload),
                    Ok(original),
                    "{label}: byte {at} of the envelope is ignored"
                );
            }

            for at in 0..PAYLOAD_BYTES {
                let mut damaged = payload;
                damaged[at] ^= 0xFF;
                assert_ne!(
                    Event::decode(&entry, &damaged),
                    Ok(original),
                    "{label}: byte {at} of the payload is ignored"
                );
            }
        }
    }

    #[test]
    fn a_byte_past_a_record_is_refused_and_not_dropped() {
        // The same clause where it is easiest to get wrong, with the refusal
        // named: the padding between a narrow record and the stride is exactly
        // where a future field would go, and a decoder that skipped it would
        // accept an entry written by a peer that has that field — which is the
        // whole of R04 for a format whose peer is a driver built from imported
        // source.
        for body in Entry::SPECIMENS {
            let original = event(body);
            let (entry, payload) = original.encode();
            let label = op::label(original.opcode());
            for at in original.payload_used()..PAYLOAD_BYTES {
                let mut damaged = payload;
                damaged[at] = 1;
                assert_eq!(
                    Event::decode(&entry, &damaged),
                    Err(Refusal::Reserved),
                    "{label}: byte {at} past the record is not refused"
                );
            }
        }
    }

    #[test]
    fn the_width_each_record_declares_is_the_width_it_writes() {
        // `WIDTH` is what `events!` bounds against `PAYLOAD_BYTES`, and
        // `Writer::put` indexes on the strength of that bound. A
        // record whose writer disagreed with its declared width would make that
        // bound a statement about the wrong number, so the two are required to
        // agree here — over every record there is, because the corpus is the
        // emitted one.
        for body in Entry::SPECIMENS {
            let original = event(body);
            let payload = original.payload();
            assert!(original.payload_used() <= PAYLOAD_BYTES);
            // Everything past the declared width is untouched by the writer,
            // which is the same statement as *the writer stopped at `width`*
            // for a buffer that started zeroed.
            assert!(
                payload[original.payload_used()..].iter().all(|byte| *byte == 0),
                "{}: the writer went past its declared width",
                op::label(body.opcode())
            );
        }
    }

    #[test]
    fn the_stamp_is_the_first_field_of_every_payload_and_is_read_back_whole() {
        // The exit's third clause, on the wire rather than in a name: the field
        // `f_input::StampNanos::from_wire_nanos` rebuilds a stamp from is these
        // eight bytes, in this position, for every opcode there is — including
        // one added after this test was written, because the corpus is emitted.
        // A stamp that survived the wire changed is a latency computed for a
        // different event.
        for body in Entry::SPECIMENS {
            let original = event(body);
            let label = op::label(original.opcode());
            let payload = original.payload();
            assert_eq!(&payload[..STAMP_BYTES], &STAMPED_AT.to_le_bytes(), "{label}");
            let (entry, payload) = original.encode();
            let back = Event::decode(&entry, &payload).expect("a stamped event decodes");
            assert_eq!(back.stamp_nanos, STAMPED_AT, "{label}");
        }
    }

    #[test]
    fn an_event_that_was_never_stamped_is_refused() {
        // And so a zeroed slot of a fresh mapping is not a pointer at the
        // origin at the beginning of time. `PointerMotion` is the case that
        // needs this: it has no closed field, so the stamp is the only thing
        // standing between a page of zeroes and a position the compositor would
        // believe.
        for body in Entry::SPECIMENS {
            let mut unstamped = event(body);
            unstamped.stamp_nanos = NOT_STAMPED;
            let label = op::label(unstamped.opcode());
            assert_eq!(unstamped.check(), Err(Refusal::NotStamped), "{label}");
            let (entry, payload) = unstamped.encode();
            assert_eq!(
                Event::decode(&entry, &payload),
                Err(Refusal::NotStamped),
                "{label}: an unstamped event was believed"
            );
        }

        // The whole-page case, which is the one that actually happens.
        let zeroed = event(Entry::PointerMotion(PointerMotion::SPECIMEN)).envelope();
        assert_eq!(Event::decode(&zeroed, &[0; PAYLOAD_BYTES]), Err(Refusal::NotStamped));
    }

    #[test]
    fn no_input_entry_carries_a_deadline() {
        // Flat, for every opcode, with no exception to find. An entry that
        // carried one would be an entry whose unread field a consumer might
        // act on — and the deadline that matters to this path belongs to the
        // frame that will show the event, which has not been scheduled yet.
        for body in Entry::SPECIMENS {
            let original = event(body);
            let label = op::label(original.opcode());
            let (mut entry, payload) = original.encode();
            assert_eq!(entry.deadline, NO_DEADLINE, "{label}");
            entry.deadline = SCHEDULED_AT;
            assert_eq!(
                Event::decode(&entry, &payload),
                Err(Refusal::Reserved),
                "{label} was allowed a deadline"
            );
        }
    }

    #[test]
    fn an_opcode_this_build_does_not_know_is_refused() {
        // R04, and the case that matters most: a zeroed slot of a fresh mapping
        // is not opcode zero doing nothing, it is an entry nobody wrote.
        assert_eq!(Event::decode(&Sqe::ZERO, &[0; PAYLOAD_BYTES]), Err(Refusal::UnknownOpcode));

        assert!(!op::known(0));
        assert_eq!(op::label(0), "unknown");
        for opcode in 0..=u8::MAX {
            assert_eq!(op::known(opcode), op::ALL.contains(&opcode));
        }

        let mut foreign = event(Entry::Key(Key::SPECIMEN)).envelope();
        foreign.opcode = 0xFE;
        assert_eq!(
            Event::decode(&foreign, &[0; PAYLOAD_BYTES]),
            Err(Refusal::UnknownOpcode),
            "the top of the space is RFC 0028's, not this service's"
        );
    }

    #[test]
    fn a_flag_an_event_may_not_carry_is_refused() {
        // Refused and not masked: a driver that believed its events were linked
        // and a consumer that dropped the bit would disagree about the order
        // the user did things in, and neither would ever find out.
        for bit in [flags::LINK, flags::DRAIN, flags::FIXED_BUF] {
            let mut linked = event(Entry::Key(Key::SPECIMEN));
            linked.flags = bit;
            assert_eq!(linked.check(), Err(Refusal::UnknownFlag));
            let (entry, payload) = linked.encode();
            assert_eq!(Event::decode(&entry, &payload), Err(Refusal::UnknownFlag));
        }
        // And a bit the ABI itself does not define.
        let mut invented = event(Entry::Key(Key::SPECIMEN));
        invented.flags = 1 << 7;
        assert_eq!(invented.check(), Err(Refusal::UnknownFlag));
    }

    #[test]
    fn an_envelope_field_this_format_never_reads_is_refused() {
        // The demonstration of the comparison in `decode`. It is a
        // demonstration and not the guard: the guard is that the canonical
        // entry has these zero, so a field added to `Sqe` is covered by a check
        // nobody has to remember to extend — including on the day this list
        // stops naming every such field.
        let original = event(Entry::StylusPoint(StylusPoint::SPECIMEN));
        let (entry, payload) = original.encode();

        for spoil in [
            |e: &mut Sqe| e.cap = 1,
            |e: &mut Sqe| e.buf_set = 1,
            |e: &mut Sqe| e.buf_index = 1,
            |e: &mut Sqe| e._reserved = 1,
            |e: &mut Sqe| e.ext[0] = 1,
            |e: &mut Sqe| e.ext[1] = 1,
        ] {
            let mut damaged = entry;
            spoil(&mut damaged);
            assert_eq!(Event::decode(&damaged, &payload), Err(Refusal::Reserved));
        }

        // The framing, which is a different disbelief and gets a different
        // refusal: this entry does not frame a payload at all.
        let mut short = entry;
        short.len = PAYLOAD_BYTES as u32 - 1;
        assert_eq!(Event::decode(&short, &payload), Err(Refusal::Malformed));

        let mut far = entry;
        far.offset = u64::from(u32::MAX) + 1;
        assert_eq!(Event::decode(&far, &payload), Err(Refusal::Malformed));
    }

    #[test]
    fn a_closed_field_outside_its_set_is_refused() {
        // A transition, a touch phase and a scroll source are the closed fields
        // in this format, and all are refused rather than clamped to something
        // plausible: a fifth touch phase arriving from a driver built later
        // than this one is a gesture this build cannot interpret, and
        // interpreting it as something else is how a stolen gesture becomes a
        // stray tap.
        let neither = PointerButton { transition: 0, ..PointerButton::SPECIMEN };
        let (entry, payload) = event(Entry::PointerButton(neither)).encode();
        assert_eq!(Event::decode(&entry, &payload), Err(Refusal::Value));

        let fifth = TouchPoint { phase: touch::CANCEL + 1, ..TouchPoint::SPECIMEN };
        let (entry, payload) = event(Entry::TouchPoint(fifth)).encode();
        assert_eq!(Event::decode(&entry, &payload), Err(Refusal::Value));

        let sourceless = Scroll { source: 0, ..Scroll::SPECIMEN };
        let (entry, payload) = event(Entry::Scroll(sourceless)).encode();
        assert_eq!(Event::decode(&entry, &payload), Err(Refusal::Value));

        let third = Key { transition: edge::RELEASED + 1, ..Key::SPECIMEN };
        let (entry, payload) = event(Entry::Key(third)).encode();
        assert_eq!(Event::decode(&entry, &payload), Err(Refusal::Value));
    }

    #[test]
    fn every_refusal_is_an_argument_error_a_completion_can_carry() {
        // RFC 0010's shape: a negative result in a domain a caller acts on,
        // with a code that already means what it says. Nothing here invents a
        // code — see the module's *why the refusals are local* — and this is
        // what would fail if one were added without a domain to put it in.
        for refusal in [
            Refusal::UnknownOpcode,
            Refusal::UnknownFlag,
            Refusal::Reserved,
            Refusal::Malformed,
            Refusal::Value,
            Refusal::NotStamped,
        ] {
            let packed = refusal.packed();
            assert!(packed < 0, "{}", refusal.message());
            let (domain, _) = error::unpack(packed).expect("a refusal is an error");
            assert_eq!(domain, error::ARGUMENT, "{}", refusal.message());
            assert!(!refusal.message().is_empty());
        }
    }

    /// A crossing of the whole corpus, in the order the specimens are declared.
    ///
    /// The producer's side of every test below. One specimen per opcode, so a
    /// seventh opcode is in the crossing the day it is declared rather than the
    /// day somebody remembers to add it.
    fn crossed() -> Crossing {
        let mut fold = Crossing::new();
        for body in Entry::SPECIMENS {
            fold.absorb(&event(body));
        }
        fold
    }

    #[test]
    fn what_was_sent_and_what_arrived_agree_only_when_they_are_the_same_crossing() {
        // The consumer's side is built out of the bytes, through `decode`,
        // rather than out of the events the producer holds — which is what a
        // consumer actually has and is the only version of this test that says
        // anything. A fold over the producer's own values on both sides would
        // agree on a wire format that lost a field.
        let sent = crossed();
        let mut arrived = Crossing::new();
        for body in Entry::SPECIMENS {
            let (entry, payload) = event(body).encode();
            let back = Event::decode(&entry, &payload).expect("a specimen decodes");
            arrived.absorb(&back);
        }
        assert!(arrived.agrees_with(sent.word()));
        assert_eq!(arrived.absorbed(), sent.absorbed());
        assert_eq!(arrived.absorbed(), op::COUNT as u64);
    }

    #[test]
    fn a_stamp_minted_on_arrival_is_what_this_fold_is_for() {
        // The defect `input/src/stamp.rs` is written against, as a relay: every
        // entry still decodes, every entry is still stamped, and the number the
        // path publishes afterwards is the consumer's queueing delay subtracted
        // out of the latency. One entry re-stamped moves the word.
        let sent = crossed();
        let mut arrived = Crossing::new();
        for (index, body) in Entry::SPECIMENS.into_iter().enumerate() {
            let mut one = event(body);
            if index == 2 {
                one.stamp_nanos = STAMPED_AT + 1;
                assert_eq!(one.check(), Ok(()), "a re-stamped entry is still a legal entry");
            }
            arrived.absorb(&one);
        }
        assert!(!arrived.agrees_with(sent.word()));
        // And the count is no help at all, which is why the word exists: the
        // relay handed on exactly as many entries as it was given.
        assert_eq!(arrived.absorbed(), sent.absorbed());
    }

    #[test]
    fn a_relay_that_reorders_or_drops_moves_the_word_and_a_count_does_not_always() {
        let sent = crossed();

        // Reordered: the same entries, the same number of them, a different
        // sequence. Nothing about a count can see this.
        let mut reordered = Crossing::new();
        for body in Entry::SPECIMENS.into_iter().rev() {
            reordered.absorb(&event(body));
        }
        assert!(!reordered.agrees_with(sent.word()));
        assert_eq!(reordered.absorbed(), sent.absorbed());

        // Dropped out of the middle, which is the case a first-and-last
        // attestation misses.
        let mut dropped = Crossing::new();
        for (index, body) in Entry::SPECIMENS.into_iter().enumerate() {
            if index == 3 {
                continue;
            }
            dropped.absorb(&event(body));
        }
        assert!(!dropped.agrees_with(sent.word()));
    }

    #[test]
    fn a_body_moved_by_one_moves_the_word() {
        // The half the module's *why the whole payload and not the stamp alone*
        // argues for: a relay that kept every reading and rewrote a coordinate
        // has invented what the user did.
        let sent = crossed();
        let mut arrived = Crossing::new();
        for body in Entry::SPECIMENS {
            let moved = match body {
                Entry::PointerMotion(motion) => {
                    Entry::PointerMotion(PointerMotion { x_x65536: motion.x_x65536 + 1, ..motion })
                }
                other => other,
            };
            arrived.absorb(&event(moved));
        }
        assert!(!arrived.agrees_with(sent.word()));
    }

    #[test]
    fn where_the_bytes_were_parked_is_not_part_of_the_crossing() {
        // The clause that lets an entry be carried in a second channel. The
        // arena offset is chosen by whichever ring the payload lands in, so a
        // fold that watched it would go red on a relay that did nothing wrong —
        // and this is the property the compositor's own drain will depend on.
        let sent = crossed();
        let mut elsewhere = Crossing::new();
        for body in Entry::SPECIMENS {
            let mut one = event(body);
            one.payload_offset = 0x40;
            one.user_data = 0xDEAD_BEEF;
            elsewhere.absorb(&one);
        }
        assert!(elsewhere.agrees_with(sent.word()));
    }

    #[test]
    fn a_fold_of_nothing_agrees_with_nobody() {
        // Two stages that both folded nothing hold one basis and would agree,
        // which is a green answer about a crossing that never happened. The
        // guard is what makes a boot whose driver produced nothing fail rather
        // than pass quietly.
        let empty = Crossing::new();
        assert_eq!(empty.absorbed(), 0);
        assert!(!empty.agrees_with(empty.word()));
        assert!(!empty.agrees_with(0));
        assert_ne!(empty.word(), 0, "a page nobody wrote must not read as a fold of nothing");
        assert_eq!(empty, Crossing::default());
    }

    #[test]
    fn the_vocabulary_is_the_one_this_service_has() {
        // Six opcodes and three closed fields, each of which refuses zero. Not
        // a count for its own sake: it is what makes adding a seventh of
        // anything a diff that fails a test somebody has to read, on top of
        // being a diff to the wire crate, which is reviewed as an ABI change.
        assert_eq!(op::COUNT, 6);
        assert_eq!(op::ALL.len(), op::COUNT);
        for opcode in op::ALL {
            assert!(op::known(opcode));
            assert_ne!(op::label(opcode), "unknown");
        }

        assert!(!edge::known(0));
        assert!(edge::known(edge::PRESSED) && edge::known(edge::RELEASED));
        assert!(!touch::known(0));
        for phase in [touch::DOWN, touch::MOVE, touch::UP, touch::CANCEL] {
            assert!(touch::known(phase));
        }
        assert!(!axis_source::known(0));
        assert!(axis_source::known(axis_source::WHEEL) && axis_source::known(axis_source::FINGER));
    }
}
