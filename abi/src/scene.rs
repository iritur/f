// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The scene-delta entry format: six opcodes, one fixed-width payload, and the
//! commit that closes a frame.
//!
//! # What crosses, and what carries it
//!
//! `docs/design/ring-scene-boot.html` section 07 says the compositor is handed
//! edits to a scene it already holds, carried *on an ordinary ring, using the
//! ordinary envelope from part I*. So there is no second transport here and no
//! second framing: a delta is an [`Sqe`] whose opcode names the edit, and a
//! fixed-width record in the channel's inline arena that the entry points at.
//! [`layout`](crate::layout) is where the arena is; this module is what is in
//! it.
//!
//! The split is not decoration. The envelope already carries the four things a
//! scheduler reads — the class, the deadline, the arrival order and the
//! submitter's own token — and every one of them would have had to be invented
//! again inside a self-framing delta stream. A second deadline field is the
//! specific disaster: see *the commit* below.
//!
//! # Why one width for all six
//!
//! Every payload is [`PAYLOAD_BYTES`] long whatever the opcode, so a batch of
//! deltas in the arena is an array with a stride rather than a list that must
//! be parsed to be walked. Three things want that. `E3-B01d` has to cut a
//! commit at every entry boundary and needs the boundaries to be arithmetic.
//! A consumer draining a hostile peer's arena must be able to bound its walk
//! before it believes any field in it. And a producer building a frame writes
//! slot *n* without having finished slot *n − 1*, which is what lets a
//! reconciler emit deltas in whatever order it discovers them.
//!
//! The cost is the padding: the widest record is [`SetTransform`] at 52 bytes
//! and the narrowest is [`RemoveNode`] at four, so a remove carries 52 bytes of
//! nothing. Section 07's arithmetic survives it — fifty deltas at 56 bytes is
//! 2.8 KB against a 4K frame's 33 MB, and the ratio the section argues from is
//! four orders of magnitude either way. *What would reverse this:* a scene
//! whose deltas are dominated by one narrow opcode, measured rather than
//! supposed, at which point the stride becomes per-opcode and the boundaries
//! stop being arithmetic.
//!
//! # An unread field is refused, and it is refused structurally
//!
//! R04 is the rule — an opcode this build does not know is refused and never
//! ignored — and the hard half of it is not the opcode. It is the *field*: a
//! peer that sets something this build does not read has a belief about what
//! just happened that this build does not share, and the two of them will not
//! find out. `abi/`'s existing answer is a `_reserved` word and a check per
//! decoder, which works exactly until somebody adds a field and a decoder that
//! was written before it is the one that sees it.
//!
//! So neither half of an entry is checked against a list of fields written out
//! by hand:
//!
//! - **The payload.** The `Reader` a record decodes through counts what that
//!   decoder consumed, and `Reader::finish` requires every byte past it to be
//!   zero. A field a
//!   record does not read is a field in the tail, and the tail is refused. The
//!   rule is written once, so a seventh record inherits it by existing.
//! - **The envelope.** [`Delta::decode`] rebuilds the [`Sqe`] this build would
//!   have written from the fields it actually read, and compares all
//!   sixty-four bytes. Any field of `Sqe` the encoder does not set — today
//!   `cap`, `buf_set`, `buf_index`, `len`'s other values, `_reserved`, `ext`,
//!   and anything a later ABI adds — must arrive zero, because the canonical
//!   entry has it zero. This is [`Layout::adopt`](crate::layout::Layout::adopt)'s
//!   argument applied one level down: neither side trusts the other's
//!   arithmetic, and both sides state it.
//!
//! [`Delta::envelope`] is written field by field rather than with
//! `..Sqe::ZERO`, so a field added to `Sqe` stops *this* build here and asks
//! whether a scene delta reads it. The byte comparison then makes whatever is
//! answered enforceable instead of documentary.
//!
//! # The commit, and the one copy of its deadline
//!
//! Section 07: *the commit carries a frame token and the deadline it was
//! scheduled against.* The token is the commit's own payload. The deadline is
//! [`Sqe::deadline`] and lives nowhere else, and that is a decision rather than
//! a saving of eight bytes.
//!
//! A deadline written a second time in the payload would be a second number
//! that can disagree with the first, in a system where every resource scheduler
//! already orders by the first — [`deadline::inherit`](crate::deadline::inherit)
//! reads `class` and `deadline` off the entry and nothing else, and a commit
//! whose payload said something different would be served against one number
//! and audited against the other. One field, the one RFC 0009 fixed the unit,
//! the epoch and the zero of.
//!
//! The rule that makes it *the commit's* is stated the other way round: an
//! opcode declares whether it carries a deadline, and a delta that is not a
//! commit must arrive with [`NO_DEADLINE`]. A delta is not scheduled work — it
//! is a write into an arena that the commit makes true — so a deadline on one
//! is a field its opcode does not read, refused by the same rule as any other.
//!
//! # No clock, no floating point, no allocator
//!
//! Nothing here reads a clock: the only time in this module is a deadline a
//! caller computed, and the encoding is a pure function of the value it is
//! given. Nothing here is binary floating point either: a transform is 16.16
//! fixed point in an `i64` and a colour is a `u16` per channel, both with the
//! scale in the name, because the two architectures do not agree on that
//! arithmetic and a scene that rendered differently on one of them would be the
//! whole thesis failing quietly. RFC 0004.
//!
//! # Why the refusals are local
//!
//! [`Refusal`] is this module's own enum, in the shape
//! [`manifest::Refusal`](crate::manifest::Refusal) already uses, and it packs
//! into the domains RFC 0010 fixes without adding a code to
//! [`error::argument`]. Adding one was the obvious move and is the wrong one
//! this week: three sibling entry formats are being written against this file
//! as their model, and a tenth `argument` code invented in each of four places
//! is four meanings wearing one number. Every refusal below reuses a code that
//! already means what it says, and the day one of them genuinely cannot,
//! `store::code`'s note is the precedent for how a new one is numbered.
//!
//! # A seventh opcode
//!
//! It is an RFC, by the rule that makes every change to this crate an ABI
//! change, and it is also a diff to one list: the `entries!` invocation below
//! emits the opcode constants, the [`Entry`] variants, the dispatch, the
//! specimens the round-trip test runs over, and the answer to *does this one
//! carry a deadline*. There is no second place a seventh opcode could be
//! written, and no way to add one that the round-trip test does not pick up.
//!
//! The opcode space is the compositor's own. Section 05 makes an opcode space
//! per-service, so these numbers are not [`crate::op`]'s, not
//! [`control::op`](crate::control::op)'s, and not the semantic ring's; nothing
//! should compare an opcode across two of them.

use crate::{NO_DEADLINE, Sqe, error, flags};

/// Bytes of inline arena one delta's payload occupies, whatever its opcode.
///
/// The widest record is [`SetTransform`] at 52 bytes, and this is the next
/// multiple of eight above it: every multi-byte field in every record below is
/// at most eight wide, so a stride that is a multiple of eight gives the same
/// alignment to the same field in every slot of a batch. Nothing here casts a
/// payload to a struct — every record is encoded by hand, field by field — so
/// the alignment buys a reader's arithmetic rather than a compiler's, which is
/// the only kind this crate is allowed to want.
/// Unit: bytes.
pub const PAYLOAD_BYTES: usize = 56;

/// The width of a submission entry, which [`Delta::decode`] compares in full.
///
/// Stated here as well as in `lib.rs` because the comparison depends on it and
/// on there being no padding inside an [`Sqe`]; the two assertions further down
/// this file are what make both facts rather than hopes.
/// Unit: bytes.
const SQE_BYTES: usize = 64;

/// No node. Zero, so that a zeroed payload names nothing.
///
/// It is also the legal value of [`CreateNode::parent`] for a node with no
/// parent and of [`CreateNode::before`] for a node appended last, which is why
/// this is a named constant rather than a sentinel a reader has to recognise.
/// Unit: none — a node identifier, not a quantity.
pub const NO_NODE: u32 = 0;

/// The submission flags a scene delta may carry.
///
/// [`flags::NO_CQE`] and nothing else. A client writing fifty deltas wants
/// fifty completions it will never read, and RFC 0028's asymmetry still holds:
/// a *refused* delta completes whatever this flag says, so suppressing the
/// success completion costs the client nothing it needs.
///
/// [`flags::LINK`] and [`flags::DRAIN`] are refused rather than honoured,
/// because ordering inside a frame is already decided: the ring's order is the
/// order, and the commit is the barrier. A second ordering mechanism on the
/// same entries is two answers to *what happens before what*, and the one that
/// wins would be whichever the compositor happened to implement.
/// [`flags::FIXED_BUF`] is refused because the payload is in the arena at
/// [`Delta::payload_offset`]; an entry naming a registered buffer instead would
/// be an entry this format does not read.
/// Unit: none — a bitmask of the [`flags`] constants.
pub const FLAGS_ACCEPTED: u8 = flags::NO_CQE;

/// The scene node kinds a delta may create.
///
/// Six, and section 07 argues the number: every additional kind is a new shader
/// path and a new source of pipeline stalls. They are enumerated *here* rather
/// than in the scene crate for [`store::node`](crate::store::node)'s reason — a
/// seventh kind becomes a diff to the wire crate, which is reviewed as an ABI
/// change, rather than a convention a reader is asked to remember. `E3-B01b`
/// turns these into the types a consumer matches on, and its own exit is that a
/// seventh is a compile error there.
///
/// A module of constants rather than an `enum`, which is this crate's shape
/// wherever a value arrives from a peer: an `enum` makes the known values a
/// type and leaves every decoder to invent what an unknown one is. Zero is not
/// a kind, so a zeroed payload decodes as nothing.
pub mod kind {
    /// An affine matrix applied to a subtree.
    /// Unit: none — a node kind, not a quantity.
    pub const TRANSFORM: u16 = 1;
    /// A path restricting a subtree.
    /// Unit: none — a node kind, not a quantity.
    pub const CLIP: u16 = 2;
    /// Opacity and blend mode. Forces an intermediate target only when it
    /// genuinely must, which is the renderer's decision and not this record's.
    /// Unit: none — a node kind, not a quantity.
    pub const LAYER: u16 = 3;
    /// A filled or stroked path, a glyph run, or an image.
    /// Unit: none — a node kind, not a quantity.
    pub const DRAW: u16 = 4;
    /// Blur, drop shadow or a material — parameterised rather than
    /// open-ended.
    /// Unit: none — a node kind, not a quantity.
    pub const EFFECT: u16 = 5;
    /// Role, label, value, relations. In the same tree and submitted in the
    /// same commit, which section 07 calls the cheapest correct decision in the
    /// whole document: accessibility that arrives with the first commit rather
    /// than as a retrofit over thousands of drawing calls with nothing attached
    /// to them.
    /// Unit: none — a node kind, not a quantity.
    pub const SEMANTIC: u16 = 6;

    /// Is this a kind this build knows?
    ///
    /// The negative answer is the one that matters, and the direction of the
    /// mistake is deliberate: a seventh kind added to this file and left out of
    /// this list is *refused*, which is loud, rather than accepted by a decoder
    /// that has no idea what it is.
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, TRANSFORM | CLIP | LAYER | DRAW | EFFECT | SEMANTIC)
    }

    /// A word for a log or a trace.
    #[must_use]
    pub const fn label(value: u16) -> &'static str {
        match value {
            TRANSFORM => "transform",
            CLIP => "clip",
            LAYER => "layer",
            DRAW => "draw",
            EFFECT => "effect",
            SEMANTIC => "semantic",
            _ => "unknown",
        }
    }
}

/// Which points a path encloses.
pub mod fill {
    /// Non-zero winding: the rule almost every vector format means by default.
    /// Unit: none — a fill rule, not a quantity.
    pub const NON_ZERO: u16 = 1;
    /// Even-odd. Kept because a glyph outline with overlapping contours renders
    /// differently under the two, and a format that could not say which would
    /// make the difference the renderer's guess.
    /// Unit: none — a fill rule, not a quantity.
    pub const EVEN_ODD: u16 = 2;

    /// Is this a rule this build knows? Zero is not one.
    #[must_use]
    pub const fn known(value: u16) -> bool {
        matches!(value, NON_ZERO | EVEN_ODD)
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
    /// own fields, a deadline on a delta that is not a commit, or any part of
    /// the envelope this format never looks at. The refusal this module is
    /// shaped around.
    Reserved,
    /// The entry does not frame a payload: a length that is not
    /// [`PAYLOAD_BYTES`], or an arena offset that is not an arena offset.
    Malformed,
    /// A closed field carries a value outside its set — a node kind, a fill
    /// rule, or a node named as its own parent.
    Value,
    /// A field that must name a node holds [`NO_NODE`]. Separate from
    /// [`Refusal::Value`] because a zeroed payload produces exactly this, and a
    /// caller chasing a producer that forgot to fill a record wants to be told
    /// that rather than *some field is wrong*.
    NoNode,
    /// A commit that schedules nothing: [`NO_DEADLINE`], or a frame token of
    /// zero. Both are refused because a commit is the one entry here that is
    /// *scheduled*, and a frame nobody scheduled is a frame the pacing loop
    /// cannot account for. Section 09.
    NotScheduled,
}

impl Refusal {
    /// A line for a log.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::UnknownOpcode => "the opcode is not one of this service's six",
            Self::UnknownFlag => "the entry carries a submission flag a scene delta may not",
            Self::Reserved => "a field this opcode does not read is not zero",
            Self::Malformed => "the entry does not frame a payload of the one width there is",
            Self::Value => "a closed field carries a value outside its set",
            Self::NoNode => "a field that must name a node names none",
            Self::NotScheduled => "a commit carries no deadline or no frame token",
        }
    }

    /// The refusal as a packed [`error`], for a completion.
    ///
    /// Every one is [`error::ARGUMENT`]: the channel is healthy and the peer is
    /// present, and what is wrong is one entry. No code here is new — see the
    /// module's *why the refusals are local* — so [`Refusal::Value`],
    /// [`Refusal::NoNode`] and [`Refusal::NotScheduled`] share
    /// [`error::argument::UNKNOWN_FLAG`], which is the code both
    /// [`manifest::Refusal`](crate::manifest::Refusal) and
    /// [`store::refusal`](crate::store::refusal) already use for *a closed
    /// field carries a value this build does not accept*. They are
    /// distinguishable as values, which is where a caller distinguishes them;
    /// they are one code on the wire, which is where RFC 0010 says the domain
    /// is the stable part.
    #[must_use]
    pub const fn packed(self) -> i32 {
        let code = match self {
            Self::UnknownOpcode => error::argument::UNKNOWN_OPCODE,
            Self::Reserved => error::argument::RESERVED_NOT_ZERO,
            Self::Malformed => error::argument::MALFORMED_HEADER,
            Self::UnknownFlag | Self::Value | Self::NoNode | Self::NotScheduled => {
                error::argument::UNKNOWN_FLAG
            }
        };
        error::pack(error::ARGUMENT, code)
    }
}

/// Writes a record's fields into a payload, counting as it goes.
///
/// Little-endian and by hand, [`store`](crate::store)'s discipline: the layout
/// is the encoding rather than the compiler's opinion of a struct, so nothing
/// here depends on the host's word size, its alignment rules or its byte order.
/// Every slot the writer does not reach stays zero, which is what makes a
/// record's tail the same bytes on both sides of the wire.
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

    /// Append four bytes.
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes.
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes of a signed quantity, two's complement.
    fn i64(&mut self, value: i64) {
        self.put(&value.to_le_bytes());
    }

    /// The one place bytes land.
    ///
    /// Indexing rather than a checked write, and the bound is a compile-time
    /// fact rather than a runtime hope: `entries!` emits
    /// `assert!(WIDTH <= PAYLOAD_BYTES)` for every record it declares — so the
    /// bound is not a list of six that a seventh joins by somebody remembering
    /// — and `the_width_each_record_declares_is_the_width_it_writes` requires
    /// the writer to stop at `WIDTH` for every record there is. A record wide
    /// enough to overflow this cannot reach a build.
    fn put(&mut self, bytes: &[u8]) {
        self.out[self.at..self.at + bytes.len()].copy_from_slice(bytes);
        self.at += bytes.len();
    }
}

/// Reads a record's fields out of a payload, counting as it goes.
///
/// The counting is the point. [`Reader::finish`] refuses every byte the
/// record's decoder did not consume, so *unread* is a property of what the
/// decoder actually did rather than of a width somebody wrote down beside it. A
/// field dropped from a decoder does not become a field that is ignored; it
/// becomes a field in the tail, and the tail is refused.
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

    /// Take eight bytes of a signed quantity, two's complement.
    fn i64(&mut self) -> i64 {
        self.u64().cast_signed()
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
/// Private, because nothing outside this module should encode a payload without
/// the envelope that frames it — [`Delta`] is the door. Its value is what it
/// makes obligatory: a record cannot exist without a width, a specimen, a
/// writer and a reader, so a seventh opcode cannot be added without all four,
/// and [`Entry::SPECIMENS`] picks the specimen up without anybody remembering
/// to add it to a test.
trait Record: Copy + Sized {
    /// The bytes this record's fields occupy, before the padding to
    /// [`PAYLOAD_BYTES`].
    /// Unit: bytes.
    const WIDTH: usize;

    /// One value of this record that is legal on the wire.
    ///
    /// Not a default and not an empty value: every closed field holds something
    /// inside its set, so that decoding it succeeds and the round-trip test has
    /// something to round-trip. A field added to a record without a value here
    /// does not compile.
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
    /// A [`Refusal`] from the record's own fields, or [`Refusal::Reserved`] for
    /// a byte past them — including the bytes of a field the record declares a
    /// width for and its decoder does not consume.
    fn from_payload(raw: &[u8; PAYLOAD_BYTES]) -> Result<Self, Refusal> {
        let mut reader = Reader { raw, at: 0 };
        let value = Self::read(&mut reader)?;
        // The decoder stopped where the record says its fields end, or the two
        // disagree and this entry is not believed. Without this line a decoder
        // that dropped its record's *last* field would be invisible: the bytes
        // it left behind are inside the declared width, so `finish` never sees
        // them and `WIDTH` is only ever checked against the writer. The rule is
        // written once, here, so a seventh record inherits it by existing —
        // which is `Reader::finish`'s own shape, one level up. A decoder that
        // read *past* its width is caught by the same comparison and is the
        // more serious half: it would be reading a later field's bytes.
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

/// The six opcodes, written once.
///
/// # Why a macro, in a tree that mostly refuses them
///
/// Because the alternative is four sequences that have to agree: the opcode
/// constants, the list `known` matches against, the [`Entry`] variants and the
/// specimens a round-trip test iterates. `interface/src/node.rs`'s
/// `vocabulary!` was written after two adversarial reviews found a role that
/// was in an enum and not in an array, and the second review found the hole
/// still open under the test that was supposed to have closed it — because a
/// loop over a list cannot see what the list omits, and an exhaustive match
/// demands an arm rather than an arm that says anything.
///
/// The exit this file is accepted on says *every opcode round-trips*. A
/// hand-written specimen table is precisely how that sentence stops being true
/// while every test stays green: a seventh opcode is added, it is not in the
/// table, and the loop over the table keeps passing. One list closes it.
/// Everything the rest of this module asks of an opcode — its number, its
/// record, its variant, whether it carries a deadline, and a legal value of it
/// — is on the line that declares it.
///
/// The cost is the indirection between a reader and the enum, which is why
/// nothing else in this file is written this way. It is paid here because here
/// the second copy was the defect.
macro_rules! entries {
    (
        $(
            $(#[$about:meta])*
            $variant:ident / $record:ident / $constant:ident = $opcode:literal,
            carries_deadline: $scheduled:literal;
        )*
    ) => {
        /// The compositor's opcode space.
        ///
        /// Per-service and not global: section 05. These numbers mean nothing
        /// on the frame's ring or the control ring, and comparing an opcode
        /// across two spaces is the mistake a single global enumeration would
        /// invite.
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

            /// Does this opcode read [`Sqe::deadline`](crate::Sqe::deadline)?
            ///
            /// `None` for an opcode this build does not know, which is a
            /// different answer from *no* and is kept different on purpose: a
            /// caller that collapsed the two would treat an unknown opcode as
            /// an unscheduled one and go on to read its payload.
            ///
            /// The answer is on the line that declares the opcode, so a
            /// seventh cannot be added without deciding it. Today exactly one
            /// opcode answers `Some(true)`, and the module's *the commit* is
            /// why that is the whole of the deadline's story.
            #[must_use]
            pub const fn carries_deadline(opcode: u8) -> Option<bool> {
                match opcode {
                    $($constant => Some($scheduled),)*
                    _ => None,
                }
            }
        }

        /// What a delta says, once its payload has been believed.
        ///
        /// One variant per opcode, emitted from the same list, so the set of
        /// things a delta can be is the set of opcodes there are.
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
            /// The round-trip test's corpus, and it is derived rather than
            /// written: a seventh opcode is round-tripped by the tests that
            /// already exist, on the day it is declared, without anybody
            /// remembering.
            /// Unit: none — one entry per opcode, in declaration order.
            pub const SPECIMENS: [Self; op::COUNT] = [$(Self::$variant($record::SPECIMEN)),*];

            /// The opcode that names this entry.
            #[must_use]
            pub const fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant(_) => op::$constant,)*
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

            /// The bytes a record's fields occupy, for the caller that needs to
            /// know where its tail starts.
            /// Unit: bytes.
            #[must_use]
            pub const fn width(&self) -> usize {
                match self {
                    $(Self::$variant(_) => $record::WIDTH,)*
                }
            }

            /// The entry an opcode and a payload name.
            fn read(opcode: u8, raw: &[u8; PAYLOAD_BYTES]) -> Result<Self, Refusal> {
                match opcode {
                    $(
                        opcode_pattern!($constant) =>
                            Ok(Self::$variant($record::from_payload(raw)?)),
                    )*
                    _ => Err(Refusal::UnknownOpcode),
                }
            }
        }

        $(
            // Every record fits the one payload width, emitted from the list
            // that declares the records rather than written out beside them. A
            // seventh opcode joins this check by existing, which is what
            // `Writer::put`'s doc claims and what a hand-written list of six
            // could not deliver: a seventh record wider than the stride built
            // clean and panicked at run time instead.
            const _: () = assert!(
                <$record as Record>::WIDTH <= PAYLOAD_BYTES,
                concat!(stringify!($record), " is wider than one payload slot"),
            );
        )*
    };
}

entries! {
    /// Create a node and hang it in the tree.
    ///
    /// The one opcode that introduces an identifier. Every other record here
    /// names a node this one already created, which is what makes *a node
    /// exists because a delta created it* a property of the format rather than
    /// of the compositor's discipline — `E3-B01b`'s exit asks for exactly that.
    CreateNode / CreateNode / CREATE_NODE = 0x01, carries_deadline: false;

    /// Set a node's affine transform.
    ///
    /// Replaces it rather than composing with it. A delta that composed would
    /// make the scene a function of how many times it had been sent, which is
    /// the one thing a retained graph must not be: `E3-B01d` cuts a commit at
    /// every entry boundary and requires the graph read back to be the old
    /// scene or the new one, and a compose-on-apply transform makes a cut into
    /// a third scene neither side ever asked for.
    SetTransform / SetTransform / SET_TRANSFORM = 0x02, carries_deadline: false;

    /// Point a node at path geometry in the arena.
    SetPath / SetPath / SET_PATH = 0x03, carries_deadline: false;

    /// Set what a node is painted with.
    SetPaint / SetPaint / SET_PAINT = 0x04, carries_deadline: false;

    /// Remove a node and everything under it.
    ///
    /// The subtree and not the node alone, because the alternative is a delta
    /// that can leave a child with no parent — a graph state no scene means,
    /// and one every consumer would have to have an opinion about.
    RemoveNode / RemoveNode / REMOVE_NODE = 0x05, carries_deadline: false;

    /// Close the frame: apply every delta since the last commit, or none.
    ///
    /// The only opcode that reads [`Sqe::deadline`](crate::Sqe::deadline), and
    /// the module's *the commit* is why it is the only one.
    Commit / Commit / COMMIT = 0x06, carries_deadline: true;
}

/// Create a node and hang it in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreateNode {
    /// The identifier this node answers to from now on.
    ///
    /// Chosen by the submitter, in the submitter's own space, and never by the
    /// compositor: an identifier the compositor allocated would have to be
    /// returned in a completion, and a client that had to wait for one could
    /// not build a frame without a round trip per node. Section 09's budget has
    /// no room for that, and it is why this is not a handle.
    /// Unit: none — a node identifier, not a quantity. [`NO_NODE`] is refused.
    pub node: u32,
    /// The node this one hangs under.
    /// Unit: none — a node identifier. [`NO_NODE`] is a node with no parent,
    /// which is how a client's own root enters the compositor's tree.
    pub parent: u32,
    /// The sibling this node is inserted in front of.
    ///
    /// Paint order is sibling order, so a format that could only append would
    /// make *insert one node in the middle* into a rebuild of every sibling
    /// after it — and `E3-B01l` names the reorder as the edit a reconciler gets
    /// wrong, which is the edit that would be paying for it.
    /// Unit: none — a node identifier, and a sibling of this node under
    /// `parent`. [`NO_NODE`] appends after the last sibling.
    pub before: u32,
    /// What kind of node this is.
    /// Unit: none — a [`kind`] constant. Zero is not a kind.
    pub kind: u16,
}

impl Record for CreateNode {
    const WIDTH: usize = 4 + 4 + 4 + 2;
    // Every field distinct and none of them zero, which is what makes the
    // round trip an observation rather than a coincidence: a decoder that
    // dropped a field, swapped two of the three identifiers, or read one at the
    // wrong width would produce a value that differs from this one. A specimen
    // whose field happens to hold the value a broken decoder invents — zero is
    // the one every broken decoder invents — cannot tell the two apart.
    // `before` is therefore a real sibling rather than `NO_NODE`. That leaves
    // this corpus blind to the one value of `parent` and `before` that carries
    // a second documented meaning, so the sentinel is observed by
    // `the_sentinel_this_record_documents_round_trips` instead — and it has to
    // be observed somewhere, because a decoder that refused `NO_NODE` outright
    // leaves every test in this module green. Measured: adding
    // `if before == NO_NODE { return Err(Refusal::NoNode) }` to `read` below
    // gave `17 passed; 0 failed` before that test existed.
    // `a_record_that_names_no_node_is_refused` cannot carry this: it asserts
    // that `node: NO_NODE` is *refused*, which is the opposite question.
    const SPECIMEN: Self = Self { node: 7, parent: 3, before: 11, kind: kind::DRAW };

    fn write(&self, out: &mut Writer) {
        out.u32(self.node);
        out.u32(self.parent);
        out.u32(self.before);
        out.u16(self.kind);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let node = raw.u32();
        let parent = raw.u32();
        let before = raw.u32();
        let kind = raw.u16();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        if !kind::known(kind) {
            return Err(Refusal::Value);
        }
        // A node under itself, or in front of itself. The only cycle one entry
        // can state on its own, so it is the only one this format can refuse; a
        // cycle spread over two entries is the graph's to catch, and `E3-B01c`
        // is where that lives.
        if parent == node || before == node {
            return Err(Refusal::Value);
        }
        Ok(Self { node, parent, before, kind })
    }
}

/// Set a node's affine transform.
///
/// Six numbers, `[a b; c d]` and a translation, in the order a matrix is read
/// rather than the order some graphics API happens to pass them. 16.16 fixed
/// point in an `i64`: the scale is in every field's name, and the width is
/// sixty-four bits because the product of two 16.16 values needs that many
/// before it is rescaled — a consumer composing transforms in `i32` would have
/// to widen anyway, and a narrower wire would only move the overflow somewhere
/// harder to see.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetTransform {
    /// The node whose transform this is.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused.
    pub node: u32,
    /// Row one, column one.
    /// Unit: none — a ratio, scaled by 65 536. Zero is a degenerate matrix and
    /// is legal: a scale of zero is how a subtree is collapsed.
    pub a_x65536: i64,
    /// Row one, column two.
    /// Unit: none — a ratio, scaled by 65 536.
    pub b_x65536: i64,
    /// Row two, column one.
    /// Unit: none — a ratio, scaled by 65 536.
    pub c_x65536: i64,
    /// Row two, column two.
    /// Unit: none — a ratio, scaled by 65 536.
    pub d_x65536: i64,
    /// Translation along x.
    /// Unit: device pixels, scaled by 65 536. Zero is no translation.
    pub tx_x65536: i64,
    /// Translation along y.
    /// Unit: device pixels, scaled by 65 536. Zero is no translation.
    pub ty_x65536: i64,
}

impl Record for SetTransform {
    const WIDTH: usize = 4 + 8 * 6;
    // Six distinct non-zero numbers, three of them negative, for
    // `CreateNode::SPECIMEN`'s reason: a matrix whose off-diagonal entries are
    // zero cannot tell a decoder that reads `b` from one that has stopped
    // reading it. This one is a shear as well as a scale, which is the only
    // way the four matrix fields are distinguishable from each other.
    const SPECIMEN: Self = Self {
        node: 0x0A0B_0C0D,
        a_x65536: 65_536,
        b_x65536: 32_768,
        c_x65536: -32_768,
        d_x65536: 131_072,
        tx_x65536: -196_608,
        ty_x65536: 3_407_872,
    };

    fn write(&self, out: &mut Writer) {
        out.u32(self.node);
        out.i64(self.a_x65536);
        out.i64(self.b_x65536);
        out.i64(self.c_x65536);
        out.i64(self.d_x65536);
        out.i64(self.tx_x65536);
        out.i64(self.ty_x65536);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let node = raw.u32();
        let value = Self {
            node,
            a_x65536: raw.i64(),
            b_x65536: raw.i64(),
            c_x65536: raw.i64(),
            d_x65536: raw.i64(),
            tx_x65536: raw.i64(),
            ty_x65536: raw.i64(),
        };
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        Ok(value)
    }
}

/// Point a node at path geometry.
///
/// # Why a range and not a handle
///
/// The geometry is bytes in the same inline arena the payload is in, named by
/// an offset and a length, and this record says nothing at all about what is in
/// them: the path encoding is `E3-B02c`'s, and a wire type that stated it would
/// be a wire type that has to change every time the encoder learns a new verb.
///
/// A range and not a handle because a frame's paths are written by the same
/// producer that writes the deltas, into the same arena, and die with the
/// commit that used them — a handle would be an allocation, a lifetime and a
/// revocation for something whose whole life is one frame. *What would reverse
/// this:* geometry that outlives one commit — a path cached across frames
/// because it is expensive to flatten — which needs a name that survives the
/// arena, and that is a different field rather than a wider one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetPath {
    /// The node whose path this is.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused.
    pub node: u32,
    /// Where the geometry starts.
    /// Unit: bytes from the first byte of the channel's inline arena — the same
    /// origin [`Delta::payload_offset`] uses, and not from the start of the
    /// mapping. Zero is the first byte of the arena, which is a legal place for
    /// geometry to be.
    pub geometry_offset: u32,
    /// How much geometry there is.
    /// Unit: bytes. Zero is an empty path, which encloses nothing and is legal:
    /// it is what a node is set to when its content disappears but the node
    /// stays.
    pub geometry_bytes: u32,
    /// Which points the geometry encloses.
    /// Unit: none — a [`fill`] constant. Zero is not a rule.
    pub fill_rule: u16,
}

impl Record for SetPath {
    const WIDTH: usize = 4 + 4 + 4 + 2;
    const SPECIMEN: Self =
        Self { node: 7, geometry_offset: 0x2000, geometry_bytes: 96, fill_rule: fill::NON_ZERO };

    fn write(&self, out: &mut Writer) {
        out.u32(self.node);
        out.u32(self.geometry_offset);
        out.u32(self.geometry_bytes);
        out.u16(self.fill_rule);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let node = raw.u32();
        let geometry_offset = raw.u32();
        let geometry_bytes = raw.u32();
        let fill_rule = raw.u16();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        if !fill::known(fill_rule) {
            return Err(Refusal::Value);
        }
        // Whether the range lies inside the arena is not a question this record
        // can answer: the arena's length is known to whoever mapped the channel
        // and is deliberately absent from the header — `layout`'s *why the
        // arena's length is not in the header*. The reader that holds the
        // mapping checks it, once, against the length it actually has.
        Ok(Self { node, geometry_offset, geometry_bytes, fill_rule })
    }
}

/// Set what a node is painted with.
///
/// # What this deliberately cannot say
///
/// A solid colour and a stroke width. No gradient, no image, no blend mode, no
/// dash pattern — each of those is a stop list or a handle, and a record that
/// carried one would be a record whose width is a function of its content,
/// which is the fixed stride gone. *What would reverse this:* a scene corpus in
/// which solid fills are the minority, at which point the honest change is a
/// seventh opcode carrying a paint *source* by name, with an RFC for it, rather
/// than a sixth field here that is meaningful only under some values of a
/// fifth.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SetPaint {
    /// The node whose paint this is.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused.
    pub node: u32,
    /// Red.
    ///
    /// **Linear light and not sRGB-encoded**, because section 08 composites in
    /// linear space and a wire value that had to be decoded first would be a
    /// colour whose meaning depends on who decoded it.
    /// Unit: none — an intensity, scaled so that 65 535 is full. Zero is none
    /// of this channel.
    pub red_x65535: u16,
    /// Green, on [`SetPaint::red_x65535`]'s terms.
    /// Unit: none — an intensity, scaled so that 65 535 is full.
    pub green_x65535: u16,
    /// Blue, on [`SetPaint::red_x65535`]'s terms.
    /// Unit: none — an intensity, scaled so that 65 535 is full.
    pub blue_x65535: u16,
    /// Opacity.
    ///
    /// **Straight and not premultiplied.** A premultiplied wire value cannot
    /// express a colour at zero alpha, and a fade interpolates through exactly
    /// that point — so premultiplying here would make the hue at the end of a
    /// fade depend on how many steps it took.
    /// Unit: none — an opacity, scaled so that 65 535 is opaque. Zero is
    /// invisible, which is a paint and not an absence.
    pub alpha_x65535: u16,
    /// How wide the stroke is, or zero for a fill.
    ///
    /// Zero is a real value with a stated meaning rather than a sentinel: a
    /// filled path is a stroke of no width, and one field that is always read
    /// beats two fields of which one is meaningless under the other. The cost
    /// is that a hairline — the stroke that is one device pixel however the
    /// subtree is scaled — cannot be written as a width of zero here and needs
    /// saying some other way. That is the reversal condition.
    /// Unit: device pixels before this node's transform, scaled by 65 536.
    pub stroke_width_x65536: u32,
}

impl Record for SetPaint {
    const WIDTH: usize = 4 + 2 + 2 + 2 + 2 + 4;
    // Six distinct non-zero values, and `stroke_width_x65536` is the field that
    // made the rule: it is the record's last, and a specimen holding zero there
    // made *the decoder read this field* and *the decoder stops before this
    // field* the same observation. 0x0001_8000 is one and a half device pixels.
    const SPECIMEN: Self = Self {
        node: 7,
        red_x65535: 0x1111,
        green_x65535: 0x2222,
        blue_x65535: 0x3333,
        alpha_x65535: 0x4444,
        stroke_width_x65536: 0x0001_8000,
    };

    fn write(&self, out: &mut Writer) {
        out.u32(self.node);
        out.u16(self.red_x65535);
        out.u16(self.green_x65535);
        out.u16(self.blue_x65535);
        out.u16(self.alpha_x65535);
        out.u32(self.stroke_width_x65536);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let node = raw.u32();
        let value = Self {
            node,
            red_x65535: raw.u16(),
            green_x65535: raw.u16(),
            blue_x65535: raw.u16(),
            alpha_x65535: raw.u16(),
            stroke_width_x65536: raw.u32(),
        };
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        Ok(value)
    }
}

/// Remove a node and everything under it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RemoveNode {
    /// The root of the subtree that goes.
    /// Unit: none — a node identifier. [`NO_NODE`] is refused, which is also
    /// what stops an all-zero payload decoding as *remove everything*.
    pub node: u32,
}

impl Record for RemoveNode {
    const WIDTH: usize = 4;
    const SPECIMEN: Self = Self { node: 7 };

    fn write(&self, out: &mut Writer) {
        out.u32(self.node);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let node = raw.u32();
        if node == NO_NODE {
            return Err(Refusal::NoNode);
        }
        Ok(Self { node })
    }
}

/// Close the frame.
///
/// The payload is the token and nothing else; the deadline is the envelope's,
/// and the module's *the commit* is the argument for there being one copy of
/// it. There is no count of the deltas this commit closes either, for the
/// reason [`store::Generation`](crate::store::Generation) stores nothing of its
/// own: the ring's order already says which entries precede this one, and a
/// second statement of a derivable fact is a second thing two writers can
/// disagree about while both pass their own tests. *What would reverse this:* a
/// delta ring with two producers on it, where *the entries before this one* is
/// no longer one client's answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Commit {
    /// Names the frame this commit builds.
    ///
    /// The submitter's, not the compositor's, and opaque here — what it has to
    /// be is *the same token on both sides of the seam*, because `E3-B01j`
    /// counts one frame's boundary crossings on each side and requires the two
    /// counts to agree, and `E3-B01k` publishes it into the compositor's state
    /// tree beside the deadline and the pacing estimate. A token nobody could
    /// match would make both of those a story rather than an observation.
    /// Unit: none — a frame identifier, not a quantity. Zero names no frame and
    /// is refused, so a zeroed payload is not a commit of frame zero.
    pub frame_token: u64,
}

impl Record for Commit {
    const WIDTH: usize = 8;
    const SPECIMEN: Self = Self { frame_token: 0x1234_5678_9ABC_DEF0 };

    fn write(&self, out: &mut Writer) {
        out.u64(self.frame_token);
    }

    fn read(raw: &mut Reader) -> Result<Self, Refusal> {
        let frame_token = raw.u64();
        if frame_token == 0 {
            return Err(Refusal::NotScheduled);
        }
        Ok(Self { frame_token })
    }
}

/// What a commit schedules: the frame, and what it was scheduled against.
///
/// `E3-B01a`'s exit sentence as a type. It exists so that the two numbers
/// travel together — `E3-B01k` publishes both into the compositor's state tree
/// and `E3-B01h` computes the deadline backwards from the next scanout — and so
/// that neither can be read off a commit without the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Frame {
    /// The frame this commit builds.
    /// Unit: none — a frame identifier, not a quantity. Never zero: a commit
    /// carrying zero does not decode.
    pub token: u64,
    /// The deadline the frame was scheduled against.
    /// Unit: nanoseconds, monotonic, in this channel's epoch — the clock
    /// `f_env::Instant` reports, which is the only clock in the system with
    /// ordering authority, and the same field and epoch
    /// [`Sqe::deadline`](crate::Sqe::deadline) states. RFC 0009. Never
    /// [`NO_DEADLINE`]: a commit carrying none does not decode.
    pub deadline: u64,
    /// The class the frame was submitted under, with the depth its urgency has
    /// already crossed.
    /// Unit: none — a `class` field, read by
    /// [`deadline::class_of`](crate::deadline::class_of) and
    /// [`deadline::depth_of`](crate::deadline::depth_of). Zero is `class::HARD`
    /// at depth zero.
    pub class: u16,
}

/// One scene delta as it crosses: part I's envelope, and the body its opcode
/// names.
///
/// Every field here is either read by this format or refused, and there is no
/// third category — which is the sentence the whole module is arranged to make
/// true. See *an unread field is refused* for how each half of it is enforced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Delta {
    /// Returned verbatim in the completion, and opaque here.
    /// Unit: none — chosen by the submitter and never interpreted. Zero is a
    /// legal token; a client that wants to match completions chooses otherwise.
    pub user_data: u64,
    /// The scheduling class, and the depth its urgency has already crossed.
    ///
    /// Read on every opcode and not only the commit, because it is the ring's
    /// field and not this format's: the service that drains the ring orders by
    /// it, and [`deadline::inherit`](crate::deadline::inherit) is what decides
    /// what it means. Nothing here re-decides it, and nothing here may discard
    /// it.
    /// Unit: none — a `class` field: an ordinal in the low byte, a depth in the
    /// high one. Zero is `class::HARD` at depth zero.
    pub class: u16,
    /// The deadline the entry was scheduled against.
    ///
    /// Read only by the opcode [`op::carries_deadline`] answers `true` for,
    /// which today is [`op::COMMIT`] alone; on any other opcode this is a field
    /// the opcode does not read, so it must be [`NO_DEADLINE`] and a non-zero
    /// value is refused rather than ignored.
    /// Unit: nanoseconds, monotonic, in this channel's epoch — RFC 0009.
    /// [`NO_DEADLINE`] is zero and is not a deadline in the past.
    pub deadline: u64,
    /// Where this delta's payload is.
    /// Unit: bytes from the first byte of the channel's inline arena, not from
    /// the start of the mapping — [`crate::op::WRITE_SERIAL`]'s origin, and for
    /// its reason. Zero is the first byte of the arena.
    pub payload_offset: u32,
    /// Submission flags.
    /// Unit: none — a bitmask, and a subset of [`FLAGS_ACCEPTED`]. Zero is no
    /// flags, which is always legal.
    pub flags: u8,
    /// What the delta says.
    /// Unit: none — one opcode's record.
    pub body: Entry,
}

impl Delta {
    /// The opcode this delta submits under.
    #[must_use]
    pub const fn opcode(&self) -> u8 {
        self.body.opcode()
    }

    /// Is this a delta a peer would accept?
    ///
    /// The value-level half of [`Delta::decode`]'s rules, exposed so that a
    /// producer can refuse its own entry before submitting it rather than
    /// learning about it in a completion. It is the same code path, not a
    /// second statement of the same rules.
    ///
    /// # Errors
    ///
    /// [`Refusal::UnknownFlag`] for a flag outside [`FLAGS_ACCEPTED`];
    /// [`Refusal::NotScheduled`] for a commit with no deadline;
    /// [`Refusal::Reserved`] for a deadline on an opcode that does not read
    /// one.
    pub const fn check(&self) -> Result<(), Refusal> {
        envelope_rules(self.opcode(), self.flags, self.deadline)
    }

    /// What a commit schedules, or `None` for a delta that schedules nothing.
    ///
    /// The arms are written out rather than closed with a wildcard, so that a
    /// seventh opcode stops this build and asks whether it names a frame. A
    /// wildcard would answer `None` on its behalf, which is the safe answer and
    /// exactly the kind of safe answer nobody ever revisits.
    #[must_use]
    pub const fn frame(&self) -> Option<Frame> {
        match self.body {
            Entry::Commit(commit) => Some(Frame {
                token: commit.frame_token,
                deadline: self.deadline,
                class: self.class,
            }),
            Entry::CreateNode(_)
            | Entry::SetTransform(_)
            | Entry::SetPath(_)
            | Entry::SetPaint(_)
            | Entry::RemoveNode(_) => None,
        }
    }

    /// The submission entry this delta crosses in.
    ///
    /// Written field by field, with no `..Sqe::ZERO`, so that a field added to
    /// [`Sqe`] stops this build here and asks whether a scene delta reads it.
    /// [`Delta::decode`] then compares an arriving entry against exactly this,
    /// which turns whatever is answered into something a peer cannot get wrong.
    #[must_use]
    pub const fn envelope(&self) -> Sqe {
        Sqe {
            opcode: self.opcode(),
            flags: self.flags,
            class: self.class,
            // A delta names no capability. Authority here is the ring's — the
            // client holds a channel to the compositor, and what it may edit is
            // the tree that channel owns — so a capability index on an entry
            // would be an authority nobody granted and a field nobody reads.
            cap: 0,
            user_data: self.user_data,
            deadline: self.deadline,
            offset: self.payload_offset as u64,
            // The payload is in the arena, at `offset`. `FIXED_BUF` is outside
            // `FLAGS_ACCEPTED`, so these two never name a registered set.
            buf_set: 0,
            buf_index: 0,
            len: PAYLOAD_BYTES as u32,
            _reserved: 0,
            // Not a spare pair of words. A field that belongs to an opcode
            // belongs in that opcode's record, where the record's own width
            // states it and `Reader::finish` polices it; a field in the
            // envelope would be a field every opcode shares whether it reads it
            // or not.
            ext: [0, 0],
        }
    }

    /// Encode: the entry, and the payload it points at.
    ///
    /// Total, on purpose. It writes what it was given, including a value
    /// [`Delta::decode`] would refuse — which is what lets a test build a
    /// malformed entry without a second encoder, and lets `E3-B01d`'s cut model
    /// and `ring/tests/hostile.rs` produce the entries they exist to produce. A
    /// producer that wants the check calls [`Delta::check`] first.
    #[must_use]
    pub fn encode(&self) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        (self.envelope(), self.body.payload())
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The order is the order a refusal is distinguishable in, which is
    /// [`store`](crate::store)'s rule for the same job: the opcode first,
    /// because every later check depends on which opcode this is; the flags and
    /// the deadline next, because they are the two a caller acts on
    /// differently; the framing; then the whole-envelope comparison, which is
    /// the catch-all nothing can be added past; and the payload last, because a
    /// payload is only meaningful once the entry around it has been believed.
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

        let body = Entry::read(entry.opcode, payload)?;
        let delta = Self {
            user_data: entry.user_data,
            class: entry.class,
            deadline: entry.deadline,
            payload_offset: entry.offset as u32,
            flags: entry.flags,
            body,
        };

        // The check this module exists for, on the envelope half. Not a list of
        // the fields a scene delta ignores — such a list is correct on the day
        // it is written and silent afterwards — but the entry this build would
        // have produced from the fields it just read, compared in full. Any
        // field `envelope` does not set, including one a later ABI adds, must
        // arrive zero.
        if sqe_bytes(&delta.envelope()) != sqe_bytes(entry) {
            return Err(Refusal::Reserved);
        }
        Ok(delta)
    }
}

/// The envelope rules that do not need the payload.
///
/// One function, called by [`Delta::check`] before submitting and by
/// [`Delta::decode`] after receiving, so that a producer and a consumer cannot
/// hold different opinions about which entries are legal.
const fn envelope_rules(opcode: u8, flags: u8, deadline: u64) -> Result<(), Refusal> {
    let Some(scheduled) = op::carries_deadline(opcode) else {
        return Err(Refusal::UnknownOpcode);
    };
    if flags & !FLAGS_ACCEPTED != 0 {
        return Err(Refusal::UnknownFlag);
    }
    if scheduled {
        if deadline == NO_DEADLINE {
            return Err(Refusal::NotScheduled);
        }
    } else if deadline != NO_DEADLINE {
        return Err(Refusal::Reserved);
    }
    Ok(())
}

/// The sixty-four bytes of a submission entry.
///
/// A byte view rather than a field-by-field comparison, and the difference is
/// the whole of [`Delta::decode`]'s guarantee: a comparison written out by hand
/// covers the fields somebody listed, and this one covers the fields there are.
fn sqe_bytes(entry: &Sqe) -> &[u8; SQE_BYTES] {
    // SAFETY: `Sqe` is `#[repr(C, align(64))]`, so its fields are laid out in
    // declaration order at fixed offsets; `size_of::<Sqe>()` is `SQE_BYTES` and
    // `SQE_FIELD_BYTES` below adds up the width of each of its fields as the
    // field's own type states it, which the assertion beside it requires to be
    // the same number — so the type carries no padding and every one of its
    // bytes is an initialised byte of an integer. `[u8; SQE_BYTES]` has
    // alignment one, which `Sqe`'s sixty-four satisfies. The reference produced
    // borrows `entry` for its own lifetime and is shared, so nothing is mutated
    // through it and nothing outlives it.
    unsafe { &*core::ptr::from_ref(entry).cast::<[u8; SQE_BYTES]>() }
}

/// The width of the field handed in, taken from the field's own type.
///
/// The type parameter is inferred at the call site, so the answer is the
/// declaration's and not a number somebody typed next to it. That is the whole
/// of the difference between [`SQE_FIELD_BYTES`] and the literal sum it
/// replaced.
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

// The two facts `sqe_bytes` rests on. The first is also asserted in `lib.rs`,
// beside the type; it is asserted again here because this is the code that
// would be unsound without it, and an assertion in another file is a fact this
// one is trusting rather than stating.
const _: () = assert!(core::mem::size_of::<Sqe>() == SQE_BYTES);
// No padding: a `#[repr(C)]` type whose size equals the sum of its fields' own
// widths has none anywhere. It has to be derived rather than transcribed,
// because the interesting edit does not move a single offset: `_reserved: u32`
// narrowed to a `u16` leaves `ext` where it was and puts two uninitialised
// bytes in front of it, and a sum somebody typed — or a chain of `offset_of!`
// comparisons against transcribed widths — still adds to sixty-four. This does
// not, because `SQE_FIELD_BYTES` reads the width off the field.
const _: () = assert!(
    SQE_FIELD_BYTES == SQE_BYTES,
    "Sqe carries padding, so sqe_bytes would read uninitialised memory"
);

// And the stride is the widest record's width rounded to eight. If a record
// ever grows past it, the assertion `entries!` emits for it is what fails; this
// one is here so
// that a *narrowing* — a field deleted, leaving the stride eight bytes wider
// than anything uses — is also visible rather than free.
const _: () = assert!(PAYLOAD_BYTES == SetTransform::WIDTH.next_multiple_of(8));
// The flags this format accepts are flags the ABI defines. A bit removed from
// `flags::KNOWN` and left here would be a bit this build accepts on an entry
// and the ring's own envelope check refuses.
const _: () = assert!(FLAGS_ACCEPTED & !flags::KNOWN == 0);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{class, deadline};

    /// A deadline far enough from zero that no single byte flipped in it lands
    /// back on [`NO_DEADLINE`].
    const SCHEDULED_AT: u64 = 0x0000_0002_1871_1A00;

    /// A delta around a body, with an envelope that is legal for it.
    ///
    /// Derived from [`op::carries_deadline`] rather than written per opcode, so
    /// that a seventh opcode gets a legal envelope from the list that declared
    /// it, and every test below covers it without being edited.
    fn delta(body: Entry) -> Delta {
        let scheduled = op::carries_deadline(body.opcode()) == Some(true);
        Delta {
            user_data: 0x0102_0304_0506_0708,
            class: deadline::pack(class::SOFT, 1),
            deadline: if scheduled { SCHEDULED_AT } else { NO_DEADLINE },
            payload_offset: 0x0E00,
            flags: flags::NO_CQE,
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
            let original = delta(body);
            let label = op::label(original.opcode());
            let (entry, payload) = original.encode();
            assert_eq!(entry.len as usize, PAYLOAD_BYTES, "{label}");
            let back = Delta::decode(&entry, &payload)
                .unwrap_or_else(|refusal| panic!("{label}: {}", refusal.message()));
            assert_eq!(back, original, "{label}");

            // And the encoding is a function of the value alone: the same delta
            // encodes to the same bytes, which is what makes two architectures
            // produce one wire image.
            let (again, again_payload) = back.encode();
            assert_eq!(sqe_bytes(&again), sqe_bytes(&entry), "{label}");
            assert_eq!(again_payload, payload, "{label}");
        }
    }

    #[test]
    fn the_bytes_are_the_ones_written_down() {
        // A real assertion about layout rather than a `size_of`: every byte of
        // one delta, in the position it occupies, with a negative fixed-point
        // value whose sign extension and byte order are both visible. A
        // `to_ne_bytes` that slipped into the writer would pass on a
        // little-endian host and fail this test's *positions* the moment a
        // field's width changed — and both architectures this tree targets are
        // little-endian, so what the arm runner establishes here is that
        // nothing in the encoding depends on the host's alignment or word size.
        let original = delta(Entry::SetTransform(SetTransform::SPECIMEN));
        let (entry, payload) = original.encode();

        #[rustfmt::skip]
        let expected_entry: [u8; SQE_BYTES] = [
            // opcode, flags, class (SOFT at depth one).
            0x02, 0x08, 0x01, 0x01,
            // cap: a delta names no capability.
            0x00, 0x00, 0x00, 0x00,
            // user_data, little-endian.
            0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01,
            // deadline: not a commit, so none.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // offset: 0x0E00 bytes into the arena.
            0x00, 0x0E, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // buf_set, buf_index: the payload is in the arena.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // len: one payload slot, 56 bytes.
            0x38, 0x00, 0x00, 0x00,
            // _reserved.
            0x00, 0x00, 0x00, 0x00,
            // ext.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(sqe_bytes(&entry), &expected_entry);

        #[rustfmt::skip]
        let expected_payload: [u8; PAYLOAD_BYTES] = [
            // node = 0x0A0B_0C0D.
            0x0D, 0x0C, 0x0B, 0x0A,
            // a = 65 536, which is 1.0 in 16.16.
            0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
            // b = 32 768, which is 0.5.
            0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // c = -32 768, which is -0.5: two's complement, sign-extended, and
            // the field that would be invisible if this matrix were diagonal.
            0x00, 0x80, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            // d = 131 072, which is 2.0.
            0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
            // tx = -196 608, which is -3.0.
            0x00, 0x00, 0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
            // ty = 3 407 872, which is 52.0.
            0x00, 0x00, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00,
            // The padding to the stride, which decoding requires to be zero.
            0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(payload, expected_payload);
    }

    #[test]
    fn every_record_is_the_bytes_written_down_beside_it() {
        // The exit says *every opcode*, and a round trip is evidence about a
        // writer and a reader agreeing with each other rather than about either
        // agreeing with the format: a pair that had swapped two fields, or read
        // one at the wrong width, would round-trip perfectly. So every record's
        // field order and field widths are pinned here as literal bytes, once,
        // against the specimen the round trip already uses.
        //
        // This list is written by hand, because a derived one would be derived
        // from the encoder it is supposed to check. What is *not* left to
        // memory is whether it is complete: the assertions below require one
        // image per opcode, in `op::ALL`'s order, so a seventh opcode fails
        // this test on the day it is declared rather than being quietly
        // uncovered by it. That is the difference between this list and the
        // hand-written arrays `docs/postmortem/0001` is about.
        #[rustfmt::skip]
        let images: [(Entry, &[u8]); op::COUNT] = [
            (Entry::CreateNode(CreateNode::SPECIMEN), &[
                0x07, 0x00, 0x00, 0x00,  // node = 7
                0x03, 0x00, 0x00, 0x00,  // parent = 3
                0x0B, 0x00, 0x00, 0x00,  // before = 11
                0x04, 0x00,              // kind = DRAW
            ]),
            (Entry::SetTransform(SetTransform::SPECIMEN), &[
                0x0D, 0x0C, 0x0B, 0x0A,
                0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x80, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x80, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                0x00, 0x00, 0x02, 0x00, 0x00, 0x00, 0x00, 0x00,
                0x00, 0x00, 0xFD, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF,
                0x00, 0x00, 0x34, 0x00, 0x00, 0x00, 0x00, 0x00,
            ]),
            (Entry::SetPath(SetPath::SPECIMEN), &[
                0x07, 0x00, 0x00, 0x00,  // node = 7
                0x00, 0x20, 0x00, 0x00,  // geometry_offset = 0x2000
                0x60, 0x00, 0x00, 0x00,  // geometry_bytes = 96
                0x01, 0x00,              // fill_rule = NON_ZERO
            ]),
            (Entry::SetPaint(SetPaint::SPECIMEN), &[
                0x07, 0x00, 0x00, 0x00,  // node = 7
                0x11, 0x11,              // red
                0x22, 0x22,              // green
                0x33, 0x33,              // blue
                0x44, 0x44,              // alpha
                0x00, 0x80, 0x01, 0x00,  // stroke width = 1.5 device pixels
            ]),
            (Entry::RemoveNode(RemoveNode::SPECIMEN), &[
                0x07, 0x00, 0x00, 0x00,  // node = 7
            ]),
            (Entry::Commit(Commit::SPECIMEN), &[
                0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12,  // frame token
            ]),
        ];

        let covered: [u8; op::COUNT] = images.map(|(body, _)| body.opcode());
        assert_eq!(covered, op::ALL, "an opcode has no byte image written down for it");

        for (body, image) in images {
            let label = op::label(body.opcode());
            assert_eq!(image.len(), body.width(), "{label}: the image is not the record's width");
            let payload = body.payload();
            assert_eq!(&payload[..image.len()], image, "{label}");
            assert!(payload[image.len()..].iter().all(|byte| *byte == 0), "{label}");
        }
    }

    /// A record whose decoder stops one field short of its declared width.
    ///
    /// Test-only, and it is the only way this file can observe the rule
    /// `Record::from_payload` states: every record that actually ships reads
    /// exactly what it declares, so deleting that rule changes nothing any real
    /// corpus can see. What it would change is the day a seventh record's
    /// decoder drops its last field — and a guard whose absence nothing
    /// notices is a guard a later edit walks past. This is that day, written
    /// down.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Stumped {
        /// The field the decoder reads.
        /// Unit: none — a test value.
        read: u32,
        /// The field it declares a width for and does not read.
        /// Unit: none — a test value.
        dropped: u32,
    }

    impl Record for Stumped {
        const WIDTH: usize = 4 + 4;
        const SPECIMEN: Self = Self { read: 0x1111_1111, dropped: 0x2222_2222 };

        fn write(&self, out: &mut Writer) {
            out.u32(self.read);
            out.u32(self.dropped);
        }

        fn read(raw: &mut Reader) -> Result<Self, Refusal> {
            // The mistake, spelled out: the second field is written and never
            // consumed, so the reader stops at four and the record says eight.
            Ok(Self { read: raw.u32(), dropped: 0 })
        }
    }

    #[test]
    fn a_record_that_stops_short_of_its_declared_width_is_refused() {
        // `Reader::finish` cannot see this: the bytes the decoder skipped are
        // inside the width the record declares, so they are not in the tail.
        // The comparison of `Reader::at` against `WIDTH` is what sees it, and
        // this is the test that goes red when that comparison is deleted.
        let payload = Stumped::SPECIMEN.to_payload();
        assert_eq!(Stumped::from_payload(&payload), Err(Refusal::Reserved));

        // And the rule is about the *decoder*, not about the bytes: a payload
        // whose dropped field happens to be zero is refused just the same,
        // which is the case a weak corpus would have let through.
        let mut blank = payload;
        blank[4..8].fill(0);
        assert_eq!(Stumped::from_payload(&blank), Err(Refusal::Reserved));
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
        // no byte of a delta — envelope or payload — that can be changed
        // without either changing what the delta means or being refused. A
        // field this format read and then dropped would show up here as a byte
        // that can be flipped and still decode to the value it started as.
        //
        // The corpus is `Entry::SPECIMENS` and the positions are every position
        // there is, so neither is a list a seventh opcode or a seventh field
        // could be left out of.
        for body in Entry::SPECIMENS {
            let original = delta(body);
            let (entry, payload) = original.encode();
            let label = op::label(original.opcode());

            for at in 0..SQE_BYTES {
                let mut damaged = entry;
                flip(&mut damaged, at);
                assert_ne!(
                    Delta::decode(&damaged, &payload),
                    Ok(original),
                    "{label}: byte {at} of the envelope is ignored"
                );
            }

            for at in 0..PAYLOAD_BYTES {
                let mut damaged = payload;
                damaged[at] ^= 0xFF;
                assert_ne!(
                    Delta::decode(&entry, &damaged),
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
        // accept an entry written by a peer that has that field.
        for body in Entry::SPECIMENS {
            let original = delta(body);
            let (entry, payload) = original.encode();
            let label = op::label(original.opcode());
            for at in body.width()..PAYLOAD_BYTES {
                let mut damaged = payload;
                damaged[at] = 1;
                assert_eq!(
                    Delta::decode(&entry, &damaged),
                    Err(Refusal::Reserved),
                    "{label}: byte {at} past the record is not refused"
                );
            }
        }
    }

    #[test]
    fn the_width_each_record_declares_is_the_width_it_writes() {
        // `WIDTH` is what the assertions under the records bound against
        // `PAYLOAD_BYTES`, and `Writer::put` indexes on the strength of them. A
        // record whose writer disagreed with its declared width would make that
        // bound a statement about the wrong number, so the two are required to
        // agree here — over every record there is, because the corpus is the
        // emitted one.
        for body in Entry::SPECIMENS {
            let payload = body.payload();
            assert!(body.width() <= PAYLOAD_BYTES);
            // Everything past the declared width is untouched by the writer,
            // which is the same statement as *the writer stopped at `width`*
            // for a buffer that started zeroed.
            assert!(
                payload[body.width()..].iter().all(|byte| *byte == 0),
                "{}: the writer went past its declared width",
                op::label(body.opcode())
            );
        }
    }

    #[test]
    fn the_commit_carries_the_frame_and_nothing_else_may() {
        // The exit's third clause. The token comes off the payload and the
        // deadline off the envelope, and `frame` is the only route to either.
        let commit = delta(Entry::Commit(Commit::SPECIMEN));
        let (entry, payload) = commit.encode();
        let back = Delta::decode(&entry, &payload).unwrap();
        let frame = back.frame().expect("a commit names a frame");
        assert_eq!(frame.token, Commit::SPECIMEN.frame_token);
        assert_eq!(frame.deadline, SCHEDULED_AT);
        assert_eq!(frame.class, deadline::pack(class::SOFT, 1));
        // And the deadline is the envelope's own field, which is the one every
        // scheduler in the system already orders by.
        assert_eq!(entry.deadline, frame.deadline);

        for body in Entry::SPECIMENS {
            let other = delta(body);
            if other.opcode() == op::COMMIT {
                continue;
            }
            assert_eq!(other.frame(), None, "{} names a frame", op::label(other.opcode()));
        }
    }

    #[test]
    fn a_commit_that_schedules_nothing_is_refused() {
        // Both halves of "scheduled": a commit with no deadline is a frame
        // nobody paced, and a commit with no token is a frame nobody can match
        // across the seam — which is what `E3-B01j`'s two counters have to do.
        let mut unscheduled = delta(Entry::Commit(Commit::SPECIMEN));
        unscheduled.deadline = NO_DEADLINE;
        assert_eq!(unscheduled.check(), Err(Refusal::NotScheduled));
        let (entry, payload) = unscheduled.encode();
        assert_eq!(Delta::decode(&entry, &payload), Err(Refusal::NotScheduled));

        let nameless = delta(Entry::Commit(Commit { frame_token: 0 }));
        let (entry, payload) = nameless.encode();
        assert_eq!(Delta::decode(&entry, &payload), Err(Refusal::NotScheduled));
    }

    #[test]
    fn a_delta_that_is_not_a_commit_may_not_carry_a_deadline() {
        // The rule that makes the deadline the commit's: on every other opcode
        // it is a field the opcode does not read, so it goes through the same
        // refusal as any other unread field rather than being quietly dropped
        // by a compositor that only ever looks at a commit's.
        for body in Entry::SPECIMENS {
            let mut wrong = delta(body);
            if wrong.opcode() == op::COMMIT {
                continue;
            }
            wrong.deadline = SCHEDULED_AT;
            assert_eq!(wrong.check(), Err(Refusal::Reserved));
            let (entry, payload) = wrong.encode();
            assert_eq!(
                Delta::decode(&entry, &payload),
                Err(Refusal::Reserved),
                "{} was allowed a deadline",
                op::label(wrong.opcode())
            );
        }
    }

    #[test]
    fn an_opcode_this_build_does_not_know_is_refused() {
        // R04, and the case that matters most: a zeroed slot of a fresh mapping
        // is not opcode zero doing nothing, it is an entry nobody wrote.
        assert_eq!(Delta::decode(&Sqe::ZERO, &[0; PAYLOAD_BYTES]), Err(Refusal::UnknownOpcode));

        assert!(!op::known(0));
        assert_eq!(op::carries_deadline(0), None);
        for opcode in 0..=u8::MAX {
            assert_eq!(op::known(opcode), op::ALL.contains(&opcode));
            assert_eq!(op::known(opcode), op::carries_deadline(opcode).is_some());
        }

        let mut foreign = delta(Entry::Commit(Commit::SPECIMEN)).envelope();
        foreign.opcode = 0xFE;
        assert_eq!(
            Delta::decode(&foreign, &[0; PAYLOAD_BYTES]),
            Err(Refusal::UnknownOpcode),
            "the top of the space is RFC 0028's, not this service's"
        );
    }

    #[test]
    fn a_flag_a_delta_may_not_carry_is_refused() {
        // Refused and not masked: a client that believed its deltas were linked
        // and a compositor that dropped the bit would disagree about the order
        // a frame was applied in, and neither would ever find out.
        for bit in [flags::LINK, flags::DRAIN, flags::FIXED_BUF] {
            let mut linked = delta(Entry::RemoveNode(RemoveNode::SPECIMEN));
            linked.flags = bit;
            assert_eq!(linked.check(), Err(Refusal::UnknownFlag));
            let (entry, payload) = linked.encode();
            assert_eq!(Delta::decode(&entry, &payload), Err(Refusal::UnknownFlag));
        }
        // And a bit the ABI itself does not define.
        let mut invented = delta(Entry::RemoveNode(RemoveNode::SPECIMEN));
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
        let original = delta(Entry::SetPaint(SetPaint::SPECIMEN));
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
            assert_eq!(Delta::decode(&damaged, &payload), Err(Refusal::Reserved));
        }

        // The framing, which is a different disbelief and gets a different
        // refusal: this entry does not frame a payload at all.
        let mut short = entry;
        short.len = PAYLOAD_BYTES as u32 - 1;
        assert_eq!(Delta::decode(&short, &payload), Err(Refusal::Malformed));

        let mut far = entry;
        far.offset = u64::from(u32::MAX) + 1;
        assert_eq!(Delta::decode(&far, &payload), Err(Refusal::Malformed));
    }

    #[test]
    fn a_closed_field_outside_its_set_is_refused() {
        // A node kind and a fill rule are the two closed fields in the payload,
        // and both are refused rather than clamped to something plausible: a
        // seventh node kind arriving from a peer built later than this one is a
        // scene this build cannot draw, and drawing something else instead is
        // how two compositors come to disagree about one frame.
        let seventh = CreateNode { kind: kind::SEMANTIC + 1, ..CreateNode::SPECIMEN };
        let (entry, payload) = delta(Entry::CreateNode(seventh)).encode();
        assert_eq!(Delta::decode(&entry, &payload), Err(Refusal::Value));

        let unruled = SetPath { fill_rule: 0, ..SetPath::SPECIMEN };
        let (entry, payload) = delta(Entry::SetPath(unruled)).encode();
        assert_eq!(Delta::decode(&entry, &payload), Err(Refusal::Value));

        // A node cannot be its own parent, which is the only cycle one entry
        // can state without help.
        let looped = CreateNode { parent: CreateNode::SPECIMEN.node, ..CreateNode::SPECIMEN };
        let (entry, payload) = delta(Entry::CreateNode(looped)).encode();
        assert_eq!(Delta::decode(&entry, &payload), Err(Refusal::Value));
    }

    #[test]
    fn a_record_that_names_no_node_is_refused() {
        // Which is also the statement that a zeroed payload decodes as nothing,
        // for every opcode that names a node.
        for body in [
            Entry::CreateNode(CreateNode { node: NO_NODE, ..CreateNode::SPECIMEN }),
            Entry::SetTransform(SetTransform { node: NO_NODE, ..SetTransform::SPECIMEN }),
            Entry::SetPath(SetPath { node: NO_NODE, ..SetPath::SPECIMEN }),
            Entry::SetPaint(SetPaint { node: NO_NODE, ..SetPaint::SPECIMEN }),
            Entry::RemoveNode(RemoveNode { node: NO_NODE }),
        ] {
            let (entry, payload) = delta(body).encode();
            assert_eq!(
                Delta::decode(&entry, &payload),
                Err(Refusal::NoNode),
                "{} accepted a payload that names no node",
                op::label(body.opcode())
            );
        }
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
            Refusal::NoNode,
            Refusal::NotScheduled,
        ] {
            let packed = refusal.packed();
            assert!(packed < 0, "{}", refusal.message());
            let (domain, _) = error::unpack(packed).expect("a refusal is an error");
            assert_eq!(domain, error::ARGUMENT, "{}", refusal.message());
            assert!(!refusal.message().is_empty());
        }
    }

    #[test]
    fn the_sentinel_this_record_documents_round_trips() {
        // `CreateNode::node`'s doc says `NO_NODE` is refused there and
        // `a_record_that_names_no_node_is_refused` observes that. `parent` and
        // `before` document the opposite — `NO_NODE` means *no parent* and
        // *append after the last sibling*, the two edits a client's first frame
        // is made of — and a sentinel whose meaning is documented and whose
        // decode nobody observes is a sentence in the format with no guard
        // under it. `Entry::SPECIMENS` cannot be that guard: its values are
        // chosen distinct and non-zero on purpose, so that a decoder which
        // dropped or swapped a field produces a different value, and `NO_NODE`
        // is `u32::MAX` in both of those fields at once.
        //
        // *What would reverse this:* `NO_NODE` losing its second meaning in
        // `parent` or `before` — at which point the entry below stops being
        // legal and this test becomes a refusal assertion rather than a round
        // trip.
        let rooted = CreateNode { parent: NO_NODE, before: NO_NODE, ..CreateNode::SPECIMEN };
        let original = delta(Entry::CreateNode(rooted));
        let (entry, payload) = original.encode();
        let back = Delta::decode(&entry, &payload)
            .unwrap_or_else(|refusal| panic!("a rooted, appended node: {}", refusal.message()));
        assert_eq!(back, original);
    }

    #[test]
    fn the_vocabulary_is_the_one_the_design_names() {
        // Section 07's six node kinds and this service's six opcodes. Not a
        // count for its own sake: it is what makes adding a seventh of either a
        // diff that fails a test somebody has to read, on top of being a diff
        // to the wire crate, which is reviewed as an ABI change.
        assert_eq!(op::COUNT, 6);
        let kinds =
            [kind::TRANSFORM, kind::CLIP, kind::LAYER, kind::DRAW, kind::EFFECT, kind::SEMANTIC];
        for value in kinds {
            assert!(kind::known(value));
            assert_ne!(kind::label(value), "unknown");
        }
        // Swept over every `u16` there is rather than at zero and one past the
        // end, for the reason `an_opcode_this_build_does_not_know_is_refused`
        // sweeps all 256 opcodes: a `known` that accepted a *distant* value —
        // a mistyped constant, a range where a list was meant — is invisible
        // to a test that only asks about the two values beside the set.
        for value in 0..=u16::MAX {
            assert_eq!(kind::known(value), kinds.contains(&value), "kind {value}");
            assert_eq!(kind::known(value), kind::label(value) != "unknown", "kind {value}");
            assert_eq!(
                fill::known(value),
                value == fill::NON_ZERO || value == fill::EVEN_ODD,
                "fill rule {value}"
            );
        }
    }
}
