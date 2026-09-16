// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Canvas participation across a ring: three entries, and the floor enforced by
//! whoever is about to render rather than only by whoever declared.
//!
//! # What crosses, and what carries it
//!
//! `docs/design/ring-scene-boot.html` section 13 says a canvas declares *both*
//! its custom rendering *and* a semantic model of its content, and RFC 0078
//! turned that into four clauses a declaration meets or is an **escape**.
//! [`semantic`](crate::semantic) carries the tree; this carries what section 13
//! asks for on top of it — where each occupant of a canvas sits in the
//! canvas's own dimension.
//!
//! There is no second transport and no second framing. A participation entry is
//! an [`Sqe`] whose opcode names the edit and a fixed-width record in the
//! channel's inline arena that the entry points at, exactly as
//! [`scene`](crate::scene) and [`semantic`](crate::semantic) already do;
//! [`layout`](crate::layout) is where the arena is. Where this format diverges
//! from its two siblings it says so: it carries no deadline and no handshake,
//! and its commit carries nothing at all.
//!
//! # The problem this module exists for
//!
//! RFC 0078's floor is real and it is enforced in exactly one place:
//! `interface`'s `Arrangement::admit`, which is a function in the process that
//! *declares* the canvas. The type it returns cannot be built any other way, so
//! within that process the floor is structural — a projection that wants to
//! present a canvas must handle the escape, because there is no value that
//! answers *which clip is at 5 s* for a canvas that did not meet it.
//!
//! **That guarantee stops at the process boundary, and it stops silently.** A
//! peer on the far side of a ring never calls that constructor. It writes
//! bytes. It can declare a canvas, name its time base, name its selection, and
//! then simply not send the placements — and a receiver that took the bytes at
//! face value would hold a canvas it can draw and cannot describe, which is the
//! opaque rectangle section 13 says sank every predecessor, arriving through
//! the one door the type system does not watch. Nothing about that peer is
//! exotic: the declaration that omits the placements is the one written last,
//! under pressure, by somebody who has the pixels working.
//!
//! So the floor is enforced twice, and the second time is here:
//!
//! - **[`Escape`] is this module's refusal type**, and it is the refusal a
//!   *decode* returns, not only the refusal a constructor returns. The variants
//!   that name a floor clause are spelled as `interface` spells them, so that
//!   *the canvas placed nothing* is one word on both sides of the ring rather
//!   than two error types a reader has to align by hand.
//! - **A receiver cannot obtain an answer about a canvas without the floor.**
//!   [`Reception::accept`] hands back [`Progress::Assembling`] for every entry
//!   that is not the commit, and that variant carries *nothing at all* — no
//!   placement, no span, no occupant. The only value in this module that
//!   answers a question about a canvas is [`Participation`], it is returned by
//!   one arm of one function, and that arm is past the floor.
//!
//! The second point is the one worth stating as a removal rather than a rule.
//! An earlier shape of this module decoded each entry into a public `Entry` and
//! let the receiver assemble what it liked; the floor was then a function a
//! receiver was *asked* to call, which is a convention, and conventions fail in
//! the direction of whoever is in a hurry. There is nothing to call past now,
//! because there is nothing to call it on.
//!
//! # Who answers for the receiving tree
//!
//! Clause 4 is a statement about a declaration **and** a tree — *every clip and
//! every marker the tree declares under this canvas has a placement* — so a
//! receiver enforcing it has to be able to ask its own tree what is under the
//! canvas. This crate cannot: `interface/` is a leaf that nothing depends on
//! and this crate sits beneath the frame, so neither can see the other, which
//! is argued at length in [`semantic`](crate::semantic).
//!
//! [`Presented`] is how the question is asked, on [`Vocabulary`](crate::semantic::Vocabulary)'s
//! terms exactly: one trait, implemented by the component that already holds
//! the tree, answering facts rather than judgements. Its answer is a closed
//! [`Census`] whose every arm is a named [`Escape`], so an implementor chooses
//! nothing about what happens next — it says what its tree holds, and this
//! module says what that means.
//!
//! What an implementor owes, and what cannot be policed from here, is
//! [`Presented`]'s own documentation. The short version is the one
//! [`semantic`](crate::semantic) gives about vocabularies: an implementation
//! that reports no occupants under every canvas defeats the floor, and nothing
//! in this crate can detect that. What *this* module guarantees is narrower and
//! is what the exit asks — it never produces a canvas-shaped answer out of
//! placements that did not arrive, and it has no fallback to fall back to.
//!
//! # Why the shapes are restated rather than imported
//!
//! [`Ticks`], [`Extent`], [`Span`], [`Placement`], [`Selection`] and
//! [`Arrangement`] all exist in `interface/src/canvas.rs` under those names.
//! They are written again here, and that is the same decision
//! [`semantic::RELATIONS_MAX`](crate::semantic::RELATIONS_MAX) records one
//! level down: the wire has to state its own shapes, because a consumer on the
//! far side does arithmetic on them before it believes any field, and it cannot
//! link a crate it has never heard of to do it.
//!
//! The copy is held against the original by the tests, which `include_str!`
//! `interface/src/canvas.rs` and RFC 0078 and read them as text — the technique
//! `interface/src/ladder.rs` already uses to hold itself to RFC 0080 and
//! `claims/0033`. No crate is linked and no type is imported; a text file is
//! read at compile time. So the two lists cannot drift quietly, only loudly:
//! a clause added to `interface`'s `Escape` fails a test in this crate until
//! somebody decides whether it crosses.
//!
//! # An unread field is refused, and it is refused structurally
//!
//! [`scene`](crate::scene)'s rule and [`semantic`](crate::semantic)'s,
//! unchanged, so that three formats in one crate cannot hold three opinions:
//!
//! - **The payload.** Every record decodes through a `Reader` that counts what
//!   it consumed, and `Reader::finish` requires every byte past that to be
//!   zero. [`op::PARTICIPATE`] is the entry that shows what this buys — its
//!   record reads *nothing*, so its whole payload is the tail, and a peer that
//!   smuggled a field into a commit is refused by the rule that was already
//!   there.
//! - **The envelope.** A decode rebuilds the [`Sqe`] this build would have
//!   written from the fields it actually read and compares all sixty-four
//!   bytes. Any field this format does not set — `cap`, `deadline`, `buf_set`,
//!   `buf_index`, `_reserved`, `ext`, and anything a later ABI adds — must
//!   arrive zero.
//!
//! # No deadline, and no handshake
//!
//! No deadline for [`semantic`](crate::semantic)'s reason and not a weaker
//! version of it: the same canvas is presented at once to a display, a screen
//! reader and an agent, and those three present it at three different moments
//! by construction, so a deadline here would be a second answer to *when is
//! this shown* in a system where no single answer is meaningful.
//!
//! No handshake because there is nothing to agree. This format carries no
//! ordinal of any closed enum — no role, no relation, no content kind. It
//! carries node identifiers, which are the submitter's own and are opaque here;
//! a tick count, which is an integer in a base the declaration itself states;
//! and two discriminants of two enums declared in this file, which are the
//! format's own and move only when the ABI does. The channel that carried the
//! tree negotiated the vocabulary already, and a second negotiation over the
//! same channel would be a second thing to keep in step.
//!
//! # No clock, no floating point, no allocator
//!
//! Every function here is a pure function of the bytes and the [`Presented`]
//! tree it is handed, and the tree is a parameter rather than something
//! consulted for the reason RFC 0004 gives: a decoder that observed something
//! could be made to decode one payload two ways. Nothing reads a clock, draws
//! randomness or observes an ordering.
//!
//! Nothing here is binary floating point either, and the absence is
//! load-bearing rather than incidental. *4.2 s* is not expressible in this
//! tree — the two architectures do not agree on the two floating-point types —
//! so a time crosses as a count of ticks and the declaration states what a tick
//! is worth. `interface/src/canvas.rs` argues at length why that base is the
//! application's and why milliseconds are the wrong answer; the wire's job is
//! to carry the pair without rounding either half, and a base of zero ticks a
//! second is [`Escape::NotATimeBase`] rather than a division nobody survives.
//!
//! # What this module is not
//!
//! It is not a delta protocol. A participation frame declares a canvas whole:
//! the canvas, its base, its selection, every placement, and the commit. Moving
//! one clip re-sends the frame. That is honest for what exists — [`PLACED_MAX`]
//! is sixty-four, which is a screen of a timeline and not a timeline, and an
//! application with more occupants than that is declaring a window of its
//! content rather than all of it — and it is the smallest thing that can carry
//! the floor, because the floor is a statement about a canvas and not about an
//! edit. RFC 0078 records windowing as the first thing a corpus will press on,
//! and a delta protocol is what presses second.

use crate::{NO_DEADLINE, Sqe, error, flags};

/// The most occupants one arrangement may place, on the wire as in the
/// declaration.
///
/// A second statement of `interface`'s `PLACED_MAX`, and a deliberate one: the
/// wire has to state its own capacity, because a consumer counts entries
/// against it before it believes any of them. The two are held together by
/// `an_escape_is_the_same_word_on_both_sides_of_the_ring`, which reads
/// `interface/src/canvas.rs` at compile time, so the copy cannot drift quietly.
///
/// *What would reverse this:* a canvas whose window of content does not fit
/// sixty-four occupants. At that point a participation frame stops being one
/// canvas and the atomicity the commit gives it — the whole arrangement is
/// admitted or none of it is — has to be rebuilt out of something else.
/// Unit: occupants.
pub const PLACED_MAX: usize = 64;

/// Bytes of inline arena one participation entry's payload occupies, whatever
/// its opcode.
///
/// The widest record is [`Declaring`], and this is the next multiple of eight
/// above it. Stated as its own constant rather than borrowed from
/// [`semantic::PAYLOAD_BYTES`](crate::semantic::PAYLOAD_BYTES): a format that
/// inherited another format's stride would change silently the day that one
/// grew a field, and the two are not one decision.
/// Unit: bytes.
pub const PAYLOAD_BYTES: usize = 40;

/// The width of a submission entry, which a decode compares in full.
///
/// Stated here as well as in `lib.rs` because the comparison depends on it and
/// on there being no padding inside an [`Sqe`]; the assertions at the foot of
/// this file are what make both facts rather than hopes.
/// Unit: bytes.
const SQE_BYTES: usize = 64;

/// No node. Zero, so that a zeroed payload names nothing.
///
/// The same value and the same reasoning as
/// [`semantic::NO_NODE`](crate::semantic::NO_NODE) and `interface`'s
/// `NodeId::UNNAMED`. Here it is also the legal value of
/// [`Selection::lane`](Selection::lane) for a selection confined to no lane,
/// which is why it is a named constant rather than a sentinel a reader has to
/// recognise.
/// Unit: none — a node identifier, not a quantity.
pub const NO_NODE: u64 = 0;

/// The submission flags a participation entry may carry.
///
/// [`flags::NO_CQE`] and nothing else, for
/// [`semantic::FLAGS_ACCEPTED`](crate::semantic::FLAGS_ACCEPTED)'s reasons: a
/// client placing sixty-four clips does not want sixty-four completions it will
/// never read, and a *refused* entry completes whatever this flag says, which
/// matters more here than anywhere because here the refusal is the whole point.
///
/// [`flags::LINK`] and [`flags::DRAIN`] are refused rather than honoured,
/// because ordering inside a frame is already decided: the ring's order is the
/// order and the commit is the barrier. [`flags::FIXED_BUF`] is refused because
/// the payload is in the arena.
/// Unit: none — a bitmask of the [`flags`] constants.
pub const FLAGS_ACCEPTED: u8 = flags::NO_CQE;

/// Emits the refusals, their codes and their words, from one list.
///
/// # Why a macro, in a crate that mostly refuses them
///
/// Because the alternative is four sequences that have to agree: the variants,
/// the array the agreement test iterates, the word each one logs under, and the
/// completion code each one packs into. `interface/src/node.rs`'s `vocabulary!`
/// was written after two adversarial reviews found a role that was in an enum
/// and not in an array, and the second review found the hole still open under
/// the test that was supposed to have closed it — because a loop over a
/// hand-written list cannot see what the list omits.
///
/// [`Escape::LABELS`] is the array that matters, because it is what the
/// agreement test compares against `interface`'s own `Escape`. Written by hand
/// it would be the same defect one crate over: a clause added here and left out
/// of the array is a clause the test never looks at, and the test would stay
/// green while the sentence it is named after stopped being true. There is no
/// way to write a variant this array does not get.
macro_rules! escapes {
    (
        $(
            $(#[$about:meta])*
            $variant:ident $(($payload:ty))? = $code:expr, $message:literal;
        )*
    ) => {
        /// Why a canvas is not a participant.
        ///
        /// **This is a wire refusal.** `interface`'s `Escape` is what a
        /// declaration gets from a constructor in the process that wrote it;
        /// this is what an arriving declaration gets from the process that is
        /// about to render it, and the clauses they share are spelled the same
        /// way on purpose. A receiver that reports *the canvas placed nothing*
        /// and a declarer that was refused for it are reporting one fact, and a
        /// reader tracing a bug across the ring should not have to translate.
        ///
        /// Three kinds of failure in one enum, for `interface`'s reason — a
        /// canvas that cannot be asked what is in it is an escape, and *how* it
        /// got that way is an implementation detail of the mistake. The first
        /// kind is the floor: a declaration that says too little, which is what
        /// this module exists for. The second is the format: an entry that is
        /// not an entry. The third is the frame's order: a placement with no
        /// canvas open, or a second canvas in one frame.
        ///
        /// Every variant names the node it is about wherever there is one, so
        /// that the answer to *which* does not need a second pass.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Escape {
            $(
                $(#[$about])*
                $variant $(($payload))?,
            )*
        }

        impl Escape {
            /// How many ways there are to fail to participate.
            ///
            /// Counted from the list rather than written down, so it cannot
            /// disagree with what it counts.
            /// Unit: refusals.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every refusal's name, in declaration order.
            ///
            /// Emitted from the same list as the variants, so it holds every
            /// one there is — not because a test checks it, but because there
            /// is no way to write one this array does not get. It is what
            /// `an_escape_is_the_same_word_on_both_sides_of_the_ring` compares
            /// against `interface/src/canvas.rs`.
            pub const LABELS: [&'static str; Self::COUNT] = [$(stringify!($variant)),*];

            /// The refusal's own name, for a log or a completion trace.
            #[must_use]
            pub const fn label(self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => stringify!($variant),)*
                }
            }

            /// A line for a log.
            #[must_use]
            pub const fn message(self) -> &'static str {
                match self {
                    $(Self::$variant { .. } => $message,)*
                }
            }

            /// The refusal as a packed [`error`], for a completion.
            ///
            /// **No code here is new.** Every one is [`error::ARGUMENT`], which
            /// is the granularity decision written as a code: the channel is
            /// healthy, the peer is present, and what is wrong is one frame.
            /// Minting a code per floor clause was the obvious move and is the
            /// wrong one — sibling entry formats are being written against the
            /// same model, and a new `argument` code invented in each of four
            /// places is four meanings wearing one number. RFC 0010 says the
            /// domain is the stable part; the clauses are distinguishable as
            /// values, which is where a caller distinguishes them.
            ///
            /// The floor clauses share
            /// [`error::argument::UNKNOWN_FLAG`], which
            /// [`scene::Refusal`](crate::scene::Refusal),
            /// [`semantic::Refusal`](crate::semantic::Refusal) and
            /// [`store::refusal`](crate::store::refusal) already use for *a
            /// field carries a value this build does not accept*. A declaration
            /// that places nothing is exactly that, one clause up.
            #[must_use]
            pub const fn packed(self) -> i32 {
                match self {
                    $(Self::$variant { .. } => $code,)*
                }
            }
        }
    };
}

escapes! {
    /// A base of zero ticks a second. A dimension with no scale is not a
    /// dimension, and every number placed in it would mean nothing.
    NotATimeBase = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "a base of no ticks a second is not a dimension";

    /// An extent whose end does not follow its start. A point in the dimension
    /// is a marker, which the vocabulary already has a word for.
    NotAnExtent = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "an extent's end does not follow its start";

    /// More placements than [`PLACED_MAX`], from a peer or from the receiving
    /// tree — the two cases are one refusal because the arrangement that cannot
    /// hold them is the same arrangement.
    TooManyPlacements = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "more placements than one arrangement holds";

    /// One occupant placed twice. Two answers to *when is this* is no answer,
    /// and it is refused where it is written and again where it is read.
    PlacedTwice(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "one occupant is placed twice";

    /// The declaration names a node the receiving tree does not hold as a
    /// canvas.
    NotACanvas(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "the declaration names a node that is not a canvas of this tree";

    /// A selection confined to a lane that is not a lane of this canvas.
    /// Another canvas's lane is refused by this too: it is a lane, and it is
    /// not one this canvas may answer for.
    NotALane(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "a selection is confined to a lane this canvas does not hold";

    /// Something that is not a clip or a marker of this canvas, as the
    /// receiving tree holds it — named by a placement, or asked about
    /// afterwards. Both are one word here because on this side of the ring they
    /// are one fact: this module places everything it can answer about, so a
    /// node it cannot answer for is exactly a node it does not place.
    NotAnOccupant(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "something that is not an occupant of this canvas";

    /// A clip placed at an instant, or a marker placed over a span. A clip with
    /// no duration and a marker with one are both somebody meaning the other.
    Mistimed(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "an occupant is placed as the other kind of occupant";

    /// **The floor.** A clip or a marker the *receiving* tree declares under
    /// this canvas that the arriving arrangement places nowhere: the node
    /// exists, the pixels exist, and where it is did not arrive.
    Unplaced(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "an occupant of this canvas has no placement";

    /// **The floor's other half.** A canvas that places nothing at all, which
    /// meets the clause above by having nothing to meet it with. It is the
    /// cheapest escape on offer — declare the canvas, name the base, name the
    /// selection, send no placements, commit — and it is the one a peer reaches
    /// by doing nothing rather than by doing something wrong.
    NothingPlaced(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "the canvas places nothing at all";

    /// **The floor's scope.** The receiving tree holds a node under this canvas
    /// whose role the dimension cannot place. Without it a peer places one
    /// token clip and hangs the content the canvas actually draws off itself as
    /// a list, which the clause above never looks at.
    Unplaceable(u64) = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "a node under this canvas is of a role the dimension cannot place";

    /// A caller offered [`Arrangement::emit`] less room than the declaration
    /// needs. Refused rather than truncated, because a frame that stops halfway
    /// is a canvas that places fewer occupants than it has — which is the
    /// escape this module exists to refuse, produced by the encoder.
    NoRoom = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
        "a declaration was offered less room than it needs";

    /// The opcode is not one of this service's three.
    UnknownOpcode = error::pack(error::ARGUMENT, error::argument::UNKNOWN_OPCODE),
        "the opcode is not one of this service's three";

    /// The entry carries a submission flag outside [`FLAGS_ACCEPTED`].
    UnknownFlag = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "the entry carries a submission flag a participation entry may not";

    /// A field this opcode does not read is not zero: anywhere in the payload
    /// past what the record consumed, anywhere in the envelope this format
    /// never looks at — including a deadline, which no opcode here reads — or a
    /// field of a record that the record's own discriminant does not read.
    Reserved = error::pack(error::ARGUMENT, error::argument::RESERVED_NOT_ZERO),
        "a field this entry does not read is not zero";

    /// The entry does not frame something this format can read: a length that
    /// is not [`PAYLOAD_BYTES`], an arena offset that is not an arena offset,
    /// or a discriminant naming no case.
    Malformed = error::pack(error::ARGUMENT, error::argument::MALFORMED_HEADER),
        "the entry does not frame a payload this format can read";

    /// A field that must name a node holds [`NO_NODE`]. Separate from the
    /// clauses above because a zeroed payload produces exactly this, and a
    /// caller chasing a producer that forgot to fill a record wants to be told
    /// that rather than *some field is wrong*.
    NoNode = error::pack(error::ARGUMENT, error::argument::UNKNOWN_FLAG),
        "a field that must name a node names none";

    /// A placement or a commit arrived with no canvas declared.
    ///
    /// Not a courtesy. A placement with no canvas open is an occupant of
    /// nothing, and a receiver that remembered it until a canvas showed up
    /// would be assembling a declaration out of entries two different frames
    /// wrote.
    NotDeclared = error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
        "a placement arrived before any canvas was declared";

    /// A second canvas declared inside one participation frame.
    ///
    /// The commit is atomic over one canvas, and there is no entry in this
    /// format that could say which placements belonged to which canvas. The
    /// commit is the unit that may declare again.
    Redeclared(u64) = error::pack(error::ARGUMENT, error::argument::FEATURE_NOT_NEGOTIATED),
        "a second canvas was declared inside one frame";
}

/// A count of ticks in a canvas's declared [`Base`]. Signed, on purpose.
///
/// `interface`'s `Ticks`, restated for the wire. Signed because a count-in, a
/// pre-roll and a clip dragged left of the origin are all real, and a base that
/// cannot express time before zero forces the application to move the origin,
/// which changes every other number it has already declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Ticks(i64);

impl Ticks {
    /// The origin of the dimension: whatever the application decided zero is.
    pub const ORIGIN: Self = Self(0);

    /// A count of ticks.
    #[must_use]
    pub const fn new(count: i64) -> Self {
        Self(count)
    }

    /// The count, for whoever does the arithmetic this type deliberately does
    /// not.
    /// Unit: ticks of the declaring canvas's own base.
    #[must_use]
    pub const fn count(self) -> i64 {
        self.0
    }
}

/// What one tick is worth: the scale of a canvas's dimension.
///
/// Clause 2 of the floor, and it travels with the declaration because it is the
/// application's decision rather than this format's. The field is private and
/// the only constructor refuses zero, so a base that names nothing cannot be
/// built and cannot be decoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Base {
    /// How many ticks make one second.
    ticks_per_second: u32,
}

impl Base {
    /// 705 600 000 ticks a second: the base that names a frame and a sample
    /// exactly.
    ///
    /// The same number `interface`'s `TimeBase::FLICKS` declares, and held
    /// against it by `an_escape_is_the_same_word_on_both_sides_of_the_ring`.
    /// It is here because a wire fixture needs a base and a base picked at
    /// random would be a base under which the round trip proves less: at this
    /// one every common frame rate and every common audio rate divides
    /// exactly, so a tick count that crosses and comes back is a boundary that
    /// crossed and came back.
    ///
    /// Nothing in this module converts a tick to a second. A base that divides
    /// is the application's problem and `interface/src/canvas.rs` is where it
    /// is argued; the wire's job is to carry the pair without rounding either
    /// half.
    /// Unit: ticks per second.
    pub const FLICKS: Self = Self { ticks_per_second: 705_600_000 };

    /// A base of `ticks_per_second`.
    ///
    /// # Errors
    ///
    /// [`Escape::NotATimeBase`] for zero.
    pub const fn new(ticks_per_second: u32) -> Result<Self, Escape> {
        if ticks_per_second == 0 {
            return Err(Escape::NotATimeBase);
        }
        Ok(Self { ticks_per_second })
    }

    /// How many ticks make one second.
    /// Unit: ticks per second, and never zero.
    #[must_use]
    pub const fn ticks_per_second(self) -> u32 {
        self.ticks_per_second
    }
}

/// A stretch of the dimension: from a tick, up to but not including another.
///
/// Half-open, for `interface`'s reason: a clip ending at 4.2 s and one starting
/// at 4.2 s do not overlap, and the sample at exactly 4.2 s belongs to the
/// second — the only convention under which butt-joined clips are neither
/// double-counted nor separated by a gap of one tick that nobody declared. Both
/// predicates below read that way, and a reader who changes one has to change
/// the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Extent {
    /// The first tick it occupies.
    start: Ticks,
    /// The first tick after it.
    end: Ticks,
}

impl Extent {
    /// A stretch from `start` up to `end`.
    ///
    /// # Errors
    ///
    /// [`Escape::NotAnExtent`] when `end` does not come after `start`. An
    /// occupant with no duration is a marker, and a zero-length clip is
    /// somebody who did not reach for that word.
    pub const fn new(start: Ticks, end: Ticks) -> Result<Self, Escape> {
        if end.0 <= start.0 {
            return Err(Escape::NotAnExtent);
        }
        Ok(Self { start, end })
    }

    /// The first tick it occupies.
    #[must_use]
    pub const fn start(self) -> Ticks {
        self.start
    }

    /// The first tick after it.
    #[must_use]
    pub const fn end(self) -> Ticks {
        self.end
    }

    /// Is `instant` inside it? Half-open: the end is not.
    #[must_use]
    pub const fn contains(self, instant: Ticks) -> bool {
        self.start.0 <= instant.0 && instant.0 < self.end.0
    }

    /// Do the two share any tick at all?
    #[must_use]
    pub const fn overlaps(self, other: Self) -> bool {
        self.start.0 < other.end.0 && other.start.0 < self.end.0
    }
}

/// Where an occupant sits in the dimension.
///
/// Two variants, because the vocabulary has two kinds of occupant and they are
/// not the same shape: a clip fills a stretch and a marker names an instant.
/// [`Arrangement::admit`] refuses each placed as the other against the
/// receiving tree's own reading of which is which, which is
/// [`Escape::Mistimed`] — not pedantry, since *what is at 5 s* has a different
/// answer depending on which the author meant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Span {
    /// A point: a playhead, a cue, a bookmark.
    At(Ticks),
    /// A stretch: a clip.
    Between(Extent),
}

impl Span {
    /// Does this sit anywhere inside `extent`?
    ///
    /// Both kinds answer, which is why it is a method rather than a `match`
    /// written out at each call site: an instant inside a stretch is inside it.
    /// `interface`'s first version had a `let … else` at one call site that
    /// answered *no* for every marker without saying so, and a predicate
    /// written once cannot disagree with itself in one place.
    #[must_use]
    pub const fn touches(self, extent: Extent) -> bool {
        match self {
            Self::At(instant) => extent.contains(instant),
            Self::Between(own) => own.overlaps(extent),
        }
    }
}

/// One occupant, and where it sits — the pair this format exists to carry.
///
/// # Why it does not say which lane
///
/// Because the tree already does, and the tree crossed first. A clip's lane is
/// its node's parent, declared by
/// [`semantic::DeclareNode`](crate::semantic::DeclareNode), and restating it
/// here would be a second copy of a fact that can disagree with the first. The
/// join is the node identifier and it goes both ways: this says *when*, the
/// tree says *what* and *where among the lanes*, and neither restates the
/// other. It is also what makes the floor checkable at all — the receiver holds
/// the tree, so it can tell which occupants should have arrived.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Which node. The submitter's own identifier, as
    /// [`semantic::DeclareNode::node`](crate::semantic::DeclareNode::node)
    /// declared it, and it must be a clip or a marker of this canvas in the
    /// receiving tree.
    /// Unit: none — a node identifier, not a quantity. [`NO_NODE`] is refused.
    pub occupant: u64,
    /// Where it sits in the dimension.
    /// Unit: none here, deliberately — [`Span`] carries its own, as [`Ticks`]
    /// counted from whatever the application declared its origin to be. A number
    /// at this level would be a second opinion about what a tick is, and the two
    /// would disagree the first time a timeline changed its base.
    pub span: Span,
}

impl Placement {
    /// A clip, filling `extent`.
    #[must_use]
    pub const fn over(occupant: u64, extent: Extent) -> Self {
        Self { occupant, span: Span::Between(extent) }
    }

    /// A marker, naming `instant`.
    #[must_use]
    pub const fn at(occupant: u64, instant: Ticks) -> Self {
        Self { occupant, span: Span::At(instant) }
    }
}

/// What is selected: a stretch of the dimension, optionally confined to one
/// lane.
///
/// Clause 3 of the floor. Not a set of node identities, for `interface`'s
/// reason: a set of identities cannot express a selection over a stretch with
/// nothing in it, and a ripple delete over an empty stretch is a real operation
/// every timeline has. It is also not the same fact as a node's own selected
/// state, which crosses on the semantic channel as a state bit — *this clip is
/// selected* is a property of a clip, and *4.2 s to 6.8 s on this lane is
/// selected* is a property of the dimension.
///
/// One stretch, not several, for version 1. RFC 0078 carries the reversal
/// condition: the first application whose selection cannot be declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The stretch, if anything is selected.
    span: Option<Extent>,
    /// The lane it is confined to, if it is confined to one.
    lane: Option<u64>,
}

impl Selection {
    /// Nothing is selected — which is a declaration, and is the thing a canvas
    /// that says nothing at all has failed to make.
    pub const NOTHING: Self = Self { span: None, lane: None };

    /// This stretch, across every lane.
    #[must_use]
    pub const fn over(span: Extent) -> Self {
        Self { span: Some(span), lane: None }
    }

    /// The same, confined to one lane.
    #[must_use]
    pub const fn on(self, lane: u64) -> Self {
        Self { span: self.span, lane: Some(lane) }
    }

    /// The selected stretch, if there is one.
    #[must_use]
    pub const fn span(self) -> Option<Extent> {
        self.span
    }

    /// The lane it is confined to, if it is.
    /// Unit: none — a node identifier.
    #[must_use]
    pub const fn lane(self) -> Option<u64> {
        self.lane
    }

    /// Is nothing selected?
    #[must_use]
    pub const fn is_nothing(self) -> bool {
        self.span.is_none()
    }
}

/// What kind of occupant a node is, as the receiving tree reads it.
///
/// Two, because the dimension has two shapes to put something in. It is the
/// tree's answer and not the wire's: the arriving arrangement says a node is
/// placed over a stretch, the tree says that node is a marker, and
/// [`Escape::Mistimed`] is the two disagreeing. A format that took the peer's
/// word for which kind a node is would be letting the peer decide what its own
/// declaration means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// A clip: it fills a stretch of the dimension.
    Clip,
    /// A marker: it names an instant in it.
    Marker,
}

/// One occupant the receiving tree declares under a canvas.
///
/// Three facts, and the third is the one a wire cannot work out for itself. A
/// placement says *when*; the tree says *what* and *where among the lanes*, and
/// [`Placement`] deliberately carries neither. So the census is where the lane
/// crosses from the tree into this module, once, at the moment the floor is
/// checked — which is also the only moment at which it is certainly the lane
/// the receiver is about to draw the clip on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Occupant {
    /// Which node.
    /// Unit: none — a node identifier, as the tree holds it.
    pub node: u64,
    /// What shape of placement it takes.
    /// Unit: none — a discriminant, not a quantity.
    pub kind: Kind,
    /// The lane it sits on, or [`NO_NODE`] for one that sits on the canvas
    /// itself.
    ///
    /// The vocabulary lets a marker hang off the canvas rather than off a lane
    /// — a sequence-wide cue is not a cue *on track 3* — so *no lane* is a real
    /// answer about a real occupant rather than a missing one, and it is
    /// spelled with the constant that already means *nobody* everywhere else in
    /// this crate.
    /// Unit: none — a node identifier.
    pub lane: u64,
}

impl Occupant {
    /// A slot nobody filled: what a caller's census buffer holds before
    /// [`Presented::census`] writes into it.
    ///
    /// Its kind is [`Kind::Marker`] because there is no *no kind* and this
    /// module will not invent one for a filler — the honest reading is that the
    /// slots past the count a census returns are not occupants at all, and a
    /// reader of one is reading past the end of the answer. Its node is
    /// [`NO_NODE`], so a census that returned a count larger than what it wrote
    /// names nothing rather than naming something plausible.
    pub const UNDECLARED: Self = Self { node: NO_NODE, kind: Kind::Marker, lane: NO_NODE };
}

/// What the receiving tree says about a canvas.
///
/// A closed set of facts, and every arm of it is a named [`Escape`]. That shape
/// is the whole reason this is an enum rather than a handful of predicates: an
/// implementor states what its tree holds and decides nothing about what
/// happens next, so there is no place for a tree-owner to be lenient. The one
/// judgement it makes — *is this node a canvas at all* — is the one only it can
/// make.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Census {
    /// Not a canvas of this tree. Clause 1 of the floor, answered by the only
    /// party that can.
    NotACanvas,
    /// A canvas, and this node under it is of a role the dimension cannot
    /// place: anything that is not a lane, a clip or a marker.
    Unplaceable(u64),
    /// A canvas with more occupants than one arrangement can carry. The tree is
    /// not wrong; the frame is too small for it, and RFC 0078 says a canvas
    /// with more content than fits must declare the window it is showing.
    Crowded,
    /// A canvas, and this many occupants were written into the caller's buffer,
    /// in the tree's own declaration order.
    /// Unit: occupants, and never above [`PLACED_MAX`].
    Occupies(usize),
}

/// The tree the receiver is already presenting, asked one question at a time.
///
/// # Why a trait and not a tree
///
/// Because this crate cannot hold one. `interface/` is a leaf that nothing
/// depends on, this crate sits beneath the frame, and neither can see the
/// other — the argument is [`semantic`](crate::semantic)'s and it is the same
/// argument. The component that owns the tree implements this, because it is
/// the only party that has both the tree and the arriving bytes.
///
/// # What an implementor owes
///
/// - **Derive the answer, never write it.** A census assembled from a list
///   somebody typed is the second place a canvas's content is written, and it
///   will disagree with the first. Walk the tree.
/// - **Answer about the tree as it stands, not as it is about to be.** The
///   floor is a statement about what the receiver would present *now*; a census
///   that anticipated the frame being assembled would be checking the
///   declaration against itself.
/// - **Be a pure function of its arguments.** No clock, no randomness, no state
///   a second call could change. RFC 0004, and also why this is a parameter
///   rather than a global: a decoder that consulted something could be made to
///   admit one frame two ways.
///
/// # Why an implementor cannot be policed here
///
/// Stated rather than left for a reviewer to find: an implementation that
/// reports [`Census::Occupies`] with a count of zero for every canvas defeats
/// the floor, and nothing in this module can detect that. What this module
/// guarantees is narrower and is what the exit asks for — *this* crate never
/// produces an answer about a canvas out of placements that did not arrive, it
/// has no fallback to fall back to, and the party that would have to write the
/// lying census is the same party that would have to render the result. The
/// floor's teeth are that the refusal happens where the rendering happens.
pub trait Presented {
    /// What this tree holds under `canvas`, with the occupants written into
    /// `into` in the tree's own declaration order.
    ///
    /// The buffer is a whole [`PLACED_MAX`] array rather than a slice so that
    /// an implementor cannot be handed less room than an arrangement can place
    /// and have to decide what to do about it. Running out of room is
    /// [`Census::Crowded`], which is a fact about the tree rather than about
    /// the buffer.
    fn census(&self, canvas: u64, into: &mut [Occupant; PLACED_MAX]) -> Census;

    /// Is `lane` a lane of `canvas` in this tree?
    ///
    /// Asked only of a selection that confines itself to one, and answered
    /// `false` for another canvas's lane as much as for a stranger: it is a
    /// lane, and it is not one this canvas may answer for.
    fn lane_of(&self, canvas: u64, lane: u64) -> bool;
}

/// What a canvas claims about its content: the three clauses a declaration can
/// state on its own, and the placements the fourth is checked against.
///
/// This type is a *claim*, not an admission. It says which canvas, in what
/// dimension, with what selected, and where each occupant it knows about sits —
/// and none of that has been checked against a tree. [`admit`](Self::admit) is
/// where it meets one, and [`Participation`] is what comes back.
///
/// Both sides of the ring hold one. The sender builds it and
/// [`emit`](Self::emit)s it; the receiver assembles it out of arriving entries
/// and admits it. That is deliberate rather than convenient: the two refusals a
/// builder makes — an occupant placed twice, more placements than fit — are
/// then the same code on both sides, so a peer cannot state something its own
/// author's build would have refused.
///
/// # What it deliberately cannot answer
///
/// *Where does this occupant sit.* There is no `placement_of` here, no
/// `placements` iterator, and no indexing. Those live on [`Participation`], and
/// the separation is the exit this module is accepted on: a receiver holding an
/// arrangement whose placements did not arrive can ask it nothing that would
/// let it draw a timeline. What it can ask is how many placements it has, which
/// is a fact about the frame it is assembling rather than an answer about a
/// canvas.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrangement {
    /// Clause 1: the canvas this is about.
    canvas: u64,
    /// Clause 2: what one tick of the dimension is worth.
    base: Base,
    /// Clause 3: what is selected in it.
    selection: Selection,
    /// Clause 4's raw material. Slots past `len` are never read.
    placed: [Option<Placement>; PLACED_MAX],
    /// How many of them are real.
    len: u8,
}

impl Arrangement {
    /// Declare a canvas: which one, in what dimension, with what selected.
    ///
    /// There is no constructor that omits any of the three and no `Default`,
    /// which is the floor being structural rather than advisory on this side
    /// too. Clause 4 cannot be stated here because it is a statement about this
    /// declaration *and* a tree — including the half this constructor could
    /// have checked and deliberately does not, that something is placed at all,
    /// since a declaration is assembled one placement at a time and begins
    /// empty.
    #[must_use]
    pub const fn declaring(canvas: u64, base: Base, selection: Selection) -> Self {
        Self { canvas, base, selection, placed: [None; PLACED_MAX], len: 0 }
    }

    /// The same declaration, with one more occupant placed.
    ///
    /// # Errors
    ///
    /// [`Escape::TooManyPlacements`] past [`PLACED_MAX`], and
    /// [`Escape::PlacedTwice`] for an occupant this arrangement already places.
    /// Both are refused here, which is where the author is on the sending side
    /// and where the peer's bytes are on the receiving one.
    pub const fn with_placement(mut self, placement: Placement) -> Result<Self, Escape> {
        if self.len as usize >= PLACED_MAX {
            return Err(Escape::TooManyPlacements);
        }
        let mut at = 0;
        while at < self.len as usize {
            if let Some(seen) = self.placed[at]
                && seen.occupant == placement.occupant
            {
                return Err(Escape::PlacedTwice(placement.occupant));
            }
            at += 1;
        }
        self.placed[self.len as usize] = Some(placement);
        self.len += 1;
        Ok(self)
    }

    /// The same declaration, selecting something else.
    ///
    /// A builder rather than a setter because the type is a value: two
    /// selections are two declarations, and a projection holding the old one
    /// still holds something true about the frame it came from.
    #[must_use]
    pub const fn with_selection(self, selection: Selection) -> Self {
        Self { canvas: self.canvas, base: self.base, selection, placed: self.placed, len: self.len }
    }

    /// The canvas this is about.
    /// Unit: none — a node identifier.
    #[must_use]
    pub const fn canvas(&self) -> u64 {
        self.canvas
    }

    /// What one tick is worth here.
    #[must_use]
    pub const fn base(&self) -> Base {
        self.base
    }

    /// What is selected.
    #[must_use]
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// How many occupants it places.
    ///
    /// A count of the frame, not an answer about the canvas: it says how much
    /// arrived, which is what a receiver assembling a frame legitimately knows
    /// before the floor is met. *Which* occupants, and where, is
    /// [`Participation`]'s to answer and only past the floor.
    /// Unit: placements.
    #[must_use]
    pub const fn placed(&self) -> usize {
        self.len as usize
    }

    /// Where this arrangement puts `occupant`, if it puts it anywhere.
    ///
    /// **Private, and the privacy is the whole mechanism.** This is the
    /// question a renderer asks, and answering it about a declaration nobody
    /// admitted is the escape section 13 is about. [`Participation`] publishes
    /// it, and there is no way to hold a [`Participation`] except past
    /// [`admit`](Self::admit).
    fn placement_of(&self, occupant: u64) -> Option<Placement> {
        self.placements().find(|placement| placement.occupant == occupant)
    }

    /// Every placement, in arrival order. Private, for
    /// [`placement_of`](Self::placement_of)'s reason.
    fn placements(&self) -> impl Iterator<Item = Placement> + '_ {
        self.placed[..self.len as usize].iter().copied().flatten()
    }

    /// Meet the receiving tree, and be admitted as a participant or refused as
    /// an escape.
    ///
    /// **This is the far side of the floor.** It is the same four clauses
    /// `interface`'s `Arrangement::admit` checks, asked of the tree the
    /// receiver is presenting rather than of the tree the sender declared
    /// against, and it is what stops a peer escaping RFC 0078 by never calling
    /// a constructor it does not have.
    ///
    /// It checks, in order: what the tree says about the canvas as a whole;
    /// that a confined selection names a lane of it; that every placement names
    /// an occupant of it, placed as the tree says its kind permits; and finally
    /// the clause that matters — that **something is placed, and every clip and
    /// every marker the receiving tree declares under this canvas is placed**.
    ///
    /// # Why the canvas comes before the placements
    ///
    /// `interface`'s order is the declaration's: a placement naming a stranger
    /// is refused before the subtree's scope is walked. This one asks the tree
    /// one question, once, and that question answers three clauses at once, so
    /// what is wrong with the *canvas* is reported before what is wrong with a
    /// *placement*. The two orders differ only for a declaration with two
    /// faults at once, which is `interface`'s own words for a declaration
    /// somebody is still writing; both refuse, and neither admits.
    ///
    /// # Errors
    ///
    /// The first [`Escape`] found. A declaration with two faults is one
    /// somebody is still writing.
    ///
    /// # Why this scans rather than indexes
    ///
    /// Because this crate has no hash set — RFC 0004 — and because a frame is
    /// sixty-four placements at most. Sixty-four times sixty-four is the whole
    /// cost, once per commit, in the one place a canvas is decided.
    pub fn admit(&self, tree: &dyn Presented) -> Result<Participation, Escape> {
        let mut occupants = [Occupant::UNDECLARED; PLACED_MAX];
        let declared = match tree.census(self.canvas, &mut occupants) {
            Census::NotACanvas => return Err(Escape::NotACanvas(self.canvas)),
            Census::Unplaceable(node) => return Err(Escape::Unplaceable(node)),
            Census::Crowded => return Err(Escape::TooManyPlacements),
            // A census that counts past the buffer it was handed is an
            // implementor that miscounted, and the slots it did not write hold
            // `NO_NODE`. Refusing is the only answer that does not invent an
            // occupant: `Crowded` is the word for a tree with more content than
            // fits, and this is that fact arriving by the other route.
            Census::Occupies(count) if count > PLACED_MAX => {
                return Err(Escape::TooManyPlacements);
            }
            Census::Occupies(count) => count,
        };
        let occupants = &occupants[..declared];

        if let Some(lane) = self.selection.lane
            && !tree.lane_of(self.canvas, lane)
        {
            return Err(Escape::NotALane(lane));
        }

        for placement in self.placements() {
            let Some(occupant) =
                occupants.iter().find(|occupant| occupant.node == placement.occupant)
            else {
                return Err(Escape::NotAnOccupant(placement.occupant));
            };
            let agrees = matches!(
                (occupant.kind, placement.span),
                (Kind::Clip, Span::Between(_)) | (Kind::Marker, Span::At(_))
            );
            if !agrees {
                return Err(Escape::Mistimed(placement.occupant));
            }
        }

        // The floor, first half. A declaration that places nothing complies
        // with the loop below by giving it nothing to object to, and that is
        // the cheapest escape a peer has: declare the canvas, name the base,
        // name the selection, send no placements, commit. It is refused before
        // the loop rather than inside it, because there is no one node to name
        // — the canvas is the thing that said nothing.
        if self.len == 0 {
            return Err(Escape::NothingPlaced(self.canvas));
        }

        // The floor, second half, and the sentence this module is accepted on.
        // Everything above refuses a frame that says something wrong; this
        // refuses one that does not say enough, which is the failure section 13
        // is actually about and the one no amount of well-formed bytes prevents.
        for occupant in occupants {
            if self.placement_of(occupant.node).is_none() {
                return Err(Escape::Unplaced(occupant.node));
            }
        }

        let mut held = [Occupant::UNDECLARED; PLACED_MAX];
        held[..declared].copy_from_slice(occupants);
        Ok(Participation { arrangement: *self, held, declared: declared as u8 })
    }

    /// Write the entries that carry this declaration into `into`, and answer
    /// how many that took.
    ///
    /// The whole frame: the declaration, one entry per placement, and the
    /// commit. It is all-or-nothing about room — see [`Escape::NoRoom`] — for
    /// the reason the floor exists: a frame that stopped halfway would be a
    /// canvas that places fewer occupants than it has, which is the escape this
    /// module refuses, produced by the encoder rather than by a peer.
    ///
    /// Each entry's payload sits one [`PAYLOAD_BYTES`] stride past the last,
    /// starting at `carriage.payload_offset`, because that is how a producer
    /// that filled an arena laid them out.
    ///
    /// # Errors
    ///
    /// [`Escape::NoRoom`] when `into` is shorter than
    /// [`Arrangement::placed`] plus two; [`Escape::UnknownFlag`] for a flag
    /// outside [`FLAGS_ACCEPTED`]; [`Escape::Malformed`] when the arena offsets
    /// would run past what the field holds.
    pub fn emit(&self, carriage: Carriage, into: &mut [Submission]) -> Result<usize, Escape> {
        envelope_rules(op::DECLARE_CANVAS, carriage.flags)?;
        let wanted = self.len as usize + 2;
        if into.len() < wanted {
            return Err(Escape::NoRoom);
        }

        let mut at = 0;
        let mut write = |body: Entry, into: &mut [Submission]| -> Result<(), Escape> {
            let stride = u32::try_from(at * PAYLOAD_BYTES).map_err(|_| Escape::Malformed)?;
            let offset = carriage.payload_offset.checked_add(stride).ok_or(Escape::Malformed)?;
            let crossing = Crossing {
                user_data: carriage.user_data,
                class: carriage.class,
                payload_offset: offset,
                flags: carriage.flags,
                body,
            };
            let (entry, payload) = crossing.encode();
            into[at] = Submission { entry, payload };
            at += 1;
            Ok(())
        };

        write(
            Entry::Declaring(Declaring {
                canvas: self.canvas,
                base: self.base,
                selection: self.selection,
            }),
            into,
        )?;
        for placement in self.placements() {
            write(Entry::Place(Place { placement }), into)?;
        }
        write(Entry::Participate(Participate), into)?;
        Ok(at)
    }
}

/// A canvas that has met the floor on the receiving side, and the only thing in
/// this module that can be asked where anything is.
///
/// Holding one is a proof rather than a convenience: it cannot be built except
/// by [`Arrangement::admit`], and the only public route to that is
/// [`Reception::accept`] returning [`Progress::Participating`]. That is the
/// whole mechanism by which *a canvas whose placements did not arrive is
/// refused rather than rendered* is a property of the types rather than a
/// sentence in a protocol document.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Participation {
    /// The declaration, admitted against the receiving tree.
    arrangement: Arrangement,
    /// What the receiving tree said was under the canvas, as it said it. Slots
    /// past `declared` are never read.
    ///
    /// Kept rather than discarded because two of the questions below are the
    /// tree's to answer and not the declaration's — which lane an occupant is
    /// on, and therefore what a lane-confined selection touches. Asking the
    /// tree again later would be asking a tree that has since moved; this is
    /// what it said at the moment the floor was met, which is the moment this
    /// canvas became presentable.
    held: [Occupant; PLACED_MAX],
    /// How many of them are real. Never zero: the floor is why.
    declared: u8,
}

impl Participation {
    /// The canvas this is about.
    /// Unit: none — a node identifier.
    #[must_use]
    pub const fn canvas(&self) -> u64 {
        self.arrangement.canvas
    }

    /// What one tick is worth here.
    #[must_use]
    pub const fn base(&self) -> Base {
        self.arrangement.base
    }

    /// What is selected.
    #[must_use]
    pub const fn selection(&self) -> Selection {
        self.arrangement.selection
    }

    /// How many occupants it places. Never zero: the floor is why.
    /// Unit: placements.
    #[must_use]
    pub const fn placed(&self) -> usize {
        self.arrangement.len as usize
    }

    /// Every placement, in arrival order.
    pub fn placements(&self) -> impl Iterator<Item = Placement> + '_ {
        self.arrangement.placements()
    }

    /// Where this canvas puts `occupant`.
    ///
    /// The question section 13 asks — *can an agent select the clip between
    /// 4.2 s and 6.8 s on track 3 without seeing a single pixel* — reaching
    /// across a ring. `None` is a real answer about a node this canvas does not
    /// place, and the floor is what makes it a narrow one: every clip and
    /// marker the receiving tree holds under this canvas has a placement, so
    /// the only nodes that answer `None` are nodes of somewhere else.
    #[must_use]
    pub fn placement_of(&self, occupant: u64) -> Option<Placement> {
        self.arrangement.placement_of(occupant)
    }

    /// Which lane an occupant of this canvas sits on, or `Ok(None)` for one
    /// that sits on the canvas itself.
    ///
    /// The other half of section 13's sentence — *the clip between 4.2 s and
    /// 6.8 s **on track 3*** — and it is answered from what the receiving tree
    /// said when the floor was met, because the lane is the tree's fact and
    /// [`Placement`] deliberately does not restate it.
    ///
    /// # Errors
    ///
    /// [`Escape::NotAnOccupant`] for a node this canvas does not place. The two
    /// answers that separates are *it sits on the canvas itself* and *it is not
    /// mine to say about*, which an `Option` would spell alike — and a caller
    /// that read the second as the first would place a stranger's marker on
    /// this canvas's timeline.
    pub fn lane_of(&self, occupant: u64) -> Result<Option<u64>, Escape> {
        match self.held[..self.declared as usize].iter().find(|held| held.node == occupant) {
            Some(held) if held.lane == NO_NODE => Ok(None),
            Some(held) => Ok(Some(held.lane)),
            None => Err(Escape::NotAnOccupant(occupant)),
        }
    }

    /// Which occupants the declared selection touches.
    ///
    /// The join an agent uses: select a stretch, learn an identity, invoke the
    /// capability that identity carries — the capability being the tree's, on
    /// the semantic channel, which is why nothing here resolves one. Empty when
    /// nothing is selected, and empty when the selected stretch is empty, which
    /// are different facts that [`Selection::is_nothing`] tells apart.
    ///
    /// Markers are included, because a marker is placed in the dimension and a
    /// selection is a stretch of it: a cue at 5 s is inside a selection of
    /// 4.2 s to 6.8 s by every reading an editor has.
    pub fn selected(&self) -> impl Iterator<Item = u64> + '_ {
        self.arrangement
            .placements()
            .filter(move |placement| self.within_selection(*placement))
            .map(|placement| placement.occupant)
    }

    /// Does the declared selection touch this placement?
    ///
    /// Both halves, and the lane half is the reason this type keeps the census.
    /// A selection confined to a lane covers the stretch *on that lane*, so a
    /// clip on another lane at the same time is not in it — and answering that
    /// needs the lane, which the wire does not carry and the tree does. An
    /// earlier version of this method had no census to consult and answered
    /// from the stretch alone, which made a confined selection report every
    /// lane's clips: a ripple delete on track 3 that took track 1 with it.
    fn within_selection(&self, placement: Placement) -> bool {
        let Some(span) = self.arrangement.selection.span else { return false };
        if !placement.span.touches(span) {
            return false;
        }
        match self.arrangement.selection.lane {
            Some(lane) => self.lane_of(placement.occupant) == Ok(Some(lane)),
            None => true,
        }
    }
}

/// What a receiver learned from one entry.
///
/// **Two variants, and one of them carries nothing.** That is not an oversight
/// and it is the exit this module is accepted on: until the floor is met there
/// is no value in this module that says where anything is, so a receiver that
/// wanted to render a half-arrived canvas has nothing to render it from. A
/// third variant carrying *the placement that just arrived* would be the escape
/// hatch, and the escape hatch is never a feature anybody asks for — it is the
/// shape of what is left when a deadline removes everything that was not
/// enforced.
///
/// # Why one variant is two kilobytes and the other is nothing
///
/// Because that asymmetry is the decision rather than a layout accident, and
/// the two ways out of it are both worse. A [`Participation`] is an
/// [`Arrangement`] — sixty-four placements of thirty-two bytes, the size
/// `interface`'s own declaration already is — and it is `Copy` for
/// [`Arrangement`]'s reason: it is optimised for being read and argued with
/// rather than for being sent onward. Boxing the large variant needs an
/// allocator this crate does not have and never will. Handing back a reference
/// into the [`Reception`] instead would borrow it for as long as the caller is
/// presenting the canvas, which forbids reading the next entry while drawing
/// this frame — a worse contract than one copy of two kilobytes, made once per
/// commit, on the path that just walked the whole arrangement twice.
#[expect(
    clippy::large_enum_variant,
    reason = "no allocator to box with, and a borrow would forbid reading the next entry \
              while this frame is being presented; see the type's own note"
)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Progress {
    /// The entry was believed and folded into the canvas being assembled.
    /// Nothing is presentable yet, and nothing is handed back.
    Assembling,
    /// The frame closed and the floor was met.
    Participating(Participation),
}

/// Where the offending entry was, and why.
///
/// The position and not the arena offset, because the sender built the frame as
/// a sequence and that is the coordinate its own code is indexed by. A producer
/// that has to map an arena offset back to the entry it wrote is a producer
/// that will guess. The node, where there is one, is inside the [`Escape`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fault {
    /// Why the entry was not believed.
    /// Unit: none — a refusal, not a quantity. Which entry it was about is
    /// `position`, one field down, and that one is a count.
    pub escape: Escape,
    /// Which entry of this frame it was, counting from zero at the last commit.
    /// Unit: entries since the last commit.
    pub at: u32,
}

/// What a producer chooses about how its entries ride.
///
/// The three fields of an [`Sqe`] this format does not decide for the caller,
/// gathered so that [`Arrangement::emit`] takes one argument rather than four
/// and so that adding a fourth is a diff to a named type. Everything else in
/// the envelope is either written by this format or must arrive zero; there is
/// no third category.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Carriage {
    /// Returned verbatim in each entry's completion, and opaque here.
    /// Unit: none — chosen by the submitter and never interpreted.
    pub user_data: u64,
    /// The scheduling class, and the depth its urgency has already crossed.
    /// Read on every opcode because it is the ring's field and not this
    /// format's.
    /// Unit: none — a class ordinal in the low byte, a depth in the high one.
    pub class: u16,
    /// Where the first entry's payload sits; each later one is one
    /// [`PAYLOAD_BYTES`] stride past the last.
    /// Unit: bytes from the first byte of the channel's inline arena.
    pub payload_offset: u32,
    /// Submission flags.
    /// Unit: none — a bitmask, and a subset of [`FLAGS_ACCEPTED`].
    pub flags: u8,
}

/// One entry as it goes on the wire: the envelope, and the payload it points
/// at.
///
/// What [`Arrangement::emit`] produces and what a submitter copies into the
/// ring and the arena. It carries no decoded body, and that absence is
/// deliberate: an encoded frame that could be read back as structure would be a
/// second route to *where does this occupant sit*, reachable without the floor.
/// Reading one back means decoding it, and decoding means [`Reception`].
#[derive(Clone, Copy, Debug)]
pub struct Submission {
    /// The submission entry.
    /// Unit: none — a ring entry, not a quantity. The quantities inside it state
    /// their own; this field is the envelope that carries them.
    pub entry: Sqe,
    /// The arena bytes it points at.
    /// Unit: bytes, `PAYLOAD_BYTES` of them. The stride of an arena slot and not
    /// the length of what was written — a reader takes the length from the entry,
    /// because a payload that stated its own length would be a second opinion
    /// about where the body ends.
    pub payload: [u8; PAYLOAD_BYTES],
}

impl Submission {
    /// An entry nobody wrote: what a caller fills a buffer with before handing
    /// it to [`Arrangement::emit`].
    ///
    /// The slots past what `emit` returns are not entries at all, and a caller
    /// that submits one is submitting a zeroed envelope, whose opcode is no
    /// opcode and which any conforming receiver refuses.
    pub const UNSENT: Self = Self { entry: Sqe::ZERO, payload: [0; PAYLOAD_BYTES] };
}

/// Writes a record's fields into a payload, counting as it goes.
///
/// Little-endian and by hand, [`store`](crate::store)'s discipline and
/// [`semantic`](crate::semantic)'s: the layout is the encoding rather than the
/// compiler's opinion of a struct, so nothing here depends on the host's word
/// size, its alignment rules or its byte order. Every slot the writer does not
/// reach stays zero, which is what makes a record's tail the same bytes on both
/// sides of the wire.
struct Writer<'a> {
    /// The payload being filled.
    out: &'a mut [u8; PAYLOAD_BYTES],
    /// How much of it has been written.
    at: usize,
}

impl Writer<'_> {
    /// Append one byte.
    fn u8(&mut self, value: u8) {
        self.put(&[value]);
    }

    /// Append four bytes.
    fn u32(&mut self, value: u32) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes.
    fn u64(&mut self, value: u64) {
        self.put(&value.to_le_bytes());
    }

    /// Append eight bytes of a signed count. Two's complement, little-endian,
    /// which is what `to_le_bytes` is on every target this tree builds for and
    /// is stated here because a tick before the origin is an ordinary value and
    /// not an edge case.
    fn i64(&mut self, value: i64) {
        self.put(&value.to_le_bytes());
    }

    /// The one place bytes land.
    ///
    /// Indexing rather than a checked write, and the bound is a compile-time
    /// fact rather than a runtime hope: every record asserts
    /// `WIDTH <= PAYLOAD_BYTES` at the foot of this file, and
    /// `no_record_writes_past_its_declared_width` requires every record's
    /// writer to stop at its own `WIDTH`.
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
    /// Take one byte.
    fn u8(&mut self) -> u8 {
        let at = self.take(1);
        self.raw[at]
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

    /// Take eight bytes of a signed count.
    fn i64(&mut self) -> i64 {
        self.u64() as i64
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
    fn finish(self) -> Result<(), Escape> {
        let mut at = self.at;
        while at < PAYLOAD_BYTES {
            if self.raw[at] != 0 {
                return Err(Escape::Reserved);
            }
            at += 1;
        }
        Ok(())
    }
}

/// The discriminant of a [`Span`] or of a [`Selection`]'s stretch, on the wire.
///
/// One byte, two values, and the values are this format's own rather than an
/// ordinal of anything `interface` declares — which is why this format needs no
/// handshake. A third shape of placement is an ABI change and is a diff to this
/// list.
mod shape {
    /// A [`super::Span::At`], or a [`super::Selection`] that selects nothing.
    /// Zero, so that a zeroed payload names the case with no extent in it.
    pub const POINT: u8 = 0;
    /// A [`super::Span::Between`], or a selection over a stretch.
    pub const STRETCH: u8 = 1;
}

/// What every opcode's record can do.
///
/// Private, because nothing outside this module encodes a payload without the
/// envelope that frames it. Its value is what it makes obligatory: a record
/// cannot exist without a width, a specimen, a writer and a reader, so a fourth
/// opcode cannot be added without all four, and [`Entry::specimens`] picks the
/// specimen up without anybody remembering to add it to a test.
trait Record: Copy + Sized {
    /// The bytes this record's fields occupy, before the padding to
    /// [`PAYLOAD_BYTES`].
    /// Unit: bytes.
    const WIDTH: usize;

    /// One value of this record that is legal on the wire.
    ///
    /// Not a default and not an empty value: every field holds something this
    /// format accepts, so that decoding it succeeds and the round-trip test has
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
    /// An [`Escape`] naming the disbelief. The tail is not this method's
    /// business: [`Record::from_payload`] holds that rule for every record.
    fn read(raw: &mut Reader) -> Result<Self, Escape>;

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
    /// An [`Escape`] from the record's own fields, or [`Escape::Reserved`] for
    /// a byte past them.
    fn from_payload(raw: &[u8; PAYLOAD_BYTES]) -> Result<Self, Escape> {
        let mut reader = Reader { raw, at: 0 };
        let value = Self::read(&mut reader)?;
        reader.finish()?;
        Ok(value)
    }
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

/// The three opcodes, written once.
///
/// # Why a macro, in a crate that mostly refuses them
///
/// Because the alternative is five sequences that have to agree: the opcode
/// constants, the list `known` matches against, the [`Entry`] variants, the
/// answer to *does this close the frame*, and the specimens the round-trip test
/// iterates. The exit this file is accepted on says a canvas whose placements
/// did not arrive is refused at the commit; a hand-written specimen table is
/// precisely how a sentence like that stops being true while every test stays
/// green — a fourth opcode is added, it is not in the table, and the loop over
/// the table keeps passing.
///
/// The `closes` column is the one that is not decoration. *Which entry ends a
/// participation frame* is the question the poison rule rests on, and it has to
/// be answerable from the raw opcode before anything is decoded, because a
/// frame that has already been refused is not decoded again. Answering it on
/// the line that declares the opcode means a fourth opcode must answer it too,
/// and two opcodes claiming it fails an assertion at the foot of this file
/// rather than producing a frame that closes twice.
macro_rules! entries {
    (
        $(
            $(#[$about:meta])*
            $variant:ident / $record:ident / $constant:ident = $opcode:literal,
            closes: $closes:literal;
        )*
    ) => {
        /// The participation service's opcode space.
        ///
        /// Per-service and not global: section 05. These numbers mean nothing
        /// on the frame's ring, the control ring, the compositor's or the
        /// semantic channel's, and comparing an opcode across two spaces is the
        /// mistake a single global enumeration would invite.
        ///
        /// They start at one, so that a zeroed entry — which is what an
        /// untouched slot of a fresh mapping holds — names no opcode and is
        /// refused rather than read as the first one.
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

            /// A word for a log or a trace.
            #[must_use]
            pub const fn label(opcode: u8) -> &'static str {
                match opcode {
                    $($constant => stringify!($variant),)*
                    _ => "unknown",
                }
            }

            /// Does this entry close a participation frame?
            ///
            /// `None` for an opcode this build does not know, which is a
            /// different answer from *no* and is kept different on purpose: a
            /// caller that collapsed the two would treat an unknown opcode as
            /// an ordinary edit and go on to fold it into the canvas being
            /// assembled, which is the wrong question about an opcode that does
            /// not exist.
            #[must_use]
            pub const fn closes(opcode: u8) -> Option<bool> {
                match opcode {
                    $($constant => Some($closes),)*
                    _ => None,
                }
            }

            /// How many of this service's opcodes close a frame.
            ///
            /// Counted from the same list, so the assertion below the
            /// invocation is about what is declared rather than about what
            /// somebody remembered.
            /// Unit: opcodes.
            pub const CLOSERS: usize = [$($closes),*].len()
                - {
                    let mut open = 0;
                    let list = [$($closes),*];
                    let mut at = 0;
                    while at < list.len() {
                        if !list[at] {
                            open += 1;
                        }
                        at += 1;
                    }
                    open
                };
        }

        /// What a participation entry says, once its payload has been believed.
        ///
        /// One variant per opcode, emitted from the same list, so the set of
        /// things an entry can be is the set of opcodes there are.
        ///
        /// **Private.** A public decoded entry would be a second route to
        /// *where does this occupant sit*, reachable by a receiver that never
        /// reached a commit — which is the exit this module is accepted on,
        /// leaking through a type rather than through a function.
        /// [`Reception`] is the door, and what comes back through it before the
        /// floor is [`Progress::Assembling`], which carries nothing.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        enum Entry {
            $(
                $(#[$about])*
                $variant($record),
            )*
        }

        impl Entry {
            /// One legal value of every opcode's record.
            ///
            /// The round-trip test's corpus, and it is derived rather than
            /// written: a fourth opcode is round-tripped by the tests that
            /// already exist, on the day it is declared, without anybody
            /// remembering.
            #[cfg(test)]
            const fn specimens() -> [Self; op::COUNT] {
                [$(Self::$variant($record::SPECIMEN)),*]
            }

            /// The opcode that names this entry.
            const fn opcode(&self) -> u8 {
                match self {
                    $(Self::$variant(_) => op::$constant,)*
                }
            }

            /// The payload as it crosses: the record's fields, then zero to
            /// [`PAYLOAD_BYTES`].
            fn payload(&self) -> [u8; PAYLOAD_BYTES] {
                match self {
                    $(Self::$variant(record) => record.to_payload(),)*
                }
            }

            /// The bytes a record's fields occupy, for the caller that needs to
            /// know where its tail starts.
            #[cfg(test)]
            const fn width(&self) -> usize {
                match self {
                    $(Self::$variant(_) => $record::WIDTH,)*
                }
            }

            /// The entry an opcode and a payload name.
            fn read(opcode: u8, raw: &[u8; PAYLOAD_BYTES]) -> Result<Self, Escape> {
                match opcode {
                    $(
                        opcode_pattern!($constant) =>
                            Ok(Self::$variant($record::from_payload(raw)?)),
                    )*
                    _ => Err(Escape::UnknownOpcode),
                }
            }
        }
    };
}

entries! {
    /// Open a participation frame: which canvas, in what dimension, with what
    /// selected.
    ///
    /// The three clauses of RFC 0078's floor that a declaration can state on
    /// its own. It must be the first entry of a frame and may appear once in
    /// one.
    Declaring / Declaring / DECLARE_CANVAS = 0x01, closes: false;

    /// Place one occupant in the dimension.
    ///
    /// The entry the floor is about, and the entry a peer escapes RFC 0078 by
    /// simply not sending. There are as many of these as the canvas has
    /// occupants, and *as many as it has* is what the commit checks.
    Place / Place / PLACE = 0x02, closes: false;

    /// Close the frame: admit the canvas against the receiving tree, or refuse
    /// it.
    ///
    /// It carries nothing at all — no canvas, no token, no count. The canvas is
    /// named by the declaration that opened the frame, and restating it here
    /// would be a second copy of a fact that can disagree with the first, which
    /// is what RFC 0077 refuses everywhere else in this vocabulary. A count of
    /// placements would be worse: the receiver would then have two answers to
    /// *how many arrived* and would have to decide which to believe, when the
    /// one that matters is neither — it is how many the **tree** says there
    /// are.
    Participate / Participate / PARTICIPATE = 0x03, closes: true;
}

/// Open a participation frame: which canvas, in what dimension, with what
/// selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Declaring {
    /// The canvas this frame is about.
    canvas: u64,
    /// What one tick of its dimension is worth.
    base: Base,
    /// What is selected in it.
    selection: Selection,
}

impl Record for Declaring {
    const WIDTH: usize = 8 + 4 + 1 + 8 + 8 + 8;
    const SPECIMEN: Self = Self { canvas: 7, base: Base::FLICKS, selection: Selection::NOTHING };

    fn write(&self, out: &mut Writer) {
        out.u64(self.canvas);
        out.u32(self.base.ticks_per_second);
        match self.selection.span {
            None => {
                out.u8(shape::POINT);
                out.i64(0);
                out.i64(0);
            }
            Some(extent) => {
                out.u8(shape::STRETCH);
                out.i64(extent.start.0);
                out.i64(extent.end.0);
            }
        }
        out.u64(self.selection.lane.unwrap_or(NO_NODE));
    }

    fn read(raw: &mut Reader) -> Result<Self, Escape> {
        let canvas = raw.u64();
        let ticks_per_second = raw.u32();
        let shape = raw.u8();
        let start = raw.i64();
        let end = raw.i64();
        let lane = raw.u64();
        if canvas == NO_NODE {
            return Err(Escape::NoNode);
        }
        // A selection confined to the canvas itself. The only such statement
        // one entry can make on its own, so it is the only one this format can
        // refuse; a lane belonging to some other canvas is the tree's to catch,
        // where `Presented::lane_of` already lives.
        if lane == canvas {
            return Err(Escape::NotALane(lane));
        }
        let base = Base::new(ticks_per_second)?;
        let span = match shape {
            // The two extent words are fields this shape does not read, so they
            // are held to the rule every unread field in this crate is held to
            // rather than quietly ignored. They are not in the tail — the
            // decoder consumed them to get past them — so `Reader::finish`
            // cannot see them, which is exactly why the check is written here.
            shape::POINT if start == 0 && end == 0 => None,
            shape::POINT => return Err(Escape::Reserved),
            shape::STRETCH => Some(Extent::new(Ticks(start), Ticks(end))?),
            _ => return Err(Escape::Malformed),
        };
        let lane = if lane == NO_NODE { None } else { Some(lane) };
        Ok(Self { canvas, base, selection: Selection { span, lane } })
    }
}

/// Place one occupant in the dimension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Place {
    /// The occupant and its span.
    placement: Placement,
}

impl Record for Place {
    const WIDTH: usize = 8 + 1 + 8 + 8;
    const SPECIMEN: Self =
        Self { placement: Placement { occupant: 7, span: Span::At(Ticks::ORIGIN) } };

    fn write(&self, out: &mut Writer) {
        out.u64(self.placement.occupant);
        match self.placement.span {
            Span::At(instant) => {
                out.u8(shape::POINT);
                out.i64(instant.0);
                out.i64(0);
            }
            Span::Between(extent) => {
                out.u8(shape::STRETCH);
                out.i64(extent.start.0);
                out.i64(extent.end.0);
            }
        }
    }

    fn read(raw: &mut Reader) -> Result<Self, Escape> {
        let occupant = raw.u64();
        let shape = raw.u8();
        let start = raw.i64();
        let end = raw.i64();
        if occupant == NO_NODE {
            return Err(Escape::NoNode);
        }
        let span = match shape {
            // `end` is a field a point does not read. See `Declaring::read`.
            shape::POINT if end == 0 => Span::At(Ticks(start)),
            shape::POINT => return Err(Escape::Reserved),
            shape::STRETCH => Span::Between(Extent::new(Ticks(start), Ticks(end))?),
            _ => return Err(Escape::Malformed),
        };
        Ok(Self { placement: Placement { occupant, span } })
    }
}

/// Close the frame: admit the canvas against the receiving tree, or refuse it.
///
/// A record with no fields, which is the one shape in this crate where
/// `Reader::finish` polices the *whole* payload. A peer that smuggled a field
/// into a commit — a count, a token, a second canvas — is refused by the rule
/// that was already there rather than by a check somebody remembered to write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Participate;

impl Record for Participate {
    const WIDTH: usize = 0;
    const SPECIMEN: Self = Self;

    fn write(&self, _out: &mut Writer) {}

    fn read(_raw: &mut Reader) -> Result<Self, Escape> {
        Ok(Self)
    }
}

/// One participation entry as it crosses: part I's envelope, and the body its
/// opcode names.
///
/// Every field here is either read by this format or refused, and there is no
/// third category — which is the sentence this half of the module is arranged
/// to make true.
///
/// Private, for [`Entry`]'s reason: it holds a decoded body, and a decoded body
/// outside a [`Participation`] is an answer about a canvas nobody admitted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Crossing {
    /// Returned verbatim in the completion, and opaque here.
    user_data: u64,
    /// The scheduling class, and the depth its urgency has already crossed.
    class: u16,
    /// Where this entry's payload is, in bytes from the arena's first byte.
    payload_offset: u32,
    /// Submission flags.
    flags: u8,
    /// What the entry says.
    body: Entry,
}

impl Crossing {
    /// The submission entry this crossing rides in.
    ///
    /// Written field by field, with no `..Sqe::ZERO`, so that a field added to
    /// [`Sqe`] stops this build here and asks whether a participation entry
    /// reads it. [`Crossing::decode`] then compares an arriving entry against
    /// exactly this, which turns whatever is answered into something a peer
    /// cannot get wrong.
    const fn envelope(&self) -> Sqe {
        Sqe {
            opcode: self.body.opcode(),
            flags: self.flags,
            class: self.class,
            // A participation entry names no capability. Authority here is the
            // ring's — the client holds a channel to the component that owns
            // the tree, and what it may declare about that tree's canvases is
            // that tree — so a capability index on an entry would be an
            // authority nobody granted and a field nobody reads. The one
            // capability a canvas carries is its rendering, which is a node's
            // content on the semantic channel.
            cap: 0,
            user_data: self.user_data,
            // No participation opcode is scheduled work: the module's *no
            // deadline* argues it, and this is where a peer that disagreed is
            // refused, because a non-zero deadline fails the byte comparison in
            // `decode`.
            deadline: NO_DEADLINE,
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
    fn encode(&self) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        (self.envelope(), self.body.payload())
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// The order is the order a refusal is distinguishable in, which is
    /// [`store`](crate::store)'s rule for the same job: the opcode and the
    /// flags first, because a caller acts on those differently; the framing;
    /// then the payload; then the whole-envelope comparison, which is the
    /// catch-all nothing can be added past.
    fn decode(entry: &Sqe, payload: &[u8; PAYLOAD_BYTES]) -> Result<Self, Escape> {
        envelope_rules(entry.opcode, entry.flags)?;
        if entry.len as usize != PAYLOAD_BYTES {
            return Err(Escape::Malformed);
        }
        // A channel mapping is a `u32` of bytes — `layout::MAX_ENTRIES` is
        // chosen so that every offset in one fits with room to spare — so an
        // offset past that is not an offset into any arena. Refused here rather
        // than truncated into the field below, because a truncation would make
        // the comparison that follows fail as *a reserved field is not zero*,
        // which is a true sentence about the wrong field.
        if entry.offset > u64::from(u32::MAX) {
            return Err(Escape::Malformed);
        }

        let body = Entry::read(entry.opcode, payload)?;
        let crossing = Self {
            user_data: entry.user_data,
            class: entry.class,
            payload_offset: entry.offset as u32,
            flags: entry.flags,
            body,
        };

        // The check this module shares with `scene` and `semantic`, on the
        // envelope half. Not a list of the fields a participation entry ignores
        // — such a list is correct on the day it is written and silent
        // afterwards — but the entry this build would have produced from the
        // fields it just read, compared in full. Any field `envelope` does not
        // set, including one a later ABI adds, must arrive zero.
        if sqe_bytes(&crossing.envelope()) != sqe_bytes(entry) {
            return Err(Escape::Reserved);
        }
        Ok(crossing)
    }
}

/// The envelope rules that do not need the payload.
///
/// One function, called by [`Arrangement::emit`] before submitting and by
/// [`Crossing::decode`] after receiving, so that a producer and a consumer
/// cannot hold different opinions about which entries are legal.
const fn envelope_rules(opcode: u8, flags: u8) -> Result<(), Escape> {
    if op::closes(opcode).is_none() {
        return Err(Escape::UnknownOpcode);
    }
    if flags & !FLAGS_ACCEPTED != 0 {
        return Err(Escape::UnknownFlag);
    }
    Ok(())
}

/// The sixty-four bytes of a submission entry.
///
/// A byte view rather than a field-by-field comparison, and the difference is
/// the whole of [`Crossing::decode`]'s guarantee: a comparison written out by
/// hand covers the fields somebody listed, and this one covers the fields there
/// are.
///
/// The same function exists privately in [`scene`](crate::scene) and in
/// [`semantic`](crate::semantic). It is duplicated rather than shared because
/// the shared home would be `lib.rs`, beside [`Sqe`], and moving it there is a
/// change to a file this one does not own. Three copies is the point at which
/// that move is worth making, and this comment is the record of the third.
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

/// One participation channel's reception: the frame being assembled, and the
/// poison.
///
/// # What it is for
///
/// [`Crossing::decode`] is a pure function and knows nothing about what came
/// before. Three of this format's rules are about exactly that — a placement
/// needs a canvas open, a frame declares one canvas, and a refusal poisons the
/// frame — so they need something that remembers. This is the smallest thing
/// that can, and it is also the door: the only public route from bytes to an
/// answer about a canvas runs through [`Reception::accept`].
///
/// # The contract, stated because it is what makes the refusal mean anything
///
/// **Present nothing until [`Reception::accept`] has returned a
/// [`Participation`].** This type helps as far as a type can — it hands back no
/// placement before the commit, and once a frame is poisoned every later entry
/// of it is refused with the same [`Fault`] — but it holds no surface, so a
/// receiver that drew something out of an entry it never saw is a receiver that
/// wrote its own decoder. The rule is written here rather than assumed.
///
/// *It hands back no placement* is a claim about every public route out of this
/// type, and a `#[derive(Debug)]` is one — which is why the [`core::fmt::Debug`]
/// impl below is written by hand rather than derived.
#[derive(Clone, Copy)]
pub struct Reception {
    /// The channel epoch this frame belongs to.
    epoch: u32,
    /// How many entries of the current frame have been offered.
    at: u32,
    /// The canvas being assembled, if one has been declared.
    building: Option<Arrangement>,
    /// The refusal that poisoned the current frame, if one did.
    poison: Option<Fault>,
}

/// Printed by hand, and the hand-written impl **is** the repair.
///
/// `#[derive(Debug)]` here printed `building`, and through it every placement
/// that had arrived — the occupant's identifier, its shape, and both tick
/// bounds — on a reception whose frame had not been committed and whose census
/// answer was a refusal. That is a public route from arriving bytes to *where
/// does this occupant sit* which does not run through [`Arrangement::admit`],
/// and the floor is precisely the claim that no such route exists. The privacy
/// of `placement_of` and `placements` was standing in for that claim, and a
/// derive walks straight past privacy: it is written against the fields, not
/// against the methods.
///
/// So this prints what [`Reception::assembling`] and [`Reception::poisoned`]
/// already publish, and nothing a renderer could draw from.
/// `a_half_arrived_canvas_does_not_come_back_through_a_debug_line` compares the
/// whole line, so a later `#[derive(Debug)]` here is a red test rather than a
/// reopened hole.
///
/// [`Arrangement`] keeps its own derived `Debug`, and that is not the same
/// question: there it is the declaring process printing placements it wrote
/// itself, which is diagnostics about its own state. What may not come back is
/// the *arriving* half before the commit has been admitted.
///
/// *What would reverse this:* a receiver that genuinely cannot debug a refused
/// frame without seeing what had arrived. The answer then is a method that says
/// so in its own name and its own documentation — not a derive that says
/// nothing and is reached by `{:?}`.
impl core::fmt::Debug for Reception {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Reception")
            .field("epoch", &self.epoch)
            .field("at", &self.at)
            .field("assembling", &self.assembling())
            .field("poison", &self.poison)
            .finish()
    }
}

impl Reception {
    /// A channel on which no canvas has been declared yet.
    #[must_use]
    pub const fn opening(epoch: u32) -> Self {
        Self { epoch, at: 0, building: None, poison: None }
    }

    /// Is a canvas being assembled?
    ///
    /// Exposed so that a receiver's own logging can see the state rather than
    /// infer it from a sequence of results. It answers *is a frame open*, not
    /// *what is in it* — see [`Arrangement`]'s *what it deliberately cannot
    /// answer*.
    #[must_use]
    pub const fn assembling(&self) -> bool {
        self.building.is_some()
    }

    /// Is the frame being assembled already refused?
    #[must_use]
    pub const fn poisoned(&self) -> Option<Fault> {
        self.poison
    }

    /// Follow the channel's epoch, discarding a half-assembled frame if it
    /// moved.
    ///
    /// [`ChannelHeader::epoch`](crate::ChannelHeader::epoch) moves when a peer
    /// restarts, and its own documentation says every outstanding token is then
    /// stale and must be discarded rather than matched. A half-declared canvas
    /// is such a token: the placements that would have completed it belonged to
    /// a peer that is gone, and admitting them alongside a new peer's would be
    /// assembling one canvas out of two processes.
    ///
    /// Answers whether anything was discarded, so that a caller can drop the
    /// surface it was presenting in the same breath.
    pub fn follow_epoch(&mut self, epoch: u32) -> bool {
        if epoch == self.epoch {
            return false;
        }
        *self = Self::opening(epoch);
        true
    }

    /// Offer the next entry of this channel, against the tree this receiver is
    /// presenting.
    ///
    /// **This is the only public route from bytes to an answer about a canvas,
    /// and it runs through the floor.** Every entry that is not the commit
    /// answers [`Progress::Assembling`], which carries nothing at all; the
    /// commit answers [`Progress::Participating`] only if
    /// [`Arrangement::admit`] agreed, and otherwise the frame is refused and
    /// the receiver has nothing new to present.
    ///
    /// # Errors
    ///
    /// A [`Fault`]: the [`Escape`] and the position of the entry that earned
    /// it. Once a frame is poisoned, every later entry of that frame — and the
    /// commit that would have closed it — is refused with the *first* fault
    /// rather than with one of its own, because the first is the cause and the
    /// rest are consequences of having decoded past it.
    pub fn accept(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        tree: &dyn Presented,
    ) -> Result<Progress, Fault> {
        // Read off the raw opcode rather than the decoded body, because a frame
        // that has already been poisoned is not decoded again: the question
        // *does this entry end the frame* has to be answerable before that. An
        // unknown opcode closes nothing, so a poisoned frame is never closed by
        // one.
        let closes = op::closes(entry.opcode) == Some(true);
        if let Some(standing) = self.poison {
            if closes {
                self.clear();
            }
            return Err(standing);
        }

        let at = self.at;
        self.at = self.at.saturating_add(1);

        match self.fold(entry, payload, tree) {
            Ok(progress) => {
                if closes {
                    self.clear();
                }
                Ok(progress)
            }
            Err(escape) => {
                let fault = Fault { escape, at };
                if closes {
                    self.clear();
                } else {
                    self.poison = Some(fault);
                }
                Err(fault)
            }
        }
    }

    /// Decode one entry and fold it into the frame.
    ///
    /// Split from [`accept`](Self::accept) so that the position bookkeeping and
    /// the poison rule are written once, around a body that only ever answers
    /// *what does this entry mean*. The match is exhaustive and wildcard-free,
    /// so a fourth opcode stops this build and asks what it does to a frame
    /// rather than being folded in as whatever the last arm happened to be.
    fn fold(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
        tree: &dyn Presented,
    ) -> Result<Progress, Escape> {
        let crossing = Crossing::decode(entry, payload)?;
        match crossing.body {
            Entry::Declaring(declaring) => {
                if let Some(standing) = self.building {
                    return Err(Escape::Redeclared(standing.canvas));
                }
                self.building = Some(Arrangement::declaring(
                    declaring.canvas,
                    declaring.base,
                    declaring.selection,
                ));
                Ok(Progress::Assembling)
            }
            Entry::Place(place) => {
                let Some(building) = self.building else { return Err(Escape::NotDeclared) };
                self.building = Some(building.with_placement(place.placement)?);
                Ok(Progress::Assembling)
            }
            Entry::Participate(Participate) => {
                let Some(building) = self.building else { return Err(Escape::NotDeclared) };
                Ok(Progress::Participating(building.admit(tree)?))
            }
        }
    }

    /// Start the next frame: no canvas, no placements, no poison, position
    /// zero.
    fn clear(&mut self) {
        self.at = 0;
        self.building = None;
        self.poison = None;
    }
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

/// The widest record's fields, computed rather than named.
///
/// What [`Writer::put`] indexing without a bounds check rests on, and it is a
/// maximum rather than a comparison against the record that happens to be
/// widest today: an assertion naming [`Declaring`] would keep passing on the
/// day [`Place`] outgrew it, which is the shape of defect this file is written
/// against everywhere else.
/// Unit: bytes.
const WIDEST: usize = {
    let widths = [Declaring::WIDTH, Place::WIDTH, Participate::WIDTH];
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

// The stride is the widest record rounded up to the arena's grain, asserted
// rather than written twice: a record that grew past it stops the build here
// instead of overflowing a payload at run time, and a record that shrank leaves
// a stride nobody needs, which is also worth a diff.
const _: () = assert!(PAYLOAD_BYTES == WIDEST.next_multiple_of(8));

// Every specimen is a value a peer may legally send. `Record::SPECIMEN` is what
// the round-trip test runs over, and a specimen a decoder would refuse would
// make that test round-trip something no wire carries — green, and about
// nothing. The two fields a record refuses outright are asserted here, in every
// build rather than only under `cfg(test)`, because a specimen is part of the
// format rather than part of its tests.
const _: () = assert!(Declaring::SPECIMEN.canvas != NO_NODE);
const _: () = assert!(Declaring::SPECIMEN.base.ticks_per_second != 0);
const _: () = assert!(Place::SPECIMEN.placement.occupant != NO_NODE);
// `Participate` has no fields to be wrong. Named here rather than left out, so
// that the list is three long like the opcode list and a fourth record's
// absence from it is visible.
const _: () = assert!(Participate::WIDTH == 0);

// Exactly one opcode closes a frame. Two would make `accept`'s poison rule
// ambiguous — which commit clears the poison — and zero would make a frame
// impossible to close at all; both are a build failure rather than a protocol
// nobody can use.
const _: () = assert!(op::CLOSERS == 1);

// Every flag this format accepts is a flag the ABI defines. A bit outside
// `flags::KNOWN` here would be a flag this service accepted and no other reader
// of an entry had ever heard of.
const _: () = assert!(FLAGS_ACCEPTED & !flags::KNOWN == 0);

// A placement count fits the byte `Arrangement` keeps it in. The capacity and
// its counter are two numbers that have to agree, and this is where they are
// made to.
const _: () = assert!(PLACED_MAX <= u8::MAX as usize);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class;

    /// The file that declares the floor, read when this file is compiled.
    ///
    /// **This is not a dependency.** `interface/` is a leaf with no
    /// dependencies and nothing depends on it, which is argued in its own
    /// manifest and is why this crate cannot import `Escape`. No crate is
    /// linked here and no type is imported: `include_str!` reads a text file at
    /// compile time, which needs neither a filesystem at run time nor an
    /// allocator, and `interface/src/ladder.rs` already holds itself to
    /// `claims/0033` and RFC 0080 in exactly this way.
    ///
    /// The cost is real and is the intended one: editing `canvas.rs` rebuilds
    /// this crate's tests. The floor and the wire that carries it are one
    /// decision written in two directories, and a rebuild is the cheapest
    /// possible way for that to be a fact the toolchain knows rather than a
    /// sentence in a comment.
    const CANVAS: &str = include_str!("../../interface/src/canvas.rs");

    /// The RFC this module implements, read when this file is compiled.
    ///
    /// Read so that reversing the decision goes red here rather than leaving a
    /// module that enforces a floor nobody believes any more.
    const RFC_0078: &str =
        include_str!("../../docs/rfc/0078-a-canvas-declares-its-content-or-it-is-an-escape.md");

    /// How many refusals this parser will hold before it gives up.
    ///
    /// Not a claim about either enum — both sizes are read here, never written.
    /// It is the fixed array a `#![no_std]` test has instead of a `Vec`, set far
    /// enough above either list that reaching it means one of them doubled,
    /// which is worth a red test of its own.
    const ESCAPE_LIMIT: usize = 64;

    /// The clauses `interface`'s `Escape` names that this format deliberately
    /// does not carry, with the reason each one stays behind.
    ///
    /// **This list is the decision, and the test is what stops it rotting.** A
    /// clause added to `interface`'s `Escape` is in neither this list nor
    /// [`Escape::LABELS`], and `an_escape_is_the_same_word_on_both_sides_of_the_ring`
    /// then fails naming it — so somebody has to decide whether it crosses,
    /// rather than the wire quietly carrying eleven of twelve clauses and
    /// nobody noticing which one it dropped.
    ///
    /// - `NotOfThisCanvas` is `interface`'s word for a question about a node it
    ///   does not declare, and it exists there because that module answers
    ///   about canvases and lanes it places nothing on. This one places
    ///   everything it can answer about, so *a node this canvas does not place*
    ///   and *a node this canvas cannot answer for* are the same node, and
    ///   `NotAnOccupant` already names it — for a placement and for a question
    ///   alike. Two words for one fact is what the agreement above exists to
    ///   prevent.
    /// - `Unsayable` is a tick count that will not fit a description in
    ///   seconds. Nothing here converts a tick to a second — the wire carries
    ///   the pair and the projection does the arithmetic — so there is no
    ///   conversion to fail.
    /// - `Vocabulary` is the semantic tree failing its own check. That check
    ///   happens on the semantic channel, before any canvas is declared over
    ///   it, and duplicating it here would be a second opinion about a tree
    ///   this format does not carry.
    const NOT_CARRIED: [&str; 3] = ["NotOfThisCanvas", "Unsayable", "Vocabulary"];

    /// The refusals this format has that `interface`'s `Escape` does not, and
    /// why none of them has a counterpart there.
    ///
    /// **Named rather than counted, and that is the repair.** What stood here
    /// was a bare `7` inside `Escape::COUNT - 7`, with no list to count it
    /// from, in a file whose whole design argument is that a count is derived
    /// from the list it counts. Worse, it made the assertion a *lower bound*,
    /// and a lower bound cannot see a clause deleted from the declaring side:
    /// removing `Unplaced` — the floor itself — from `interface`'s `Escape`
    /// left the comparison green while the test's own comment claimed that
    /// deleting a variant made it red.
    ///
    /// Every one of these is a property of an arriving *entry* rather than of a
    /// canvas: an opcode, a flag, a byte that should have been zero, a payload
    /// that does not frame, a field that names no node, and the two rules about
    /// a frame's order. A constructor in the declaring process cannot fail any
    /// of them, because there are no bytes there to be wrong — which is why
    /// their absence one crate over is a fact rather than an omission, and why
    /// the test below asserts that `interface` does *not* declare them.
    const WIRE_ONLY: [&str; 7] = [
        "UnknownOpcode",
        "UnknownFlag",
        "Reserved",
        "Malformed",
        "NoNode",
        "NotDeclared",
        "Redeclared",
    ];

    /// The canvas, as the receiving tree holds it.
    const SEQUENCE: u64 = 320;
    /// Its first lane.
    const DIALOGUE_LANE: u64 = 321;
    /// Its third lane — *track 3*, the one section 13's sentence is about.
    const FOLEY_LANE: u64 = 323;
    /// A lane of some other canvas entirely.
    const REEL_LANE: u64 = 341;
    /// The clip section 13's sentence asks for.
    const DIALOGUE: u64 = 331;
    /// A second clip, on another lane, so that a floor met by one occupant is
    /// not a floor met by all of them.
    const STEPS: u64 = 332;
    /// A marker: the playhead. It is here because a fixture whose occupants are
    /// all the same shape proves only that the module works on one shape.
    const PLAYHEAD: u64 = 333;
    /// A node the tree holds under the canvas that the dimension cannot place.
    const TAKES: u64 = 334;
    /// A node of somewhere else entirely.
    const STRANGER: u64 = 900;

    /// A time in tenths of a second, as ticks of [`Base::FLICKS`].
    ///
    /// The scale is in the name because `CLAUDE.md` asks for that wherever the
    /// writer knows what is being counted, and here it does: `42` is 4.2 s.
    /// Written as arithmetic rather than as a conversion because this module
    /// deliberately has none — a tenth of a second is `ticks_per_second / 10`,
    /// which is exact at this base and is why this base is the one the fixture
    /// uses.
    fn seconds_x10(value: i64) -> Ticks {
        Ticks::new(value * i64::from(Base::FLICKS.ticks_per_second()) / 10)
    }

    /// A stretch, in tenths of a second.
    fn span_x10(from: i64, to: i64) -> Extent {
        Extent::new(seconds_x10(from), seconds_x10(to)).expect("a span that advances")
    }

    /// The tree a receiver is presenting, answering for one canvas.
    ///
    /// A real implementor walks its nodes; this holds the answers a walk would
    /// have produced, because this crate has no tree to walk. What matters is
    /// that it decides nothing — every field is a fact about the tree and every
    /// arm of [`Census`] is reachable from one.
    struct Tree {
        /// The one canvas this tree holds.
        canvas: u64,
        /// Every clip and marker under it, in declaration order.
        occupants: &'static [Occupant],
        /// Every lane of it.
        lanes: &'static [u64],
        /// A node under it the dimension cannot place, if the fixture wants
        /// one.
        unplaceable: Option<u64>,
        /// More occupants than one arrangement can carry.
        crowded: bool,
    }

    impl Presented for Tree {
        fn census(&self, canvas: u64, into: &mut [Occupant; PLACED_MAX]) -> Census {
            if canvas != self.canvas {
                return Census::NotACanvas;
            }
            if let Some(node) = self.unplaceable {
                return Census::Unplaceable(node);
            }
            if self.crowded {
                return Census::Crowded;
            }
            for (slot, occupant) in into.iter_mut().zip(self.occupants) {
                *slot = *occupant;
            }
            Census::Occupies(self.occupants.len())
        }

        fn lane_of(&self, canvas: u64, lane: u64) -> bool {
            canvas == self.canvas && self.lanes.contains(&lane)
        }
    }

    /// The timeline the receiver is presenting: two clips and a playhead, on
    /// three lanes.
    fn timeline() -> Tree {
        const OCCUPANTS: [Occupant; 3] = [
            Occupant { node: DIALOGUE, kind: Kind::Clip, lane: DIALOGUE_LANE },
            Occupant { node: STEPS, kind: Kind::Clip, lane: FOLEY_LANE },
            // On the canvas itself rather than on a lane: a sequence-wide
            // playhead is not a cue *on track 3*, and a fixture where every
            // occupant sits on a lane would let `lane_of` return `Some`
            // unconditionally and still pass.
            Occupant { node: PLAYHEAD, kind: Kind::Marker, lane: NO_NODE },
        ];
        const LANES: [u64; 3] = [DIALOGUE_LANE, 322, FOLEY_LANE];
        Tree {
            canvas: SEQUENCE,
            occupants: &OCCUPANTS,
            lanes: &LANES,
            unplaceable: None,
            crowded: false,
        }
    }

    /// The arrangement a conforming peer sends: every occupant the tree holds,
    /// placed.
    fn complete() -> Arrangement {
        Arrangement::declaring(SEQUENCE, Base::FLICKS, Selection::over(span_x10(42, 68)))
            .with_placement(Placement::over(DIALOGUE, span_x10(42, 68)))
            .expect("room for the first")
            .with_placement(Placement::over(STEPS, span_x10(10, 20)))
            .expect("room for the second")
            .with_placement(Placement::at(PLAYHEAD, seconds_x10(50)))
            .expect("room for the third")
    }

    /// How a test's entries ride.
    fn carriage() -> Carriage {
        Carriage { user_data: 0x0dd_ba11, class: class::SOFT, payload_offset: 64, flags: 0 }
    }

    /// One entry as it crosses, with the fields a producer chooses.
    fn crossing(body: Entry) -> (Sqe, [u8; PAYLOAD_BYTES]) {
        Crossing { user_data: 0x0dd_ba11, class: class::SOFT, payload_offset: 64, flags: 0, body }
            .encode()
    }

    /// Feed a whole frame to a reception and answer what the last entry said.
    ///
    /// Written once because every hostile-peer test below is *this frame, minus
    /// something*, and a helper that stopped early or swallowed a fault would
    /// make each of them assert less than its name.
    fn offer(
        reception: &mut Reception,
        tree: &Tree,
        frame: &[Submission],
    ) -> Result<Progress, Fault> {
        let mut last = Ok(Progress::Assembling);
        for submission in frame {
            last = reception.accept(&submission.entry, &submission.payload, tree);
        }
        last
    }

    /// Drain an iterator into a fixed array, because a `#![no_std]` test has no
    /// `Vec` and an assertion against a whole slice says more than a loop that
    /// counts: it catches an answer that is too long as well as one that is too
    /// short, and it catches the order.
    fn collect(nodes: impl Iterator<Item = u64>) -> ([u64; PLACED_MAX], usize) {
        let mut out = [NO_NODE; PLACED_MAX];
        let mut found = 0;
        for node in nodes {
            out[found] = node;
            found += 1;
        }
        (out, found)
    }

    /// A fixed byte buffer a `core::fmt::Write` can be driven into.
    ///
    /// A `#![no_std]` test has no `String`, and the point of
    /// `a_half_arrived_canvas_does_not_come_back_through_a_debug_line` is to
    /// *look at* a formatted line rather than trust what produced it.
    struct Sink {
        /// The bytes written so far.
        buffer: [u8; 4096],
        /// How many of them there are.
        at: usize,
    }

    impl Sink {
        /// An empty sink.
        fn new() -> Self {
            Self { buffer: [0; 4096], at: 0 }
        }

        /// What has been written, as text.
        fn text(&self) -> &str {
            core::str::from_utf8(&self.buffer[..self.at]).expect("a formatter writes text")
        }
    }

    impl core::fmt::Write for Sink {
        fn write_str(&mut self, text: &str) -> core::fmt::Result {
            let bytes = text.as_bytes();
            // Refused rather than truncated. A truncated line makes an
            // assertion about a prefix while reading as an assertion about the
            // whole, and a debug line that was cut off before the placements
            // would pass the test that exists to find them.
            if self.at + bytes.len() > self.buffer.len() {
                return Err(core::fmt::Error);
            }
            self.buffer[self.at..self.at + bytes.len()].copy_from_slice(bytes);
            self.at += bytes.len();
            Ok(())
        }
    }

    /// The whole of a peer's frame, encoded.
    fn frame(arrangement: &Arrangement) -> ([Submission; PLACED_MAX + 2], usize) {
        let mut out = [Submission::UNSENT; PLACED_MAX + 2];
        let written = arrangement.emit(carriage(), &mut out).expect("room for the frame");
        (out, written)
    }

    #[test]
    fn the_floor_is_enforced_by_the_receiver_and_not_only_by_the_declaring_process() {
        // The exit clause, and it has three moving parts, so all three are
        // asserted against one tree.
        //
        // *The floor's other half.* A peer declares the canvas, names the base,
        // names the selection and commits, sending no placements at all. Every
        // byte of that frame is well-formed. `interface`'s constructor never
        // ran, because the peer does not have it. The commit is refused as
        // `NothingPlaced`, naming the canvas, at the position of the commit.
        //
        // *The floor.* The same peer sends one placement of the three the
        // receiving tree holds. Well-formed again, and a receiver that took it
        // at face value would present a timeline with two thirds of its content
        // undescribable. It is refused as `Unplaced`, naming the first occupant
        // the tree holds that did not arrive.
        //
        // *The refusal is not a rendering.* Neither frame produces a
        // `Participation`, and `Progress::Assembling` — the only other thing a
        // reception hands back — carries nothing that could be drawn.
        //
        // *The edits that make this go red:* deleting either half of the floor
        // in `Arrangement::admit`; making `Progress` carry a placement, which
        // stops `nothing_says_where_anything_is_until_the_floor_is_met`
        // compiling rather than merely failing.
        //
        // *And what no test here can see, said plainly rather than listed as
        // though it could.* Making `Arrangement::placement_of` or
        // `Arrangement::placements` public leaves every test in this file
        // green, and so does giving `Participation` a second constructor. An
        // earlier version of this comment listed both among the edits that go
        // red. They are conventions, not guards, and listing a convention as a
        // guard is worse than listing nothing — this comment is where a reader
        // looks for the structural guarantee. What is actually held is
        // narrower and is held where it can be: `Reception` is this module's
        // only public decoder, everything before the commit answers
        // `Progress::Assembling`, which carries nothing, and
        // `a_half_arrived_canvas_does_not_come_back_through_a_debug_line`
        // closes the one route that was walking past that.
        let tree = timeline();

        let silent = Arrangement::declaring(SEQUENCE, Base::FLICKS, Selection::NOTHING);
        let (entries, written) = frame(&silent);
        assert_eq!(written, 2, "a canvas with no placements is two entries");
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::NothingPlaced(SEQUENCE), at: 1 })
        );

        let partial = Arrangement::declaring(SEQUENCE, Base::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(DIALOGUE, span_x10(42, 68)))
            .expect("room for one");
        let (entries, written) = frame(&partial);
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::Unplaced(STEPS), at: 2 })
        );

        // And the conforming peer, so that the two refusals above are about the
        // floor rather than about the fixture being unadmittable.
        let (entries, written) = frame(&complete());
        let mut reception = Reception::opening(1);
        let Ok(Progress::Participating(participating)) =
            offer(&mut reception, &tree, &entries[..written])
        else {
            panic!("a canvas that placed everything the tree holds is admitted")
        };
        assert_eq!(participating.placed(), 3);
    }

    #[test]
    fn nothing_says_where_anything_is_until_the_floor_is_met() {
        // The structural half of the exit, asserted as an absence: every entry
        // before the commit answers `Progress::Assembling`, and that value is a
        // unit variant. There is no placement in it, no span, no occupant and
        // no arrangement, so a receiver that wanted to draw a half-arrived
        // canvas has nothing to draw it from — which is why the floor is not
        // something a receiver can be in a hurry and skip.
        //
        // The assertion is an equality against a variant that carries nothing,
        // and it is worth saying why that is not a tautology: the day somebody
        // adds a field to `Assembling` to help a caller along, this line stops
        // compiling.
        let tree = timeline();
        let (entries, written) = frame(&complete());
        let mut reception = Reception::opening(1);
        for submission in &entries[..written - 1] {
            assert_eq!(
                reception.accept(&submission.entry, &submission.payload, &tree),
                Ok(Progress::Assembling)
            );
        }
        assert!(reception.assembling(), "the frame is open until the commit");
    }

    #[test]
    fn a_half_arrived_canvas_does_not_come_back_through_a_debug_line() {
        // The other public route out of a reception, and the one a derive opens
        // without anybody writing a line of code. `#[derive(Debug)]` on
        // `Reception` printed its private `building`, and through it every
        // placement that had arrived — occupant, shape and both tick bounds —
        // on a frame whose commit had not happened and whose census answer is a
        // refusal. The floor's structural claim is that no public route yields
        // a placement before the commit; privacy on `placement_of` and
        // `placements` was standing in for that claim, and a derive is written
        // against fields rather than against methods.
        //
        // The whole line is compared rather than scanned. A scan for an
        // occupant's digits passes the day somebody prints a placement some
        // other way; an equality fails on any field appearing at all.
        //
        // *The edits that make this go red:* putting `Debug` back on
        // `Reception`'s derive; adding any field to the hand-written impl.
        use core::fmt::Write as _;

        let tree = timeline();
        let partial = Arrangement::declaring(SEQUENCE, Base::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(DIALOGUE, span_x10(42, 68)))
            .expect("room for one");
        let (entries, written) = frame(&partial);
        let mut reception = Reception::opening(1);

        // Everything but the commit, so the reception is holding a canvas the
        // floor has not admitted and never will.
        for submission in &entries[..written - 1] {
            assert_eq!(
                reception.accept(&submission.entry, &submission.payload, &tree),
                Ok(Progress::Assembling)
            );
        }
        assert!(reception.assembling(), "the fixture is a half-arrived canvas");

        let mut line = Sink::new();
        write!(line, "{reception:?}").expect("a reception's debug line fits");
        assert_eq!(
            line.text(),
            "Reception { epoch: 1, at: 2, assembling: true, poison: None }",
            "a reception prints something other than what `assembling` and `poisoned` publish"
        );

        // And the occupant that did arrive is not in it, spelled the way it
        // crossed. Redundant against the equality above and kept anyway,
        // because it is the sentence the exit is about and it names the thing
        // that leaked.
        let mut occupant = Sink::new();
        write!(occupant, "{DIALOGUE}").expect("a node identifier fits");
        assert!(!line.text().contains(occupant.text()), "the occupant that arrived is printed");
        let mut tick = Sink::new();
        write!(tick, "{}", seconds_x10(42).count()).expect("a tick count fits");
        assert!(!line.text().contains(tick.text()), "the tick bound that arrived is printed");

        // And the frame really is refused, so this is a reception mid-flight
        // rather than one that was never going to answer anything.
        let commit = &entries[written - 1];
        assert_eq!(
            reception.accept(&commit.entry, &commit.payload, &tree),
            Err(Fault { escape: Escape::Unplaced(STEPS), at: 2 })
        );
    }

    #[test]
    fn the_clip_between_4_2_and_6_8_on_track_3_survives_the_crossing() {
        // Section 13's own sentence, asked of a canvas that arrived as bytes.
        // The numbers are its numbers, written in tenths of a second because
        // 4.2 is not expressible in this tree — RFC 0004 — and the base is the
        // one that names a sample boundary exactly.
        //
        // What it asserts is the pair the exit contrasts: the canvas was
        // refused or it is rendered, and this is the second case. The admitted
        // participation answers where the clip is, answers it from the
        // declaration alone, and answers the same thing the peer declared. The
        // lane is the tree's to say and the tree said it; what crossed is the
        // *when*, which is the half `node.rs` cannot hold.
        let tree = timeline();
        let (entries, written) = frame(&complete());
        let mut reception = Reception::opening(1);
        let Ok(Progress::Participating(participating)) =
            offer(&mut reception, &tree, &entries[..written])
        else {
            panic!("the conforming frame is admitted")
        };

        assert_eq!(
            participating.placement_of(DIALOGUE),
            Some(Placement::over(DIALOGUE, span_x10(42, 68)))
        );
        assert_eq!(participating.canvas(), SEQUENCE);
        assert_eq!(participating.base(), Base::FLICKS);
        assert_eq!(participating.selection().span(), Some(span_x10(42, 68)));
        assert_eq!(participating.placement_of(STRANGER), None);

        // *On track 3.* The lane is the tree's fact and the wire does not carry
        // it; what crossed the ring is the *when*, and the census is where the
        // *where* joined it. A marker that sits on the canvas itself answers
        // `None`, which is a real answer about a real occupant — and a node
        // this canvas does not place is refused rather than answered `None`,
        // because a caller that read the second as the first would put a
        // stranger's cue on this timeline.
        assert_eq!(participating.lane_of(STEPS), Ok(Some(FOLEY_LANE)));
        assert_eq!(participating.lane_of(DIALOGUE), Ok(Some(DIALOGUE_LANE)));
        assert_eq!(participating.lane_of(PLAYHEAD), Ok(None));
        assert_eq!(participating.lane_of(STRANGER), Err(Escape::NotAnOccupant(STRANGER)));

        // The join an agent uses: the selection over 4.2 s to 6.8 s touches the
        // dialogue clip and the playhead at 5 s, and does not touch the
        // footsteps at 1 s to 2 s. Markers are included, because a cue inside a
        // selected stretch is selected by every reading an editor has.
        let (selected, found) = collect(participating.selected());
        assert_eq!(&selected[..found], &[DIALOGUE, PLAYHEAD]);
    }

    #[test]
    fn a_selection_confined_to_a_lane_touches_only_that_lane() {
        // The half of a selection the wire cannot answer on its own, and the
        // reason a `Participation` keeps what the census said. A ripple delete
        // over 0 s to 10 s on the foley lane takes the footsteps and nothing
        // else; the same stretch unconfined takes all three. An earlier version
        // of `within_selection` had no lanes to consult and answered the second
        // for both, which is a delete on track 3 that took track 1 with it.
        let tree = timeline();
        let whole = span_x10(0, 100);

        let confined = complete().with_selection(Selection::over(whole).on(FOLEY_LANE));
        let (entries, written) = frame(&confined);
        let mut reception = Reception::opening(1);
        let Ok(Progress::Participating(participating)) =
            offer(&mut reception, &tree, &entries[..written])
        else {
            panic!("a selection on a lane of this canvas is admitted")
        };
        let (selected, found) = collect(participating.selected());
        assert_eq!(&selected[..found], &[STEPS]);

        let across = complete().with_selection(Selection::over(whole));
        let (entries, written) = frame(&across);
        let mut reception = Reception::opening(1);
        let Ok(Progress::Participating(participating)) =
            offer(&mut reception, &tree, &entries[..written])
        else {
            panic!("a selection across every lane is admitted")
        };
        let (selected, found) = collect(participating.selected());
        assert_eq!(&selected[..found], &[DIALOGUE, STEPS, PLAYHEAD]);

        // And a selection that selects nothing touches nothing, which is a
        // different fact from an empty stretch and is the one clause 3 is
        // about: saying *nothing is selected* is a statement.
        let quiet = complete().with_selection(Selection::NOTHING);
        let (entries, written) = frame(&quiet);
        let mut reception = Reception::opening(1);
        let Ok(Progress::Participating(participating)) =
            offer(&mut reception, &tree, &entries[..written])
        else {
            panic!("a canvas that selects nothing has still said so")
        };
        assert!(participating.selection().is_nothing());
        assert_eq!(collect(participating.selected()).1, 0);
    }

    #[test]
    fn an_escape_is_the_same_word_on_both_sides_of_the_ring() {
        // The agreement between the two enums, read rather than asserted from
        // memory. Three things are held here, and the first is the one that
        // keeps the other two honest.
        //
        // *Every clause is accounted for.* Each variant `interface`'s `Escape`
        // declares is either a word this format also uses or is on
        // `NOT_CARRIED` with a reason. A clause added there is in neither, and
        // this fails naming it — which is the question somebody should be asked
        // rather than a wire that silently carries eleven of twelve clauses.
        //
        // *Every word is still a clause.* The other direction, and the one the
        // lower bound this replaced could not see: each variant this format
        // carries is still declared by `interface`, unless it is on
        // `WIRE_ONLY`, in which case `interface` must *not* declare it. A floor
        // clause deleted one crate over used to appear in neither loop and pass.
        //
        // *The reasons do not rot.* Every name on `NOT_CARRIED` is still a
        // variant `interface` declares, so a clause deleted or renamed there
        // does not leave a stale excuse behind.
        //
        // *The counts meet exactly.* `interface`'s list is this one, minus what
        // only a wire can fail, plus what only a constructor can. An equality,
        // with both subtrahends read off lists, because the `>=` that stood
        // here admitted a whole missing clause and would have become a spurious
        // failure the day four more wire-only refusals were added.
        //
        // *The capacity is one number.* `PLACED_MAX` is read out of the
        // declaring file, because a wire that held sixty-four placements while
        // the declaration held thirty-two would refuse valid canvases in one
        // direction and truncate them in the other.
        //
        // *The edits that make this go red:* adding, renaming or deleting a
        // variant of `interface`'s `Escape`; adding, renaming or deleting one
        // of this module's carried variants; changing `PLACED_MAX` on either
        // side; changing `TimeBase::FLICKS`. All three verbs were run in both
        // directions, and deleting `Unplaced` from `interface` — the case that
        // used to pass — is now red.
        let (declared, count) = declared_escapes();
        let declared = &declared[..count];

        for name in declared {
            let carried = Escape::LABELS.contains(name);
            let deliberate = NOT_CARRIED.contains(name);
            assert!(
                carried != deliberate,
                "`{name}` is a clause of the floor that this wire neither carries nor \
                 deliberately leaves behind — decide which, in `Escape` or in `NOT_CARRIED`"
            );
        }
        for label in Escape::LABELS {
            if WIRE_ONLY.contains(&label) {
                assert!(
                    !declared.contains(&label),
                    "`{label}` is listed as this wire's own and `interface` declares it too"
                );
                continue;
            }
            assert!(
                declared.contains(&label),
                "`{label}` is carried across the ring and `interface` no longer declares it"
            );
        }
        for name in NOT_CARRIED {
            assert!(
                declared.contains(&name),
                "`{name}` is excused from the wire and `interface` no longer declares it"
            );
        }
        assert_eq!(
            count,
            Escape::COUNT - WIRE_ONLY.len() + NOT_CARRIED.len(),
            "the two refusal vocabularies no longer account for each other"
        );

        assert_eq!(PLACED_MAX, declared_usize("PLACED_MAX"), "two capacities, one decision");
        assert!(
            CANVAS.contains("705_600_000"),
            "the base this module names is no longer the base `interface` declares"
        );
    }

    #[test]
    fn the_decision_this_module_enforces_is_still_the_decision() {
        // RFC 0078 read as text, for the two sentences this module is the far
        // side of. A decision reversed in the document and left standing in the
        // code is the failure this tree writes RFCs to avoid, and it is not
        // observable from a compiler.
        assert!(
            RFC_0078.contains("that places none at all is refused outright"),
            "RFC 0078 no longer refuses a canvas that places nothing"
        );
        assert!(
            RFC_0078.contains("`Escape::NothingPlaced`"),
            "RFC 0078 no longer spells the floor's other half the way this module does"
        );
        assert!(
            RFC_0078.contains("`Escape::Unplaceable`"),
            "RFC 0078 no longer spells the floor's scope the way this module does"
        );
    }

    #[test]
    fn a_canvas_the_receiving_tree_does_not_hold_is_refused() {
        // Clause 1, answered by the only party that can. A peer may name any
        // node it likes; whether that node is a canvas is a fact about the tree
        // the receiver is presenting.
        let tree = timeline();
        let elsewhere = Arrangement::declaring(STRANGER, Base::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(DIALOGUE, span_x10(42, 68)))
            .expect("room for one");
        let (entries, written) = frame(&elsewhere);
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::NotACanvas(STRANGER), at: 2 })
        );
    }

    #[test]
    fn a_selection_confined_to_a_lane_this_canvas_does_not_hold_is_refused() {
        // Another canvas's lane, which is the case a predicate that only asked
        // *is this a lane* would let through — and the answer it would then
        // give is about a surface this canvas has no standing over.
        let tree = timeline();
        let foreign = complete().with_selection(Selection::over(span_x10(42, 68)).on(REEL_LANE));
        let (entries, written) = frame(&foreign);
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::NotALane(REEL_LANE), at: 4 })
        );

        // And the canvas itself named as a lane, which is the one such
        // statement a single entry can make on its own, so it is refused at the
        // decode rather than at the commit.
        let itself = complete().with_selection(Selection::over(span_x10(42, 68)).on(SEQUENCE));
        let (entries, written) = frame(&itself);
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::NotALane(SEQUENCE), at: 0 })
        );
    }

    #[test]
    fn an_occupant_placed_as_the_other_kind_of_occupant_is_refused() {
        // The tree says the playhead is a marker and the peer placed it over a
        // stretch. *What is at 5 s* has a different answer depending on which
        // the author meant, so the disagreement is refused rather than resolved
        // in favour of whoever spoke last.
        let tree = timeline();
        let muddled = Arrangement::declaring(SEQUENCE, Base::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(DIALOGUE, span_x10(42, 68)))
            .expect("room")
            .with_placement(Placement::over(STEPS, span_x10(10, 20)))
            .expect("room")
            .with_placement(Placement::over(PLAYHEAD, span_x10(49, 51)))
            .expect("room");
        let (entries, written) = frame(&muddled);
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::Mistimed(PLAYHEAD), at: 4 })
        );
    }

    #[test]
    fn a_placement_naming_a_node_the_canvas_does_not_hold_is_refused() {
        let tree = timeline();
        let stranger = complete()
            .with_placement(Placement::over(STRANGER, span_x10(0, 1)))
            .expect("room for a fourth");
        let (entries, written) = frame(&stranger);
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::NotAnOccupant(STRANGER), at: 5 })
        );
    }

    #[test]
    fn a_node_the_dimension_cannot_place_is_refused_on_the_far_side_too() {
        // Clause 4's scope. Without it a peer places one token clip and hangs
        // the content the canvas actually draws off itself as a list — placed
        // nowhere, asked nothing, invisible to a clause that only counts clips.
        // The tree is what sees it, and the tree is on the receiving side.
        let mut tree = timeline();
        tree.unplaceable = Some(TAKES);
        let (entries, written) = frame(&complete());
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::Unplaceable(TAKES), at: 4 })
        );
    }

    #[test]
    fn a_tree_with_more_occupants_than_one_frame_carries_is_refused() {
        // Not a defect in the tree. A two-hour sequence has thousands of clips
        // and this format carries sixty-four, so an application with more is
        // declaring a window of its content rather than all of it — and until
        // it does, the honest answer is a refusal rather than a canvas missing
        // everything past the sixty-fourth.
        let mut tree = timeline();
        tree.crowded = true;
        let (entries, written) = frame(&complete());
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::TooManyPlacements, at: 4 })
        );
    }

    #[test]
    fn an_occupant_placed_twice_is_refused_where_it_is_written_and_where_it_is_read() {
        // The builder is the same builder on both sides, which is the property
        // being asserted: a peer cannot state something its own author's build
        // would have refused, because the receiver assembles the arriving
        // frame through the same constructor.
        assert_eq!(
            complete().with_placement(Placement::over(DIALOGUE, span_x10(0, 1))),
            Err(Escape::PlacedTwice(DIALOGUE))
        );

        let tree = timeline();
        let (mut entries, written) = frame(&complete());
        // The peer re-sends the first placement rather than the commit, which
        // is a frame no builder on its side would have produced.
        entries[written - 1] = entries[1];
        let mut reception = Reception::opening(1);
        assert_eq!(
            offer(&mut reception, &tree, &entries[..written]),
            Err(Fault { escape: Escape::PlacedTwice(DIALOGUE), at: 4 })
        );
    }

    #[test]
    fn a_frame_that_never_declared_a_canvas_refuses_its_placements() {
        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);
        // Skip the declaration and offer a placement first.
        assert_eq!(
            reception.accept(&entries[1].entry, &entries[1].payload, &tree),
            Err(Fault { escape: Escape::NotDeclared, at: 0 })
        );

        // And a second canvas inside one frame, which is the other order rule:
        // there is no entry in this format that could say which placements
        // belonged to which canvas.
        let mut reception = Reception::opening(1);
        assert_eq!(
            reception.accept(&entries[0].entry, &entries[0].payload, &tree),
            Ok(Progress::Assembling)
        );
        assert_eq!(
            reception.accept(&entries[0].entry, &entries[0].payload, &tree),
            Err(Fault { escape: Escape::Redeclared(SEQUENCE), at: 1 })
        );
    }

    #[test]
    fn every_entry_after_a_refusal_is_refused_with_the_first_fault() {
        // The first fault is the cause and the rest are consequences of having
        // decoded past it, so a producer chasing its own bug is told where the
        // frame went wrong rather than where it noticed.
        let tree = timeline();
        let (mut entries, written) = frame(&complete());
        entries[1].entry.deadline = 1;
        let first = Fault { escape: Escape::Reserved, at: 1 };
        let mut reception = Reception::opening(1);
        assert_eq!(
            reception.accept(&entries[0].entry, &entries[0].payload, &tree),
            Ok(Progress::Assembling)
        );
        for submission in &entries[1..written] {
            assert_eq!(
                reception.accept(&submission.entry, &submission.payload, &tree),
                Err(first),
                "every entry of a poisoned frame is refused with the first fault"
            );
        }

        // The commit cleared the poison, so the next frame starts from nothing
        // — including from no half-declared canvas.
        assert!(reception.poisoned().is_none());
        assert!(!reception.assembling());
        let (entries, written) = frame(&complete());
        assert!(matches!(
            offer(&mut reception, &tree, &entries[..written]),
            Ok(Progress::Participating(_))
        ));
    }

    #[test]
    fn an_epoch_move_discards_a_half_assembled_canvas() {
        // The placements that would have completed the frame belonged to a peer
        // that is gone. Admitting them alongside a new peer's would be
        // assembling one canvas out of two processes.
        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);
        assert_eq!(
            reception.accept(&entries[0].entry, &entries[0].payload, &tree),
            Ok(Progress::Assembling)
        );
        assert!(reception.assembling());
        assert!(reception.follow_epoch(2));
        assert!(!reception.assembling());
        assert!(!reception.follow_epoch(2), "the same epoch discards nothing");
        assert_eq!(
            reception.accept(&entries[1].entry, &entries[1].payload, &tree),
            Err(Fault { escape: Escape::NotDeclared, at: 0 })
        );
    }

    #[test]
    fn a_field_this_format_does_not_read_is_refused() {
        // The envelope half of *an unread field is refused*, asserted field by
        // field rather than by trusting the comparison. Each of these is a peer
        // that filled in something no participation opcode reads, and each is
        // `Reserved` because that is what it is.
        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);

        for spoil in [
            |entry: &mut Sqe| entry.deadline = 1,
            |entry: &mut Sqe| entry.cap = 1,
            |entry: &mut Sqe| entry.buf_set = 1,
            |entry: &mut Sqe| entry.buf_index = 1,
            |entry: &mut Sqe| entry._reserved = 1,
            |entry: &mut Sqe| entry.ext = [1, 0],
        ] {
            let mut submission = entries[0];
            spoil(&mut submission.entry);
            assert_eq!(
                reception.accept(&submission.entry, &submission.payload, &tree),
                Err(Fault { escape: Escape::Reserved, at: 0 })
            );
            reception = Reception::opening(1);
        }

        // The payload half: a byte past what the record read. The commit is the
        // case worth writing down, because its record reads nothing at all, so
        // its whole payload is the tail.
        let mut submission = entries[4];
        submission.payload[0] = 1;
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::Reserved, at: 0 })
        );

        // And a field a record's own discriminant does not read, which is not
        // in the tail because the decoder consumed it to get past it.
        let mut reception = Reception::opening(1);
        let mut submission = entries[3];
        submission.payload[Place::WIDTH - 1] = 1;
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::Reserved, at: 0 })
        );
    }

    #[test]
    fn an_unknown_opcode_and_an_unknown_flag_are_refused_rather_than_ignored() {
        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);

        let mut submission = entries[0];
        submission.entry.opcode = 0x7f;
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::UnknownOpcode, at: 0 })
        );

        let mut reception = Reception::opening(1);
        let mut submission = entries[0];
        submission.entry.flags = flags::LINK;
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::UnknownFlag, at: 0 })
        );

        // A zeroed entry, which is what an untouched slot of a fresh mapping
        // holds, names no opcode.
        assert_eq!(op::closes(0), None);
        assert!(!op::ALL.contains(&0));
        assert_eq!(op::label(0), "unknown");
    }

    #[test]
    fn a_dimension_with_no_scale_and_a_span_that_does_not_advance_are_refused() {
        // Clause 2, refused at the decode because it is a statement one entry
        // makes on its own. Neither can be built either — `Base::new` and
        // `Extent::new` are the only constructors — so these are the wire being
        // held to what the type already refuses.
        assert_eq!(Base::new(0), Err(Escape::NotATimeBase));
        assert_eq!(Extent::new(Ticks::new(5), Ticks::new(5)), Err(Escape::NotAnExtent));
        assert_eq!(Extent::new(Ticks::new(5), Ticks::new(4)), Err(Escape::NotAnExtent));

        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);
        let mut submission = entries[0];
        // The base sits one `u64` into the declaration's payload.
        submission.payload[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::NotATimeBase, at: 0 })
        );

        // And a placement whose stretch does not advance: the occupant, then
        // the shape byte, then two eight-byte counts.
        let mut reception = Reception::opening(1);
        let mut submission = entries[1];
        submission.payload[9..17].copy_from_slice(&0i64.to_le_bytes());
        submission.payload[17..25].copy_from_slice(&0i64.to_le_bytes());
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::NotAnExtent, at: 0 })
        );
    }

    #[test]
    fn a_field_that_must_name_a_node_and_names_none_is_told_so() {
        // A zeroed payload produces exactly this, and a caller chasing a
        // producer that forgot to fill a record wants to be told that rather
        // than *some field is wrong*.
        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);

        let mut submission = entries[0];
        submission.payload[..8].copy_from_slice(&NO_NODE.to_le_bytes());
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::NoNode, at: 0 })
        );

        let mut reception = Reception::opening(1);
        let mut submission = entries[1];
        submission.payload[..8].copy_from_slice(&NO_NODE.to_le_bytes());
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::NoNode, at: 0 })
        );
    }

    #[test]
    fn a_shape_byte_naming_no_shape_is_refused() {
        // The one closed field this format declares of its own, and it has two
        // values. A third is an ABI change; a third arriving from a peer is a
        // peer this build cannot read.
        let tree = timeline();
        let (entries, _) = frame(&complete());
        let mut reception = Reception::opening(1);
        let mut submission = entries[1];
        submission.payload[8] = 2;
        assert_eq!(
            reception.accept(&submission.entry, &submission.payload, &tree),
            Err(Fault { escape: Escape::Malformed, at: 0 })
        );
    }

    #[test]
    fn every_opcode_round_trips_through_the_envelope_it_declares() {
        // The corpus is `Entry::specimens`, which is emitted from the same list
        // as the opcodes — so a fourth opcode is round-tripped here on the day
        // it is declared, without anybody remembering to add it.
        for body in Entry::specimens() {
            let (entry, payload) = crossing(body);
            let back = Crossing::decode(&entry, &payload).expect("a specimen this build wrote");
            assert_eq!(
                back.body,
                body,
                "{} does not survive its own encoder",
                op::label(entry.opcode)
            );
            assert_eq!(entry.len as usize, PAYLOAD_BYTES);
            assert_eq!(entry.deadline, NO_DEADLINE);
            assert_eq!(entry.cap, 0);
        }
        assert_eq!(Entry::specimens().len(), op::COUNT);
    }

    #[test]
    fn no_record_writes_past_its_declared_width() {
        // What `Writer::put` indexes without checking rests on: every byte a
        // record writes is inside its own `WIDTH`, and every byte past it is
        // the tail the reader refuses. A record whose writer outgrew its width
        // would corrupt the next field of a payload rather than fail.
        for body in Entry::specimens() {
            let payload = body.payload();
            for (at, byte) in payload.iter().enumerate().skip(body.width()) {
                assert_eq!(*byte, 0, "{} writes past its width at {at}", op::label(body.opcode()));
            }
        }
    }

    #[test]
    fn a_declaration_offered_less_room_than_it_needs_is_refused_rather_than_truncated() {
        // A frame that stopped halfway would be a canvas that places fewer
        // occupants than it has, which is the escape this module exists to
        // refuse, produced by the encoder rather than by a peer.
        let arrangement = complete();
        let mut cramped = [Submission::UNSENT; 4];
        assert!(matches!(arrangement.emit(carriage(), &mut cramped), Err(Escape::NoRoom)));
        let mut exact = [Submission::UNSENT; 5];
        assert_eq!(arrangement.emit(carriage(), &mut exact), Ok(5));

        // And a producer that asked for a flag no participation entry carries
        // learns before it submits rather than in a completion.
        let mut room = [Submission::UNSENT; 8];
        let bad = Carriage { flags: flags::DRAIN, ..carriage() };
        assert_eq!(arrangement.emit(bad, &mut room), Err(Escape::UnknownFlag));
    }

    #[test]
    fn each_entry_of_a_frame_points_at_its_own_slot_of_the_arena() {
        // The stride is the format's, and a producer that laid its payloads out
        // any other way would be pointing two entries at one slot.
        let (entries, written) = frame(&complete());
        for (at, submission) in entries[..written].iter().enumerate() {
            let expected = carriage().payload_offset as u64 + (at * PAYLOAD_BYTES) as u64;
            assert_eq!(submission.entry.offset, expected);
        }
    }

    #[test]
    fn every_refusal_packs_into_a_code_the_abi_already_has() {
        // No code here is new, which is the decision `Escape::packed` records.
        // The assertion is that every one of them is a refusal — a negative
        // result in a domain RFC 0010 fixed — rather than a success value some
        // caller would read as a completion.
        for (at, label) in Escape::LABELS.iter().enumerate() {
            assert!(!label.is_empty(), "the refusal at {at} has no name");
        }
        assert_eq!(Escape::LABELS.len(), Escape::COUNT);

        // The list below is written out by hand, because a refusal that carries
        // a node needs a node and there is no way to build one from a name. So
        // it is held to the emitted list rather than trusted: the count must
        // match, and every name the macro emitted must be somewhere in it. A
        // refusal added to `escapes!` and forgotten here fails on the first
        // assertion; one listed twice instead of a sibling fails on the second.
        // Without this pair the list is precisely the hand-written array this
        // whole file is written against — green, and about eighteen of
        // nineteen.
        let listed = [
            Escape::NotATimeBase,
            Escape::NotAnExtent,
            Escape::TooManyPlacements,
            Escape::PlacedTwice(1),
            Escape::NotACanvas(1),
            Escape::NotALane(1),
            Escape::NotAnOccupant(1),
            Escape::Mistimed(1),
            Escape::Unplaced(1),
            Escape::NothingPlaced(1),
            Escape::Unplaceable(1),
            Escape::NoRoom,
            Escape::UnknownOpcode,
            Escape::UnknownFlag,
            Escape::Reserved,
            Escape::Malformed,
            Escape::NoNode,
            Escape::NotDeclared,
            Escape::Redeclared(1),
        ];
        assert_eq!(
            listed.len(),
            Escape::COUNT,
            "a refusal was added to `escapes!` and this test was not given it"
        );
        for label in Escape::LABELS {
            assert!(
                listed.iter().any(|escape| escape.label() == label),
                "`{label}` is emitted by `escapes!` and is not in this test's list"
            );
        }

        for escape in listed {
            let packed = escape.packed();
            assert!(packed < 0, "{} packs into a success value", escape.label());
            let (domain, _) = error::unpack(packed).expect("a refusal is negative");
            assert_eq!(domain, error::ARGUMENT, "{} is not an argument refusal", escape.label());
            assert!(!escape.message().is_empty(), "{} has no message", escape.label());
        }
    }

    /// The variant names `interface`'s `Escape` declares, in order.
    ///
    /// A parse rather than a copy. It fails loudly — a panic is a red test —
    /// rather than skipping a line it does not understand, because a parser
    /// that silently skips is a parser that reports a shorter list and an
    /// agreement over the wrong thing.
    fn declared_escapes() -> ([&'static str; ESCAPE_LIMIT], usize) {
        let mut names = [""; ESCAPE_LIMIT];
        let mut found = 0;
        let mut inside = false;
        for line in CANVAS.lines() {
            if !inside {
                inside = line == "pub enum Escape {";
                continue;
            }
            if line == "}" {
                return (names, found);
            }
            let text = line.trim();
            if text.is_empty() || text.starts_with("//") {
                continue;
            }
            let name = text.split(['(', ',', '{', ' ']).next().unwrap_or("");
            assert!(
                name.starts_with(|c: char| c.is_ascii_uppercase()),
                "a line of `interface`'s `Escape` is neither a doc comment nor a variant: \
                 `{text}`"
            );
            assert!(found < ESCAPE_LIMIT, "`interface`'s `Escape` has outgrown this parser");
            names[found] = name;
            found += 1;
        }
        panic!("`pub enum Escape {{` never closes in interface/src/canvas.rs")
    }

    /// A `usize` constant `interface/src/canvas.rs` declares, read out of it.
    fn declared_usize(name: &str) -> usize {
        let needle = "pub const ";
        let mut from = 0;
        while let Some(at) = CANVAS[from..].find(needle) {
            let start = from + at + needle.len();
            let rest = &CANVAS[start..];
            from = start;
            let Some(eq) = rest.find('=') else { continue };
            if !rest[..eq].starts_with(name) {
                continue;
            }
            let Some(end) = rest[eq..].find(';') else { continue };
            let value = rest[eq + 1..eq + end].trim();
            return value.parse().unwrap_or_else(|_| panic!("`{name}` is not a number: `{value}`"));
        }
        panic!("interface/src/canvas.rs declares no `{name}`")
    }
}
