// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Timeline semaphores: what a producer has undertaken to signal, and the wait
//! that is refused because nothing will ever keep it.
//!
//! # The chain, and why it is two timelines rather than three
//!
//! `docs/design/ring-scene-boot.html` section 08 states the whole mechanism in
//! one sentence: *the application signals value N when its deltas are ready;
//! the compositor waits N and signals M when composited; the present engine
//! waits M.* [`Chain`] is that sentence as a type, and the first thing it makes
//! visible is that the chain has three stages and two timelines — the present
//! engine is a waiter and signals nothing, so it owns no timeline, and a design
//! that gave it one would have invented a value nobody reads.
//!
//! A timeline is a `u64` that only ever goes up, with **one producer**. That is
//! not a simplification of the Vulkan object it is named after; it is the
//! premise everything below rests on. A timeline with two producers has two
//! parties who can each promise, so *what this timeline could ever signal* is
//! no longer one party's answer, and the refusal this module exists for stops
//! being decidable. *What would reverse this:* a stage whose signal genuinely
//! comes from either of two components — at which point the honest change is a
//! second timeline and a wait on both, not a second writer on one.
//!
//! # The failure this module is shaped around
//!
//! A wait has exactly three outcomes, and only one of them is a disaster:
//!
//! - **At or below what has already been signalled.** Satisfied before the
//!   waiter runs. Not a wait at all, and nothing here refuses it.
//! - **Above that, and at or below what the producer has undertaken to
//!   signal.** A real wait. It blocks, the producer reaches the value, it
//!   proceeds. This is the mechanism working.
//! - **Above anything any producer could ever signal.** A hang. The waiter
//!   blocks on a value that will never arrive, and — this is the whole of the
//!   argument — *a hang writes no log line*. Every other failure in this tree
//!   leaves a completion with a packed [`error`] in it. This one leaves a
//!   process that is simply still there, and a frame that never appears, and a
//!   bug report that says the screen froze.
//!
//! So the third case is refused where a submission is *built*, not detected
//! where it is waited on. That is only possible if *what this timeline could
//! ever signal* is in the record: [`Timeline::ceiling`] is the highest value
//! its producer has undertaken to signal, [`Timeline::promise`] is how it is
//! raised, and [`Timeline::wait`] refuses a value above it with
//! [`Refusal::Unreachable`].
//!
//! # Why the refusal is a type and not a check
//!
//! A check is a line somebody can stop calling. [`Wait`] and [`Signal`] have
//! private fields and live in a private module with exactly one constructor
//! each, and those constructors take the [`Timeline`] the value is against.
//! There is no other way to obtain either value — not from outside this crate,
//! where the fields are unreachable, and not from the rest of this file, where
//! the module boundary is what holds. [`Submission::waiting`] takes a [`Wait`]
//! rather than a number, so a submission carrying an unreachable wait is not a
//! submission this build refuses; it is a value that cannot be constructed.
//! [`Submission::decode`] reaches the same constructor, so a peer's bytes are
//! admitted by the same function a local caller is, and there is no second
//! statement of the rule to fall out of step with the first. What it *returns*
//! is [`Arrived`] rather than a [`Submission`], because a driver is a receiver
//! and a receiver holding the builder is a receiver that can add a wait the
//! client never asked for — `E3-B05c`, RFC 0111, and the type's own paragraph
//! is where the argument lives.
//!
//! That is the strongest form available, and it is worth saying what it is not:
//! it is not a proof that the *number* is right, only that some timeline
//! admitted it. A caller that hands [`Timeline::wait`] a stale timeline gets a
//! wait checked against a stale ceiling. The ceiling never falls
//! ([`Timeline::promise`] refuses that with [`Refusal::NotMonotonic`]),
//! precisely so that a stale ceiling is *lower* than the true one and the
//! mistake is a spurious refusal rather than an admitted hang.
//!
//! # What this cannot catch, said plainly
//!
//! A producer that promises a value and then dies before reaching it leaves a
//! waiter blocked on a value that was reachable when it was admitted and is not
//! any more. No submission-time check can see that, because at submission time
//! the promise was good. That failure belongs to `E3-B05e` — a timeout is a
//! named fate and the supervisor decides it — and this module does not pretend
//! to cover it. Nor does this module catch a cycle across two submissions: two
//! stages each waiting on a value the other has promised are both admitted
//! here, both reachable in isolation, and deadlocked together. That needs the
//! graph and not the record, and the honest statement of what is bought here is
//! narrow: **a wait that could never have been satisfied by anybody is refused
//! before it is submitted.** The waits that *could* have been satisfied and
//! were not are a timeout's business.
//!
//! # Fixed width, and where it is stated
//!
//! The exit asks for fixed width on both architectures, so the widths are in
//! the function signatures rather than in a test: [`Timeline::encode`] returns
//! `[u8; TIMELINE_BYTES]` and [`Submission::encode`] returns
//! `[u8; SUBMISSION_BYTES]`, and the decoders take references to arrays of
//! exactly those widths. A record that changed width would not fail a test; it
//! would fail to compile at every call site.
//!
//! Every field is a `u32` or a `u64` written through `to_le_bytes` and read
//! back through `from_le_bytes`, by hand, in [`crate::store`]'s discipline — no
//! `usize` reaches the wire, nothing is cast to a struct, and no field's
//! position depends on the host's alignment rules or word size. Both
//! architectures this tree targets are little-endian, so what the AArch64 job
//! establishes here is the alignment and word-size half; the byte order half is
//! established by the encoding being explicit rather than by the host agreeing.
//!
//! There is no floating point anywhere in this module. A timeline value is an
//! ordinal and a count of nothing, so it has no scale in its name — it is a
//! `u64` because that is what it is, not because a fixed point was rounded off.
//! RFC 0004.
//!
//! # Why the refusals are local
//!
//! [`Refusal`] is this module's own enum, in the shape
//! [`scene::Refusal`](crate::scene::Refusal) and
//! [`manifest::Refusal`](crate::manifest::Refusal) already use, and it packs
//! into the domains RFC 0010 fixes without adding a code to [`error::argument`].
//! Every refusal below reuses a code that already means what it says. The
//! temptation here is a code for *unreachable*, because it is the interesting
//! one; it is declined for `scene.rs`'s reason, which is that four sibling
//! formats are being written against that file this week and a tenth `argument`
//! code invented in four places is four meanings wearing one number. The day
//! this genuinely cannot reuse one, `store::code`'s note is the precedent for
//! how a new code is numbered.
//!
//! # An eleventh refusal
//!
//! The `refusals!` invocation below emits the variants, their messages, their
//! packed codes and [`Refusal::ALL`] from one list, for `interface/src/node.rs`
//! `vocabulary!`'s reason: a hand-written `ALL` is a list a new variant does not
//! appear in, and an exhaustive `match` is a match a new variant can walk
//! through on an arm that says nothing. Here the arm *is* the declaration, and
//! a message that said nothing would not compile — the macro asserts each one
//! non-empty at compile time.
//!
//! The eleventh is the evidence that was owed. [`Refusal::NoStage`] is
//! [`crate::trace`]'s and not this module's, it arrived from a different file
//! for a different record, and it cost **one declaration** — no arm in a
//! `match`, no row in an `ALL`, no line in a test. The refusals live here rather
//! than in a second enum of the same shape because the subject is the same: a
//! trace entry is a wait that already happened, admitted by
//! [`Timeline::wait`], so a second list of reasons a wait is not believed would
//! be a second answer to one question.

use crate::error;
use crate::trace::{Stage, Trace};

/// No timeline. Zero, so that a zeroed record names nothing.
///
/// Every field that must name a timeline refuses it, which is what stops an
/// untouched slot of a fresh mapping from decoding as *timeline zero, waiting
/// for nothing*.
/// Unit: none — a timeline identifier, not a quantity.
pub const NO_TIMELINE: u32 = 0;

/// The value every timeline starts at, and the value that is not a value.
///
/// A timeline that has signalled nothing has signalled `UNSIGNALLED`, so a wait
/// on it would be satisfied by every timeline that ever existed and a signal of
/// it would move none. Both are refused with [`Refusal::NoValue`] rather than
/// accepted as no-ops, because a producer that submitted one meant something
/// else and would never find out.
/// Unit: none — a timeline value is an ordinal, not a quantity with a scale.
pub const UNSIGNALLED: u64 = 0;

/// Bytes a published [`Timeline`] record occupies.
///
/// Three fields — a value, a ceiling and an identifier — at 8, 8 and 4, rounded
/// to the next multiple of eight so that a table of them has the same alignment
/// in every slot. The assertion at the foot of this file, against the sum of the
/// field widths, is what keeps that a fact rather than a coincidence.
/// Unit: bytes.
pub const TIMELINE_BYTES: usize = 24;

/// Bytes one wait or one signal occupies inside a [`Submission`].
///
/// One width for both, so a submission is an array with a stride rather than a
/// list that must be parsed to be walked — [`scene`](crate::scene)'s argument
/// for one payload width, and it applies here for the same three reasons: a
/// consumer draining a hostile peer can bound its walk before it believes any
/// field, a producer can fill slot *n* without having finished slot *n − 1*, and
/// the boundaries are arithmetic.
/// Unit: bytes.
pub const SLOT_BYTES: usize = 16;

/// Bytes a [`Submission`] occupies, whatever it carries.
///
/// Two cache lines. The record is written into a channel's inline arena beside
/// the scene deltas it synchronises, and the width is fixed for the same reason
/// theirs is; two lines is the smallest power-of-two multiple of a line that
/// holds the chain section 08 names with room for a compositor waiting on
/// several clients at once. [`MAX_WAITS`] then falls out of this number rather
/// than being chosen beside it, so there is no second place the capacity is
/// written.
///
/// *What would reverse this:* a stage that must wait on more than [`MAX_WAITS`]
/// timelines in one submission — measured, not supposed. The repair is not a
/// wider record: it is the waits moving to a counted range in the arena, the
/// shape [`scene::SetPath`](crate::scene::SetPath) already uses for geometry, at
/// which point the record stops being self-contained and a reader needs the
/// mapping to believe it.
/// Unit: bytes.
pub const SUBMISSION_BYTES: usize = 128;

/// The [`Submission`] header: two counts, and the bytes after them that this
/// build does not read.
/// Unit: bytes.
const HEADER_BYTES: usize = 8;

/// Where the wait slots start.
/// Unit: bytes from the start of the record.
const WAITS_AT: usize = HEADER_BYTES;

/// How many waits one submission frames.
///
/// Derived from [`SUBMISSION_BYTES`] and not chosen: the header, then as many
/// wait slots as fit beside the one signal slot. A submission that needs more
/// is the reversal condition on [`SUBMISSION_BYTES`], stated there.
/// Unit: waits.
pub const MAX_WAITS: usize = (SUBMISSION_BYTES - HEADER_BYTES - SLOT_BYTES) / SLOT_BYTES;

/// Where the one signal slot starts.
/// Unit: bytes from the start of the record.
const SIGNAL_AT: usize = WAITS_AT + MAX_WAITS * SLOT_BYTES;

/// Where the bytes this build does not read start.
/// Unit: bytes from the start of the record.
const TAIL_AT: usize = SIGNAL_AT + SLOT_BYTES;

// The record holds what it says it holds, and there is a tail after it. The
// tail is not slack: it is where a field added to this record would go, and
// `decode`'s whole-image comparison is what makes a peer that already has that
// field visible to a build that does not. If an edit ever drives this to zero,
// the next field is a width change and an RFC rather than a quiet reuse.
const _: () = assert!(TAIL_AT < SUBMISSION_BYTES);
// One signal, and the argument is in `Submission::signal`: a submission is one
// stage of the chain and a timeline has one producer, so the only timeline a
// submission may signal is its own.
const _: () = assert!(SUBMISSION_BYTES - TAIL_AT == 8);

/// Why a record or a value was not believed.
///
/// A value rather than a packed integer, for
/// [`store::refusal`](crate::store::refusal)'s reason: a caller compares against
/// these, and a test that spelled the packed integer out itself would pass on a
/// decoder that returned the right refusal for the wrong reason.
/// [`Refusal::packed`] is the one place the mapping to RFC 0010's domains is
/// written.
///
/// # Why a macro, in a tree that mostly refuses them
///
/// Because the alternative is four sequences that have to agree — the variants,
/// their messages, their codes and the array a test iterates — and this
/// project's own record is that they stop agreeing silently. The emitted
/// [`Refusal::ALL`] holds every variant because there is no way to declare one
/// it misses, and a message is on the line that declares the variant, so an arm
/// that said nothing would be a thing somebody wrote rather than a thing
/// somebody forgot. The compile-time assertion under each declaration is what
/// makes *said nothing* not compile.
macro_rules! refusals {
    (
        $(
            $(#[$about:meta])*
            $variant:ident => $code:expr, $message:literal;
        )*
    ) => {
        /// Why a record or a value was not believed.
        ///
        /// Emitted, with its messages and its codes, from the one list the
        /// `refusals!` invocation below holds — see that macro for why this
        /// module has one at all.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Refusal {
            $(
                $(#[$about])*
                $variant,
            )*
        }

        $(
            // A refusal with nothing to say is a refusal a caller cannot act
            // on, and this is that rule where it cannot be walked past.
            const _: () = assert!(!$message.is_empty());
        )*

        impl Refusal {
            /// Every refusal this module can return, in declaration order.
            ///
            /// Emitted from the same list as the variants, so it holds every
            /// one there is — not because a test checks it, but because there
            /// is no way to declare one this array does not get.
            /// Unit: none — refusals.
            pub const ALL: [Self; [$(stringify!($variant)),*].len()] = [$(Self::$variant),*];

            /// A line for a log.
            #[must_use]
            pub const fn message(self) -> &'static str {
                match self {
                    $(Self::$variant => $message,)*
                }
            }

            /// The refusal as a packed [`error`], for a completion.
            ///
            /// Every one is [`error::ARGUMENT`]: the channel is healthy and the
            /// peer is present, and what is wrong is one record. The domain is
            /// a literal outside the match rather than a column inside it, so
            /// *every refusal is an argument error* is a property of this
            /// function's shape and not of an arm somebody remembered to write
            /// correctly.
            #[must_use]
            pub const fn packed(self) -> i32 {
                error::pack(error::ARGUMENT, match self {
                    $(Self::$variant => $code,)*
                })
            }
        }
    };
}

refusals! {
    /// A field that must name a timeline holds [`NO_TIMELINE`]. Separate from
    /// the others because a zeroed record produces exactly this, and a caller
    /// chasing a producer that forgot to fill one wants to be told that rather
    /// than *some field is wrong*.
    NoTimeline => error::argument::UNKNOWN_FLAG,
        "a field that must name a timeline names none";

    /// A wait or a signal on [`UNSIGNALLED`], which is where every timeline
    /// starts: a wait on it is satisfied by everything and a signal of it moves
    /// nothing, so neither is what its submitter meant.
    NoValue => error::argument::UNKNOWN_FLAG,
        "a wait or a signal names the value every timeline starts at";

    /// **The refusal this module exists for.** A wait above the highest value
    /// the timeline's producer has undertaken to signal. Nothing will ever
    /// satisfy it, and a waiter that was allowed to block on it would leave no
    /// evidence at all — see the module's *the failure this module is shaped
    /// around*.
    Unreachable => error::argument::UNKNOWN_FLAG,
        "a wait names a value above anything its timeline has undertaken to signal";

    /// A wait or a signal names a timeline the receiver does not hold, so its
    /// ceiling is unknown. Refused for [`Refusal::Unreachable`]'s reason
    /// exactly: an unknown ceiling and an exceeded one are the same ignorance
    /// about whether anything will ever arrive, and admitting the first while
    /// refusing the second would make the guarantee depend on which one a peer
    /// happened to send.
    NoSuchTimeline => error::argument::UNKNOWN_FLAG,
        "a wait or a signal names a timeline this side does not hold";

    /// A number that does not move forward: a ceiling that falls, a signal at
    /// or below what has already landed, or a landing that does not advance.
    /// A timeline that could go back would turn an already-admitted wait into a
    /// hang after the fact, which is the one thing the admission has to be
    /// worth.
    NotMonotonic => error::argument::UNKNOWN_FLAG,
        "a timeline value would move backwards, and a timeline only moves forward";

    /// A signal above the ceiling its own producer declared. Refused rather
    /// than treated as an implicit promise, because a ceiling that could be
    /// raised by the signal itself would be a promise made after the fact —
    /// and a consumer that had already been refused a wait on that value would
    /// have been refused for a reason that stopped being true without anybody
    /// telling it. [`Timeline::promise`] first, then signal.
    Overcommitted => error::argument::UNKNOWN_FLAG,
        "a signal names a value above what its own producer undertook to signal";

    /// A submission waits on the timeline it signals, at or above the value it
    /// signals. The waits of a submission are entered before its work runs and
    /// its signal happens after, and a timeline has one producer — so the only
    /// thing that could satisfy that wait is the submission the wait is
    /// blocking. Admitted, it is a hang with no evidence, which is this
    /// module's own failure wearing a different shape.
    SelfBlocked => error::argument::UNKNOWN_FLAG,
        "a submission waits on its own signal, which cannot arrive until it runs";

    /// Two waits on one timeline in one submission, or two signals. Two waits
    /// are two answers to *what value does this need*, of which only the higher
    /// is ever read; two signals are two producers' worth of writes from one
    /// stage. Both are refused rather than merged, because merging picks an
    /// answer the submitter did not write.
    Duplicate => error::argument::UNKNOWN_FLAG,
        "one timeline is named twice in one submission";

    /// More waits or signals than the record frames. In the same family as a
    /// malformed header and not as a resource limit: what is wrong is that the
    /// count does not describe a record of this width.
    Full => error::argument::MALFORMED_HEADER,
        "the record does not frame that many waits or signals";

    /// A field this build does not read is not zero: a byte past a record's own
    /// fields, an unused slot, or anything in the tail. The refusal the
    /// whole-image comparison in each decoder produces, and the one that makes
    /// a peer built later than this one visible rather than silently
    /// misunderstood. R04.
    Reserved => error::argument::RESERVED_NOT_ZERO,
        "a field this build does not read is not zero";

    /// A field that must name a stage of the chain names none, or names one this
    /// build does not have. [`crate::trace`]'s refusal rather than this
    /// module's, and it is declared *here* rather than in a second enum of the
    /// same shape because the subject is the same: a trace entry **is** a wait,
    /// admitted by the same constructor, and two lists over one subject are two
    /// lists that can disagree about what a wait is. It reuses a code that
    /// already means what it says, for the reason the module header gives for
    /// declining a tenth `argument` code.
    NoStage => error::argument::UNKNOWN_FLAG,
        "a field that must name a stage of the chain names none this build has";
}

/// The one place a [`Wait`] or a [`Signal`] is made.
///
/// A module and not merely private fields, and the difference is what this file
/// is accepted on. Private fields stop the rest of the workspace from building
/// one of these out of two numbers; a module boundary also stops the rest of
/// *this file* from doing it. Everything a wait or a signal has to be true of
/// is checked in the two constructors below, and there is no third way in — so
/// the existence of one of these values is itself the evidence that some
/// timeline admitted it.
///
/// A later edit that wants a shortcut has to add a constructor here, on the
/// same screen as this paragraph. That is the most a Rust module can do to make
/// a rule structural, and it is deliberately more than a check at a call site
/// somebody can stop making.
mod proof {
    use super::{Refusal, SUBMISSION_BYTES, Submission, Timeline, UNSIGNALLED};

    /// A wait on a timeline value, which exists only because that timeline
    /// admitted it.
    ///
    /// Carrying no lifetime and no reference to the timeline is deliberate:
    /// this is a value a submission holds and a record encodes, and the ceiling
    /// it was checked against can only rise afterwards ([`Timeline::promise`]
    /// refuses a fall), so an admitted wait stays admitted for as long as the
    /// timeline exists.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Wait {
        /// The timeline waited on.
        timeline: u32,
        /// The value waited for.
        value: u64,
    }

    impl Wait {
        /// Admit a wait against the timeline that would satisfy it.
        ///
        /// The whole of the exit's second clause, in one function with one
        /// caller-visible door: [`Timeline::wait`].
        ///
        /// # Errors
        ///
        /// [`Refusal::NoValue`] for [`UNSIGNALLED`], which every timeline has
        /// already passed. [`Refusal::Unreachable`] for a value above the
        /// ceiling its producer has undertaken to reach — the refusal that
        /// replaces a hang with a completion.
        pub(super) const fn admitted(against: &Timeline, value: u64) -> Result<Self, Refusal> {
            if value == UNSIGNALLED {
                return Err(Refusal::NoValue);
            }
            if value > against.ceiling() {
                return Err(Refusal::Unreachable);
            }
            // A value at or below what has already been signalled is *not*
            // refused and is not a mistake: it is a wait that is satisfied the
            // moment it is entered, which is what a stage submits when it is
            // already behind. Refusing it would make a correct producer's
            // ordinary case an error.
            Ok(Self { timeline: against.id(), value })
        }

        /// The timeline this waits on.
        #[must_use]
        pub const fn timeline(&self) -> u32 {
            self.timeline
        }

        /// The value this waits for.
        #[must_use]
        pub const fn value(&self) -> u64 {
            self.value
        }

        /// Is this wait already satisfied by `timeline` as it stands?
        ///
        /// `false` when the identifiers differ, because a wait is not satisfied
        /// by a timeline that is not its own — the question is meaningless
        /// rather than affirmative, and returning `true` on a mismatch is the
        /// shape of bug that ends with a frame presented before its deltas
        /// landed.
        #[must_use]
        pub const fn satisfied_by(&self, timeline: &Timeline) -> bool {
            timeline.id() == self.timeline && self.value <= timeline.signalled()
        }
    }

    /// An undertaking to move a timeline to a value, which exists only because
    /// that timeline's own producer made it.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Signal {
        /// The timeline signalled.
        timeline: u32,
        /// The value it is moved to.
        value: u64,
    }

    impl Signal {
        /// Undertake to move `by` to `value`.
        ///
        /// # Errors
        ///
        /// [`Refusal::NoValue`] for [`UNSIGNALLED`]. [`Refusal::NotMonotonic`]
        /// for a value the timeline has already reached, which would be a
        /// signal that signals nothing. [`Refusal::Overcommitted`] for a value
        /// above the producer's own declared ceiling — the producer raises the
        /// ceiling first, where a waiter can see it, and signals second.
        pub(super) const fn undertaken(by: &Timeline, value: u64) -> Result<Self, Refusal> {
            if value == UNSIGNALLED {
                return Err(Refusal::NoValue);
            }
            if value <= by.signalled() {
                return Err(Refusal::NotMonotonic);
            }
            if value > by.ceiling() {
                return Err(Refusal::Overcommitted);
            }
            Ok(Self { timeline: by.id(), value })
        }

        /// The timeline this signals.
        #[must_use]
        pub const fn timeline(&self) -> u32 {
            self.timeline
        }

        /// The value it is moved to.
        #[must_use]
        pub const fn value(&self) -> u64 {
            self.value
        }
    }

    /// A submission as a peer sent it: a record, and not a builder.
    ///
    /// # What this is for, in one sentence
    ///
    /// A driver is a *receiver*. [`Submission::decode`] returns this rather than
    /// a [`Submission`] so that the value a receiver holds has **no constructor
    /// that adds a wait to it** — no `waiting`, no `signalling`, and no way
    /// back to the builder the bytes were admitted through. `E3-B05c`'s exit asks
    /// for a submission that *refuses one that would let the driver insert its
    /// own*, and the thing that would have let it was not a missing check: it
    /// was that the decoded value was still a builder.
    ///
    /// # Why this is stronger than the check it replaces
    ///
    /// A check that refused an extra wait would have to run somewhere, on
    /// something, at a moment a driver could be past. This has no moment. A
    /// receiver that wanted to wait on one more timeline has to build a *new*
    /// submission, which is a new record with its own bytes, its own admissions
    /// and its own entry in a frame trace — visible, in other words, which is
    /// the whole of what the parent task's negative asks for. What is not
    /// available is the thing implicit synchronisation actually is: a wait
    /// appearing inside the submission the client wrote, with the client's own
    /// bytes still around it.
    ///
    /// # What it does not buy, said plainly
    ///
    /// Nothing here reaches an imported driver's internals. A third-party driver
    /// behind the licence boundary can do as it likes with the hardware and this
    /// tree cannot read its trace — `E3-B05c`'s exit says as much, and calls
    /// this the half that still holds against such a driver. What holds is that
    /// the *record* it was handed says what the client asked for and nothing
    /// else, and that a wait it added is not expressible as the client having
    /// asked for it. RFC 0111.
    ///
    /// # The two things that do not compile
    ///
    /// `ring/src/buffers.rs` is the precedent: a property the type system holds
    /// is asserted by a fixture that fails to compile, with the error code
    /// named, because a test that passes cannot tell *unrepresentable* apart
    /// from *not currently done*.
    ///
    /// A receiver cannot add a wait to what arrived:
    ///
    /// ```compile_fail,E0599
    /// use f_abi::sync::{Chain, Submission, Timeline};
    /// use f_abi::trace::Trace;
    /// let mut chain = Chain::new(
    ///     Timeline::declare(1).unwrap(),
    ///     Timeline::declare(2).unwrap(),
    /// )
    /// .unwrap();
    /// let mut trace = Trace::EMPTY;
    /// chain.application_signals(9).unwrap();
    /// let built = chain.compositor_waits_and_signals(9, 4, &mut trace).unwrap();
    /// let arrived = Submission::decode(&built.encode(), &chain.timelines()).unwrap();
    /// let more = chain.application().wait(9).unwrap();
    /// // The driver, inserting its own. There is no such function.
    /// let _ = arrived.waiting(more);
    /// ```
    ///
    /// And what arrived is not a [`Submission`], so no amount of passing it
    /// along reaches one:
    ///
    /// ```compile_fail,E0308
    /// use f_abi::sync::{Chain, Submission, Timeline};
    /// use f_abi::trace::Trace;
    /// let mut chain = Chain::new(
    ///     Timeline::declare(1).unwrap(),
    ///     Timeline::declare(2).unwrap(),
    /// )
    /// .unwrap();
    /// let mut trace = Trace::EMPTY;
    /// chain.application_signals(9).unwrap();
    /// let built = chain.compositor_waits_and_signals(9, 4, &mut trace).unwrap();
    /// let _: Submission = Submission::decode(&built.encode(), &chain.timelines()).unwrap();
    /// ```
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Arrived(Submission);

    impl Arrived {
        /// Seal a submission the decoder has just admitted.
        ///
        /// `pub(super)` and reached from exactly one place. There is deliberately
        /// no inverse: nothing in this crate or outside it turns an `Arrived`
        /// back into a [`Submission`], because that function would be the door
        /// this type exists to close, and a door with one caller is still a door.
        pub(super) const fn sealed(submission: Submission) -> Self {
            Self(submission)
        }

        /// How many waits this record carries.
        /// Unit: waits.
        #[must_use]
        pub const fn wait_count(&self) -> usize {
            self.0.wait_count()
        }

        /// The wait at `index`, or `None` for an index this record does not
        /// carry.
        ///
        /// Hands out a [`Wait`], which is safe to hand out for the reason the
        /// module above it exists: a `Wait` is evidence that some timeline
        /// admitted a value, and evidence is not a capability to add one.
        #[must_use]
        pub const fn wait(&self, index: usize) -> Option<Wait> {
            self.0.wait(index)
        }

        /// The signal this stage leaves, or `None` for a stage that leaves none.
        #[must_use]
        pub const fn signal(&self) -> Option<Signal> {
            self.0.signal()
        }

        /// The record as it crosses, unchanged.
        ///
        /// Present so that a receiver can forward exactly what it was handed —
        /// a supervisor persisting it, or a trace recording it — without the
        /// bytes going back through a builder on the way. Re-encoding an
        /// `Arrived` yields the image it was decoded from, which is what makes
        /// *unchanged* checkable rather than asserted.
        #[must_use]
        pub fn encode(&self) -> [u8; SUBMISSION_BYTES] {
            self.0.encode()
        }
    }

    /// Compare what arrived against what somebody built.
    ///
    /// Here rather than on the caller's side, and asymmetric on purpose: it lets
    /// a test say *the bytes decoded to the submission I built* without a
    /// conversion existing in either direction. A `From<Arrived> for Submission`
    /// would say the same thing and would also hand a receiver the builder.
    impl PartialEq<Submission> for Arrived {
        fn eq(&self, built: &Submission) -> bool {
            self.0 == *built
        }
    }
}

pub use proof::{Arrived, Signal, Wait};

/// One timeline, as its producer has published it.
///
/// Three numbers that only go up, and the third is the one that makes this
/// module's refusal possible at all. Fields are private because the three are
/// not independent — `signalled <= ceiling` always, and an identifier of
/// [`NO_TIMELINE`] names nothing — and a record a caller could assemble field by
/// field is a record a caller can assemble wrongly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timeline {
    /// Names this timeline. Never [`NO_TIMELINE`].
    id: u32,
    /// The highest value observed to have landed.
    signalled: u64,
    /// The highest value this timeline's producer has undertaken to signal.
    ceiling: u64,
}

impl Timeline {
    /// The bytes this record's fields occupy, before the padding to
    /// [`TIMELINE_BYTES`].
    /// Unit: bytes.
    const FIELD_BYTES: usize = 8 + 8 + 4;

    /// A timeline that has signalled nothing and promised nothing.
    ///
    /// Which means it admits **no wait at all** — every value is above a
    /// ceiling of [`UNSIGNALLED`]. That is the right starting state rather than
    /// an awkward one: a producer that has not said what it will signal has not
    /// said anything a consumer may block on, and the first thing it does is
    /// [`Timeline::promise`].
    ///
    /// # Errors
    ///
    /// [`Refusal::NoTimeline`] for [`NO_TIMELINE`].
    pub const fn declare(id: u32) -> Result<Self, Refusal> {
        Self::adopt(id, UNSIGNALLED, UNSIGNALLED)
    }

    /// A timeline as a peer published it, or as a state tree recorded it.
    ///
    /// The door [`Timeline::decode`] goes through, and the one a component
    /// restoring its own synchronisation state from `E3-B05f`'s subtree uses,
    /// so that a restored timeline is subject to the same two facts a fresh one
    /// is rather than to whatever was in the tree.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoTimeline`] for [`NO_TIMELINE`].
    /// [`Refusal::NotMonotonic`] for a timeline that has signalled past what it
    /// promised, which is a producer that has already broken the undertaking
    /// every wait against it was admitted on.
    pub const fn adopt(id: u32, signalled: u64, ceiling: u64) -> Result<Self, Refusal> {
        if id == NO_TIMELINE {
            return Err(Refusal::NoTimeline);
        }
        if signalled > ceiling {
            return Err(Refusal::NotMonotonic);
        }
        Ok(Self { id, signalled, ceiling })
    }

    /// Names this timeline.
    /// Unit: none — a timeline identifier, not a quantity.
    #[must_use]
    pub const fn id(&self) -> u32 {
        self.id
    }

    /// The highest value observed to have landed.
    ///
    /// A wait at or below this is satisfied the moment it is entered. Moved by
    /// [`Timeline::landed`] and never by submitting a signal, because submitting
    /// work is not the work having happened, and a field that moved at
    /// submission would say a frame was composited while it was still queued.
    /// Unit: none — a timeline value, which is an ordinal.
    #[must_use]
    pub const fn signalled(&self) -> u64 {
        self.signalled
    }

    /// The highest value this timeline's producer has undertaken to signal.
    ///
    /// The number the exit's second clause rests on. It is a *promise about the
    /// producer's own future*, which is the only honest thing a submitter can
    /// declare about values nobody has produced yet — it cannot be derived,
    /// inferred from the last signal, or extrapolated, and a consumer that
    /// guessed it would be guessing about whether its own wait terminates.
    /// Unit: none — a timeline value, which is an ordinal.
    #[must_use]
    pub const fn ceiling(&self) -> u64 {
        self.ceiling
    }

    /// Raise what this producer has undertaken to signal.
    ///
    /// Published before the signal it covers, never after, so that a consumer's
    /// wait is checked against a promise that already exists. Equal is accepted
    /// and is a no-op: re-stating a promise costs nothing and refusing it would
    /// make an idempotent republish an error.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotMonotonic`] for a ceiling below the current one. A falling
    /// ceiling would retroactively unadmit waits that were admitted against it
    /// — turning a checked wait into exactly the hang this module refuses, some
    /// time after the check that was supposed to have prevented it. This is the
    /// reason the field can only rise.
    pub const fn promise(&self, ceiling: u64) -> Result<Self, Refusal> {
        if ceiling < self.ceiling {
            return Err(Refusal::NotMonotonic);
        }
        Ok(Self { id: self.id, signalled: self.signalled, ceiling })
    }

    /// Record that a signal landed.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoValue`] for [`UNSIGNALLED`]. [`Refusal::NotMonotonic`] for
    /// a value at or below what has already landed. [`Refusal::Overcommitted`]
    /// for a value above the ceiling — a producer that signalled past its own
    /// promise, which is refused *here* as well as at
    /// [`Signal::undertaken`](Signal) because this is where it would be recorded
    /// as true.
    pub const fn landed(&self, value: u64) -> Result<Self, Refusal> {
        if value == UNSIGNALLED {
            return Err(Refusal::NoValue);
        }
        if value <= self.signalled {
            return Err(Refusal::NotMonotonic);
        }
        if value > self.ceiling {
            return Err(Refusal::Overcommitted);
        }
        Ok(Self { id: self.id, signalled: value, ceiling: self.ceiling })
    }

    /// A wait on this timeline, or the refusal that replaces a hang.
    ///
    /// The only door to [`Wait`] a caller has. What comes back is not a checked
    /// number but a value that could not have been built if the check had
    /// failed, which is the difference between this and a guard.
    ///
    /// # Errors
    ///
    /// [`Refusal::Unreachable`] for a value above [`Timeline::ceiling`], and
    /// [`Refusal::NoValue`] for [`UNSIGNALLED`].
    pub const fn wait(&self, value: u64) -> Result<Wait, Refusal> {
        Wait::admitted(self, value)
    }

    /// An undertaking to move this timeline to `value`.
    ///
    /// The only door to [`Signal`].
    ///
    /// # Errors
    ///
    /// [`Refusal::NoValue`], [`Refusal::NotMonotonic`] or
    /// [`Refusal::Overcommitted`], as [`Signal::undertaken`](Signal) states.
    pub const fn signal(&self, value: u64) -> Result<Signal, Refusal> {
        Signal::undertaken(self, value)
    }

    /// The record as it crosses: three little-endian fields, then zero.
    #[must_use]
    pub fn encode(&self) -> [u8; TIMELINE_BYTES] {
        let mut out = [0u8; TIMELINE_BYTES];
        out[0..8].copy_from_slice(&self.signalled.to_le_bytes());
        out[8..16].copy_from_slice(&self.ceiling.to_le_bytes());
        out[16..20].copy_from_slice(&self.id.to_le_bytes());
        out
    }

    /// Decode, refusing before any field is handed back.
    ///
    /// # Errors
    ///
    /// Whatever [`Timeline::adopt`] refuses, or [`Refusal::Reserved`] for any
    /// byte this build does not read — established by rebuilding the image this
    /// build would have written and comparing all [`TIMELINE_BYTES`] of it,
    /// rather than by a list of the fields somebody remembered were padding.
    pub fn decode(raw: &[u8; TIMELINE_BYTES]) -> Result<Self, Refusal> {
        let signalled = u64_at(raw, 0);
        let ceiling = u64_at(raw, 8);
        let id = u32_at(raw, 16);
        let timeline = Self::adopt(id, signalled, ceiling)?;
        if timeline.encode() != *raw {
            return Err(Refusal::Reserved);
        }
        Ok(timeline)
    }
}

/// What one stage of the chain enters on and what it leaves behind.
///
/// Up to [`MAX_WAITS`] waits and **at most one signal**, and the one is not a
/// budget: a submission is one stage of the chain and a timeline has one
/// producer, so the only timeline a submission may signal is its own. A record
/// that could signal two would be a record that could make one stage the
/// producer of somebody else's timeline, which is the premise the module's
/// first section says everything rests on.
///
/// Every wait in here is a [`Wait`], which is to say every wait in here was
/// admitted by the timeline it names. There is no constructor that takes a
/// number, so a submission carrying a wait nothing can satisfy is not a
/// submission this build refuses — it is a value that does not exist.
/// There is no count field beside the array, and that is a decision rather than
/// a saving of one byte. A count is a second statement of how much of the array
/// is real, and this file's whole argument is that a second statement of a
/// derivable fact is a thing two writers disagree about while both pass their
/// own tests. An empty slot is `None` — not a resting [`Wait`] naming nothing,
/// because a resting [`Wait`] would be a wait no timeline ever admitted, and the
/// point of the type is that no such value exists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Submission {
    /// The waits, in the order they were added, filling the array from the
    /// front. [`Submission::waiting`] is the only thing that writes one, and it
    /// appends at the first empty slot, so the used slots are a prefix.
    waits: [Option<Wait>; MAX_WAITS],
    /// The one signal, if this stage leaves one. `None` is the present engine,
    /// which waits and signals nothing.
    signal: Option<Signal>,
}

impl Submission {
    /// A submission that waits on nothing and signals nothing.
    ///
    /// The starting value of every builder below. It is legal on the wire: a
    /// stage with no dependencies and no dependents is a stage that is neither,
    /// and refusing it would be refusing a well-formed statement about a
    /// pipeline of one.
    pub const EMPTY: Self = Self { waits: [None; MAX_WAITS], signal: None };

    /// Add a wait.
    ///
    /// Takes a [`Wait`] and not a timeline and a number, which is where the
    /// exit's second clause is enforced: by the time a wait reaches this
    /// function it has already been admitted by [`Timeline::wait`], and there
    /// is no path that reaches here without having been.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] past [`MAX_WAITS`]. [`Refusal::Duplicate`] for a
    /// timeline this submission already waits on. [`Refusal::SelfBlocked`] for
    /// a wait on this submission's own signal at or above the value it signals.
    pub const fn waiting(self, wait: Wait) -> Result<Self, Refusal> {
        let count = self.wait_count();
        if count == MAX_WAITS {
            return Err(Refusal::Full);
        }
        let mut at = 0;
        while at < count {
            if let Some(already) = self.waits[at]
                && already.timeline() == wait.timeline()
            {
                return Err(Refusal::Duplicate);
            }
            at += 1;
        }
        if let Some(signal) = self.signal
            && self_blocked(&wait, &signal)
        {
            return Err(Refusal::SelfBlocked);
        }
        let mut waits = self.waits;
        waits[count] = Some(wait);
        Ok(Self { waits, signal: self.signal })
    }

    /// Set the one signal.
    ///
    /// # Errors
    ///
    /// [`Refusal::Duplicate`] if this submission already signals — a stage
    /// signals its own timeline once, and a second signal is a second producer
    /// or a second value. [`Refusal::SelfBlocked`] if a wait already in this
    /// submission names this signal's timeline at or above its value.
    pub const fn signalling(self, signal: Signal) -> Result<Self, Refusal> {
        if self.signal.is_some() {
            return Err(Refusal::Duplicate);
        }
        let count = self.wait_count();
        let mut at = 0;
        while at < count {
            if let Some(entered) = self.waits[at]
                && self_blocked(&entered, &signal)
            {
                return Err(Refusal::SelfBlocked);
            }
            at += 1;
        }
        Ok(Self { waits: self.waits, signal: Some(signal) })
    }

    /// How many waits this submission carries.
    ///
    /// Counted from the array rather than stored beside it, so it cannot
    /// disagree with what it counts.
    /// Unit: waits.
    #[must_use]
    pub const fn wait_count(&self) -> usize {
        let mut at = 0;
        while at < MAX_WAITS {
            if self.waits[at].is_none() {
                return at;
            }
            at += 1;
        }
        MAX_WAITS
    }

    /// The wait at `index`, or `None` for an index this submission does not
    /// carry.
    #[must_use]
    pub const fn wait(&self, index: usize) -> Option<Wait> {
        if index >= MAX_WAITS {
            return None;
        }
        self.waits[index]
    }

    /// The signal this stage leaves, or `None` for a stage that leaves none.
    #[must_use]
    pub const fn signal(&self) -> Option<Signal> {
        self.signal
    }

    /// The record as it crosses.
    ///
    /// Little-endian and by hand, so nothing here depends on the host's word
    /// size, alignment rules or byte order. Every slot the writer does not reach
    /// stays zero, which is what makes two producers of the same submission
    /// produce the same bytes and what gives [`Submission::decode`]'s
    /// whole-image comparison something to compare against.
    #[must_use]
    pub fn encode(&self) -> [u8; SUBMISSION_BYTES] {
        let mut out = [0u8; SUBMISSION_BYTES];
        out[0] = self.wait_count() as u8;
        out[1] = u8::from(self.signal.is_some());
        for (index, slot) in self.waits.iter().enumerate() {
            if let Some(wait) = slot {
                put_slot(&mut out, WAITS_AT + index * SLOT_BYTES, wait.timeline(), wait.value());
            }
        }
        if let Some(signal) = self.signal {
            put_slot(&mut out, SIGNAL_AT, signal.timeline(), signal.value());
        }
        out
    }

    /// Decode against the timelines this side holds.
    ///
    /// `known` is a slice and is scanned linearly, which is the right shape at
    /// this size — the chain has two timelines and a compositor's set is the
    /// clients it is compositing — and is also the only shape available: a hash
    /// map is forbidden in this workspace because its iteration order is seeded
    /// per process, and a lookup whose answer depended on that would make a
    /// refusal depend on it too. RFC 0004.
    ///
    /// Every wait and the signal go back through [`Timeline::wait`] and
    /// [`Timeline::signal`], so a peer's bytes are admitted by exactly the
    /// function a local caller is. There is no decode-side copy of the rules to
    /// fall out of step with the encode-side ones, which is the failure
    /// `docs/postmortem/0001` records for a grammar split between a reader and
    /// a composer.
    ///
    /// # What comes back is not a builder
    ///
    /// [`Arrived`] and not `Self`, and that is `E3-B05c`. A driver is a
    /// receiver; if what a receiver held were a [`Submission`], it would hold
    /// [`Submission::waiting`] too, and a driver inserting a wait of its own
    /// into the client's record would be an ordinary use of a public function
    /// rather than something the type refuses. The admission is unchanged —
    /// every wait still goes through [`Timeline::wait`], the same door a local
    /// caller uses — and what changed is that the door closes behind it. See
    /// [`Arrived`] for what that does and does not buy. RFC 0111.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] for a count the record cannot frame;
    /// [`Refusal::NoSuchTimeline`] for a timeline this side does not hold;
    /// whatever the admission of each wait and signal refuses — including
    /// [`Refusal::Unreachable`]; and [`Refusal::Reserved`] for any byte this
    /// build does not read, which is established by rebuilding the image this
    /// build would have written and comparing all [`SUBMISSION_BYTES`] of it.
    pub fn decode(raw: &[u8; SUBMISSION_BYTES], known: &[Timeline]) -> Result<Arrived, Refusal> {
        let waits = raw[0] as usize;
        let signals = raw[1] as usize;
        if waits > MAX_WAITS || signals > 1 {
            return Err(Refusal::Full);
        }

        let mut submission = Self::EMPTY;
        for index in 0..waits {
            let (id, value) = slot_at(raw, WAITS_AT + index * SLOT_BYTES);
            let timeline = held(known, id)?;
            submission = submission.waiting(timeline.wait(value)?)?;
        }
        if signals == 1 {
            let (id, value) = slot_at(raw, SIGNAL_AT);
            let timeline = held(known, id)?;
            submission = submission.signalling(timeline.signal(value)?)?;
        }

        // The catch-all, and the reason none of the zero regions above is
        // checked by hand: an unused wait slot, the six header bytes this build
        // does not read, and the tail are all bytes the rebuilt image has zero,
        // so anything a peer wrote in them fails this comparison. A list of the
        // regions would be correct on the day it was written and silent
        // afterwards; this is correct on the day a field is added.
        if submission.encode() != *raw {
            return Err(Refusal::Reserved);
        }
        Ok(Arrived::sealed(submission))
    }
}

/// The three-stage chain section 08 names, with its two timelines.
///
/// The application signals N, the compositor waits N and signals M, the present
/// engine waits M. Holding it as one value is what makes the ordering a
/// property of the type rather than of three components remembering the same
/// sentence — and it is what lets [`Chain::present_waits`] refuse a present
/// engine that is running ahead of a compositor that never promised the value
/// it is waiting for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Chain {
    /// The application's timeline: it alone signals it.
    application: Timeline,
    /// The compositor's timeline: it alone signals it.
    compositor: Timeline,
}

impl Chain {
    /// The chain over two timelines.
    ///
    /// # Errors
    ///
    /// [`Refusal::Duplicate`] if both stages name one timeline. Two producers
    /// on one timeline is the premise this module refuses, and the chain is
    /// where the two would meet.
    pub const fn new(application: Timeline, compositor: Timeline) -> Result<Self, Refusal> {
        if application.id() == compositor.id() {
            return Err(Refusal::Duplicate);
        }
        Ok(Self { application, compositor })
    }

    /// The application's timeline, as the chain currently holds it.
    #[must_use]
    pub const fn application(&self) -> Timeline {
        self.application
    }

    /// The compositor's timeline, as the chain currently holds it.
    #[must_use]
    pub const fn compositor(&self) -> Timeline {
        self.compositor
    }

    /// Both timelines, for [`Submission::decode`] to check an arriving record
    /// against.
    /// Unit: none — the chain's timelines, application first.
    #[must_use]
    pub const fn timelines(&self) -> [Timeline; 2] {
        [self.application, self.compositor]
    }

    /// The application promises `n` and submits the signal of it.
    ///
    /// The promise is raised *before* the submission exists, which is what makes
    /// the compositor's wait on `n` admissible at all. A design that signalled
    /// without promising would leave every consumer's wait refused until after
    /// the signal had already landed, which is a wait nobody would ever need.
    ///
    /// # Errors
    ///
    /// [`Refusal::NotMonotonic`] for an `n` that does not move the application's
    /// timeline forward, and [`Refusal::NoValue`] for [`UNSIGNALLED`].
    pub fn application_signals(&mut self, n: u64) -> Result<Submission, Refusal> {
        let promised = self.application.promise(n)?;
        let submission = Submission::EMPTY.signalling(promised.signal(n)?)?;
        self.application = promised;
        Ok(submission)
    }

    /// The compositor waits `n` on the application's timeline and promises `m`
    /// on its own, and the wait it enters is recorded in `trace`.
    ///
    /// The trace is an argument and not a field of the chain, which is the
    /// decision worth stating: a chain that held its own trace would decide for
    /// every caller how long a frame is, and *one frame* is the compositor's
    /// unit rather than this type's. What the argument buys instead is that
    /// there is no path through this function that enters a wait and does not
    /// record it — `E3-B05b`'s *names every wait*, held at the door rather
    /// than asked of a caller. `crate::trace` states exactly how far that
    /// reaches and where it stops.
    ///
    /// # Errors
    ///
    /// [`Refusal::Unreachable`] if `n` is above what the application has
    /// undertaken to signal — the case the whole module exists for, and the one
    /// that would otherwise be a compositor blocked forever on a frame an
    /// application never promised. Then whatever the compositor's own promise
    /// and signal refuse. A refusal records nothing, because a wait that was
    /// refused was never entered.
    pub fn compositor_waits_and_signals(
        &mut self,
        n: u64,
        m: u64,
        trace: &mut Trace,
    ) -> Result<Submission, Refusal> {
        let wait = self.application.wait(n)?;
        let promised = self.compositor.promise(m)?;
        let submission = Submission::EMPTY.waiting(wait)?.signalling(promised.signal(m)?)?;
        self.compositor = promised;
        trace.entering(Stage::Compositor, &submission, &self.timelines());
        Ok(submission)
    }

    /// The present engine waits `m` on the compositor's timeline, and the wait
    /// is recorded in `trace`.
    ///
    /// Takes `&self` and not `&mut self`, and that is the chain's third stage
    /// saying what it is: a stage that signals nothing changes no timeline, so
    /// there is no state for it to move. It is also why the chain holds two
    /// timelines rather than three. The trace is `&mut` and the chain is not,
    /// which is the argument surviving the recording rather than being traded
    /// for it: what changes is the record of the frame, not the pipeline. Had
    /// the trace been a field of `Chain` this function would have had to become
    /// `&mut self` and the sentence above would have been withdrawn for a
    /// reason that has nothing to do with what a present engine is.
    ///
    /// # Errors
    ///
    /// [`Refusal::Unreachable`] if `m` is above what the compositor has
    /// undertaken to signal. A refusal records nothing.
    pub fn present_waits(&self, m: u64, trace: &mut Trace) -> Result<Submission, Refusal> {
        let submission = Submission::EMPTY.waiting(self.compositor.wait(m)?)?;
        trace.entering(Stage::Present, &submission, &self.timelines());
        Ok(submission)
    }

    /// Record that a signal landed on one of the chain's timelines, closing in
    /// `trace` every wait that landing satisfied.
    ///
    /// This is where the trace's *who signalled it* is answered, and it is
    /// answered at the landing rather than at the wait because that is where the
    /// fact becomes true: a submitted signal is not a signal that happened
    /// ([`Timeline::signalled`] says so), and a record that named a signaller
    /// before the signal landed would say a frame was composited while it was
    /// still queued.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoSuchTimeline`] for an identifier that is neither of the
    /// chain's, and whatever [`Timeline::landed`] refuses. A refusal closes
    /// nothing in the trace: a landing that did not happen released no wait.
    pub fn landed(&mut self, timeline: u32, value: u64, trace: &mut Trace) -> Result<(), Refusal> {
        if timeline == self.application.id() {
            self.application = self.application.landed(value)?;
        } else if timeline == self.compositor.id() {
            self.compositor = self.compositor.landed(value)?;
        } else {
            return Err(Refusal::NoSuchTimeline);
        }
        trace.released(timeline, value);
        Ok(())
    }
}

/// Would this wait block on a signal only this submission can produce?
///
/// One function, called by both builders, so that the order a caller happens to
/// add a wait and a signal in cannot change whether the pair is refused.
const fn self_blocked(wait: &Wait, signal: &Signal) -> bool {
    wait.timeline() == signal.timeline() && wait.value() >= signal.value()
}

/// The timeline `id` names, out of the set this side holds.
///
/// # Errors
///
/// [`Refusal::NoSuchTimeline`]. A wait against a timeline whose ceiling is
/// unknown is refused for the same reason a wait above a known ceiling is: in
/// neither case can this side say anything will ever arrive.
pub(crate) fn held(known: &[Timeline], id: u32) -> Result<Timeline, Refusal> {
    known.iter().copied().find(|timeline| timeline.id() == id).ok_or(Refusal::NoSuchTimeline)
}

/// Write one wait or signal slot: the value, the timeline, then zero.
///
/// The value first so that it is eight-byte aligned in every slot of a record
/// whose own start is aligned. Nothing here casts a slot to a struct — the
/// alignment buys a reader's arithmetic rather than a compiler's, which is the
/// only kind this crate is allowed to want.
fn put_slot(out: &mut [u8; SUBMISSION_BYTES], at: usize, timeline: u32, value: u64) {
    out[at..at + 8].copy_from_slice(&value.to_le_bytes());
    out[at + 8..at + 12].copy_from_slice(&timeline.to_le_bytes());
}

/// Read one slot: the timeline it names and the value it carries.
///
/// The four bytes past the fields are not read here and are not checked here
/// either; they are covered by the whole-image comparison in
/// [`Submission::decode`], which is the one place *this build did not read that
/// byte* is decided for the entire record.
fn slot_at(raw: &[u8; SUBMISSION_BYTES], at: usize) -> (u32, u64) {
    (u32_at(raw, at + 8), u64_at(raw, at))
}

/// Eight little-endian bytes.
pub(crate) fn u64_at(raw: &[u8], at: usize) -> u64 {
    let mut word = [0u8; 8];
    word.copy_from_slice(&raw[at..at + 8]);
    u64::from_le_bytes(word)
}

/// Four little-endian bytes.
pub(crate) fn u32_at(raw: &[u8], at: usize) -> u32 {
    let mut word = [0u8; 4];
    word.copy_from_slice(&raw[at..at + 4]);
    u32::from_le_bytes(word)
}

// The widths, asserted beside the records rather than in a table somewhere
// else. A field added to either that would not fit stops the build here.
const _: () = assert!(Timeline::FIELD_BYTES <= TIMELINE_BYTES);
const _: () = assert!(TIMELINE_BYTES == Timeline::FIELD_BYTES.next_multiple_of(8));
// A slot holds a `u64` and a `u32` with room to the stride. Written as the sum
// so the arithmetic is in front of whoever changes a field.
const _: () = assert!(8 + 4 <= SLOT_BYTES);
// And the submission's regions tile the record without overlapping it.
const _: () = assert!(WAITS_AT + MAX_WAITS * SLOT_BYTES == SIGNAL_AT);
const _: () = assert!(SIGNAL_AT + SLOT_BYTES == TAIL_AT);
const _: () = assert!(TAIL_AT <= SUBMISSION_BYTES);
// The counts fit the bytes they are written into. A capacity past 255 would
// make `wait_count` lossy on the wire, silently, which is the kind of narrowing
// that passes every test until the day somebody raises `SUBMISSION_BYTES`.
const _: () = assert!(MAX_WAITS <= u8::MAX as usize);

#[cfg(test)]
mod tests {
    use super::*;

    /// The application's timeline, and the compositor's.
    const APP: u32 = 0x0000_0011;
    const COMP: u32 = 0x0000_0022;

    /// A trace nothing in this file reads.
    ///
    /// Every test below is about the record — its bytes, its refusals, its
    /// admissions — and the trace is threaded through the chain's doors for a
    /// different exit. `crate::trace`'s own tests are where what lands in it is
    /// asserted, and a copy of those assertions here would be a second place one
    /// property is checked and a second place it can be weakened.
    fn untraced() -> Trace {
        Trace::EMPTY
    }

    /// A chain whose application has promised `n` and whose compositor has
    /// promised `m`.
    fn chain(n: u64, m: u64) -> Chain {
        let application = Timeline::declare(APP).unwrap().promise(n).unwrap();
        let compositor = Timeline::declare(COMP).unwrap().promise(m).unwrap();
        Chain::new(application, compositor).unwrap()
    }

    #[test]
    fn the_bytes_are_the_ones_written_down() {
        // The exit's first clause, as an assertion about layout rather than a
        // `size_of`: every byte of both records, in the position it occupies.
        // The widths themselves are in the signatures — `encode` returns an
        // array of a named length — so a record that changed width would not
        // reach this test; it would fail to compile at every call site. What
        // this adds is the *positions*, which is what a `to_ne_bytes` that
        // slipped into a writer would move on one of the two architectures.
        let timeline = Timeline::adopt(APP, 7, 9).unwrap();
        #[rustfmt::skip]
        let expected: [u8; TIMELINE_BYTES] = [
            // signalled = 7.
            0x07, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // ceiling = 9.
            0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // id = 0x11.
            0x11, 0x00, 0x00, 0x00,
            // The padding to the stride, which decoding requires to be zero.
            0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(timeline.encode(), expected);

        let mut pipeline = chain(9, 0x0102);
        let composited = pipeline.compositor_waits_and_signals(9, 0x0102, &mut untraced()).unwrap();
        #[rustfmt::skip]
        let expected: [u8; SUBMISSION_BYTES] = [
            // One wait, one signal, then six bytes this build does not read.
            0x01, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // Wait slot zero: value 9 on timeline 0x11, then four zero.
            0x09, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x11, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // Wait slots one to five, unused and therefore zero: ten rows,
            // which is five slots of sixteen bytes and is the arithmetic
            // `MAX_WAITS` is derived from.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // The signal slot: value 0x0102 on timeline 0x22, then four zero.
            0x02, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x22, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            // The tail, which decoding requires to be zero.
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(composited.encode(), expected);
    }

    #[test]
    fn every_record_round_trips_through_its_bytes() {
        // The rest of the exit's first clause. It runs on x86-64 and, through
        // `cargo xtask test` on the arm runner, on AArch64 — which is what the
        // encoding being written by hand is for: every field goes out through
        // `to_le_bytes` and comes back through `from_le_bytes`, so nothing here
        // can inherit the host's word size, its alignment or its byte order.
        let mut pipeline = chain(9, 0x0102);
        for timeline in pipeline.timelines() {
            assert_eq!(Timeline::decode(&timeline.encode()), Ok(timeline));
        }

        let known = pipeline.timelines();
        let mut ahead = pipeline;
        for submission in [
            Submission::EMPTY,
            ahead.application_signals(9).unwrap(),
            pipeline.compositor_waits_and_signals(9, 0x0102, &mut untraced()).unwrap(),
            pipeline.present_waits(0x0102, &mut untraced()).unwrap(),
        ] {
            let raw = submission.encode();
            // `Arrived` and not a `Submission`, compared through the one
            // asymmetric `PartialEq` there is: *the bytes decoded to the
            // submission I built*, said without a conversion in either
            // direction. RFC 0111.
            assert_eq!(Submission::decode(&raw, &known).unwrap(), submission);
            // And the encoding is a function of the value alone, which is what
            // makes two architectures produce one wire image.
            assert_eq!(Submission::decode(&raw, &known).unwrap().encode(), raw);
        }
    }

    #[test]
    fn a_wait_nothing_can_ever_signal_is_refused_at_submission() {
        // The exit's second clause, and the reason the task exists. The
        // application has undertaken to signal 9. A compositor asking to wait
        // on 10 is asking to block on a value no producer will ever write, and
        // there is no later moment at which that becomes visible: the process
        // is simply still there.
        let mut pipeline = chain(9, 0x0102);
        assert_eq!(pipeline.application().wait(10), Err(Refusal::Unreachable));
        assert_eq!(
            pipeline.compositor_waits_and_signals(10, 0x0102, &mut untraced()),
            Err(Refusal::Unreachable)
        );
        // Refused *at submission*: the chain did not move, so a caller that
        // ignored the refusal has submitted nothing rather than submitted a
        // hang.
        assert_eq!(pipeline, chain(9, 0x0102));
        // And on the value itself: 9 is admissible, 10 is not, and the boundary
        // is the promise rather than anything derived from it.
        assert!(pipeline.application().wait(9).is_ok());
        assert_eq!(pipeline.present_waits(0x0103, &mut untraced()), Err(Refusal::Unreachable));
        assert!(pipeline.present_waits(0x0102, &mut untraced()).is_ok());

        // The same refusal on the way in from a peer, reached through the same
        // constructor rather than through a second copy of the rule: take a
        // legal submission's bytes and raise the value one past the ceiling.
        let legal = pipeline.compositor_waits_and_signals(9, 0x0102, &mut untraced()).unwrap();
        let mut raw = legal.encode();
        raw[WAITS_AT] = 10;
        assert_eq!(
            Submission::decode(&raw, &chain(9, 0x0102).timelines()),
            Err(Refusal::Unreachable)
        );
    }

    #[test]
    fn a_producer_that_has_promised_nothing_admits_no_wait() {
        // The base case, and it falls out rather than being arranged: a fresh
        // timeline's ceiling is `UNSIGNALLED`, and every value is above it. A
        // consumer cannot block on a producer that has not said what it will
        // produce, which is the correct answer and not a degenerate one.
        let fresh = Timeline::declare(APP).unwrap();
        assert_eq!(fresh.ceiling(), UNSIGNALLED);
        assert_eq!(fresh.wait(1), Err(Refusal::Unreachable));
        assert_eq!(fresh.wait(u64::MAX), Err(Refusal::Unreachable));
        // And zero is not a wait at all: it is where the timeline already is.
        assert_eq!(fresh.wait(UNSIGNALLED), Err(Refusal::NoValue));
        // A zeroed record names no timeline, so it is not timeline zero waiting
        // on nothing.
        assert_eq!(Timeline::decode(&[0; TIMELINE_BYTES]), Err(Refusal::NoTimeline));
        assert_eq!(Timeline::declare(NO_TIMELINE), Err(Refusal::NoTimeline));
    }

    #[test]
    fn a_wait_at_or_below_what_has_landed_is_not_a_hang_and_is_not_refused() {
        // The other half of the boundary, stated because a module that refused
        // this would have replaced a hang with a broken producer. A stage that
        // is behind submits a wait that is already satisfied, and that is the
        // ordinary case rather than an error.
        let timeline = Timeline::adopt(APP, 7, 9).unwrap();
        let entered = timeline.wait(3).unwrap();
        assert!(entered.satisfied_by(&timeline));
        assert!(!timeline.wait(8).unwrap().satisfied_by(&timeline));
        // And a wait is never satisfied by a timeline that is not its own, at
        // any value: the question is meaningless rather than affirmative.
        let other = Timeline::adopt(COMP, u64::MAX, u64::MAX).unwrap();
        assert!(!entered.satisfied_by(&other));
    }

    #[test]
    fn a_promise_never_falls_so_an_admitted_wait_stays_admitted() {
        // What makes the admission worth having. A ceiling that could fall
        // would turn a wait that passed the check into a hang some time after
        // the check that was supposed to prevent it — the same failure, moved
        // where nobody is looking for it.
        let promised = Timeline::declare(APP).unwrap().promise(9).unwrap();
        assert_eq!(promised.promise(8), Err(Refusal::NotMonotonic));
        assert_eq!(promised.promise(9), Ok(promised));
        assert_eq!(promised.promise(10).unwrap().ceiling(), 10);
        // A timeline that had signalled past its promise could not be adopted
        // at all, which is the same rule where a record arrives from outside.
        assert_eq!(Timeline::adopt(APP, 10, 9), Err(Refusal::NotMonotonic));
        let mut raw = Timeline::adopt(APP, 9, 9).unwrap().encode();
        raw[0] = 10;
        assert_eq!(Timeline::decode(&raw), Err(Refusal::NotMonotonic));
    }

    #[test]
    fn a_signal_moves_the_timeline_forward_or_it_is_not_a_signal() {
        let timeline = Timeline::adopt(APP, 7, 9).unwrap();
        assert_eq!(timeline.signal(UNSIGNALLED), Err(Refusal::NoValue));
        assert_eq!(timeline.signal(7), Err(Refusal::NotMonotonic));
        assert_eq!(timeline.signal(10), Err(Refusal::Overcommitted));
        assert_eq!(timeline.signal(8).unwrap().value(), 8);
        // Submitting a signal is not the signal having happened: the timeline
        // moves when it lands, and a field that moved at submission would say a
        // frame was composited while it was still queued.
        assert_eq!(timeline.signal(8).unwrap().timeline(), APP);
        assert_eq!(timeline.signalled(), 7);
        assert_eq!(timeline.landed(8).unwrap().signalled(), 8);
        assert_eq!(timeline.landed(7), Err(Refusal::NotMonotonic));
        assert_eq!(timeline.landed(10), Err(Refusal::Overcommitted));
    }

    #[test]
    fn a_submission_may_not_wait_on_the_signal_it_has_not_produced_yet() {
        // This module's own failure wearing a different shape: a submission
        // whose wait can only be satisfied by its own signal is a hang, and it
        // leaves exactly as little evidence. Refused in both orders, because
        // the order a caller adds the two in is not a fact about the record.
        let timeline = Timeline::adopt(COMP, 4, 9).unwrap();
        let signal = timeline.signal(8).unwrap();
        let blocking = timeline.wait(8).unwrap();
        assert_eq!(
            Submission::EMPTY.waiting(blocking).unwrap().signalling(signal),
            Err(Refusal::SelfBlocked)
        );
        assert_eq!(
            Submission::EMPTY.signalling(signal).unwrap().waiting(blocking),
            Err(Refusal::SelfBlocked)
        );
        // Waiting on the *previous* value of one's own timeline is the ordinary
        // double-buffered case and is admitted: it is below the signal, so
        // something other than this submission produced it.
        let ordinary = Submission::EMPTY.waiting(timeline.wait(4).unwrap()).unwrap();
        assert!(ordinary.signalling(signal).is_ok());
    }

    #[test]
    fn one_timeline_is_named_once_and_the_record_frames_what_it_says() {
        let application = Timeline::adopt(APP, 0, 9).unwrap();
        let compositor = Timeline::adopt(COMP, 0, 9).unwrap();
        let twice = Submission::EMPTY.waiting(application.wait(3).unwrap()).unwrap();
        assert_eq!(twice.waiting(application.wait(4).unwrap()), Err(Refusal::Duplicate));
        let once = Submission::EMPTY.signalling(compositor.signal(1).unwrap()).unwrap();
        assert_eq!(once.signalling(compositor.signal(2).unwrap()), Err(Refusal::Duplicate));
        assert_eq!(Chain::new(application, application), Err(Refusal::Duplicate));

        // The capacity is the record's, and it is the record that refuses past
        // it rather than an array silently truncating.
        let mut full = Submission::EMPTY;
        for id in 1..=MAX_WAITS {
            let timeline = Timeline::adopt(id as u32, 0, 9).unwrap();
            full = full.waiting(timeline.wait(1).unwrap()).unwrap();
        }
        assert_eq!(full.wait_count(), MAX_WAITS);
        assert_eq!(full.wait(MAX_WAITS), None);
        let extra = Timeline::adopt(MAX_WAITS as u32 + 1, 0, 9).unwrap();
        assert_eq!(full.waiting(extra.wait(1).unwrap()), Err(Refusal::Full));

        // And a count a record cannot frame is refused rather than clamped.
        let mut raw = full.encode();
        raw[0] = MAX_WAITS as u8 + 1;
        assert_eq!(Submission::decode(&raw, &[]), Err(Refusal::Full));
        let mut raw = full.encode();
        raw[1] = 2;
        assert_eq!(Submission::decode(&raw, &[]), Err(Refusal::Full));
    }

    #[test]
    fn what_arrived_forwards_the_bytes_it_arrived_as() {
        // `E3-B05c`'s positive half, and the reason `Arrived::encode` exists at
        // all: a receiver that must pass the record on — a supervisor
        // persisting it, `E3-B05f`'s subtree — does it without the bytes going
        // back through a builder, so *unchanged* is checkable rather than
        // asserted. The two things a driver cannot do are asserted by the
        // `compile_fail` fixtures on `Arrived`, because a test that passes
        // cannot tell unrepresentable from not-currently-done.
        let mut pipeline = chain(9, 0x0102);
        let built = pipeline.compositor_waits_and_signals(9, 0x0102, &mut untraced()).unwrap();
        let raw = built.encode();
        let arrived = Submission::decode(&raw, &pipeline.timelines()).unwrap();
        assert_eq!(arrived, built);
        assert_eq!(arrived.encode(), raw, "what arrived re-encodes to what arrived");
        assert_eq!(arrived.wait_count(), 1);
        assert_eq!(arrived.wait(0), built.wait(0));
        assert_eq!(arrived.signal(), built.signal());
        assert_eq!(arrived.wait(MAX_WAITS), None);
    }

    #[test]
    fn a_timeline_this_side_does_not_hold_is_refused_rather_than_assumed() {
        // An unknown ceiling and an exceeded one are the same ignorance. A
        // decoder that admitted a wait on a timeline it had never heard of
        // would be admitting a wait it cannot say anything about, which is the
        // exit's clause with the check removed.
        let mut pipeline = chain(9, 0x0102);
        let submission = pipeline.compositor_waits_and_signals(9, 0x0102, &mut untraced()).unwrap();
        let raw = submission.encode();
        assert_eq!(Submission::decode(&raw, &[]), Err(Refusal::NoSuchTimeline));
        assert_eq!(
            Submission::decode(&raw, &[pipeline.compositor()]),
            Err(Refusal::NoSuchTimeline)
        );
        assert_eq!(Submission::decode(&raw, &pipeline.timelines()).unwrap(), submission);
        assert_eq!(pipeline.landed(0x33, 1, &mut untraced()), Err(Refusal::NoSuchTimeline));
    }

    #[test]
    fn nothing_in_a_submission_is_ignored() {
        // Stated as strongly as it can be: there is no byte of a submission
        // that can be changed without either changing what the submission means
        // or being refused. A field this format read and then dropped would
        // show up here as a byte that can be flipped and still decode to the
        // value it started as.
        // The ceilings are `u64::MAX` on purpose: with a low ceiling most flips
        // of a value would be refused as unreachable, which would make this
        // test pass for a reason other than the one it is written for.
        let application = Timeline::adopt(APP, 0, u64::MAX).unwrap();
        let compositor = Timeline::adopt(COMP, 0, u64::MAX).unwrap();
        let known = [application, compositor];
        let waiting = Submission::EMPTY.waiting(application.wait(9).unwrap()).unwrap();
        let submission = waiting.signalling(compositor.signal(0x0102).unwrap()).unwrap();
        let raw = submission.encode();
        for at in 0..SUBMISSION_BYTES {
            let mut damaged = raw;
            damaged[at] ^= 0xFF;
            assert!(
                !Submission::decode(&damaged, &known).is_ok_and(|arrived| arrived == submission),
                "byte {at} of the submission is ignored"
            );
        }

        // The same for a timeline record.
        let timeline = Timeline::adopt(APP, 7, 9).unwrap();
        let raw = timeline.encode();
        for at in 0..TIMELINE_BYTES {
            let mut damaged = raw;
            damaged[at] ^= 0xFF;
            assert_ne!(
                Timeline::decode(&damaged),
                Ok(timeline),
                "byte {at} of the timeline record is ignored"
            );
        }
    }

    #[test]
    fn a_byte_this_build_does_not_read_is_refused_and_not_dropped() {
        // The clause where it is easiest to get wrong. The header's spare
        // bytes, an unused wait slot and the tail are exactly where a future
        // field goes, and a decoder that skipped them would accept a record
        // written by a peer that has that field — two peers with different
        // beliefs about what just happened, and no way for either to find out.
        let mut pipeline = chain(9, 0x0102);
        let known = pipeline.timelines();
        let submission = pipeline.compositor_waits_and_signals(9, 0x0102, &mut untraced()).unwrap();
        let raw = submission.encode();
        let unused_slot = WAITS_AT + SLOT_BYTES;
        for at in (2..HEADER_BYTES)
            .chain(unused_slot..SIGNAL_AT)
            .chain(TAIL_AT..SUBMISSION_BYTES)
            .chain(WAITS_AT + 12..WAITS_AT + SLOT_BYTES)
        {
            let mut damaged = raw;
            damaged[at] = 1;
            assert_eq!(
                Submission::decode(&damaged, &known),
                Err(Refusal::Reserved),
                "byte {at} is not refused"
            );
        }

        for at in Timeline::FIELD_BYTES..TIMELINE_BYTES {
            let mut damaged = Timeline::adopt(APP, 7, 9).unwrap().encode();
            damaged[at] = 1;
            assert_eq!(Timeline::decode(&damaged), Err(Refusal::Reserved), "byte {at}");
        }
    }

    #[test]
    fn the_chain_the_design_names() {
        // Section 08's sentence, executed. The application signals N, the
        // compositor waits N and signals M, the present engine waits M — and
        // the present engine holds no timeline of its own, which is why the
        // last of the three takes `&self`.
        let application = Timeline::declare(APP).unwrap();
        let compositor = Timeline::declare(COMP).unwrap();
        let mut pipeline = Chain::new(application, compositor).unwrap();

        // Before the application has promised anything, the compositor cannot
        // wait on it. This is the ordering the type enforces.
        assert_eq!(
            pipeline.compositor_waits_and_signals(1, 1, &mut untraced()),
            Err(Refusal::Unreachable)
        );

        let signals_n = pipeline.application_signals(1).unwrap();
        assert_eq!(signals_n.wait_count(), 0);
        assert_eq!(signals_n.signal().unwrap().timeline(), APP);
        assert_eq!(signals_n.signal().unwrap().value(), 1);

        let composites = pipeline.compositor_waits_and_signals(1, 1, &mut untraced()).unwrap();
        assert_eq!(composites.wait(0).unwrap().timeline(), APP);
        assert_eq!(composites.wait(0).unwrap().value(), 1);
        assert_eq!(composites.signal().unwrap().timeline(), COMP);

        let presents = pipeline.present_waits(1, &mut untraced()).unwrap();
        assert_eq!(presents.wait(0).unwrap().timeline(), COMP);
        assert_eq!(presents.signal(), None);

        // The present engine cannot run ahead of a promise either, and the next
        // frame's wait is admitted only once the next frame is promised.
        assert_eq!(pipeline.present_waits(2, &mut untraced()), Err(Refusal::Unreachable));
        pipeline.landed(APP, 1, &mut untraced()).unwrap();
        pipeline.application_signals(2).unwrap();
        assert!(pipeline.application().wait(2).is_ok());
    }

    #[test]
    fn every_refusal_says_something_and_is_an_argument_error() {
        // RFC 0010's shape: a negative result in a domain a caller acts on,
        // with a code that already means what it says. The corpus is
        // `Refusal::ALL`, emitted from the same list as the variants, so an
        // eleventh refusal is covered the day it is declared; and the messages are
        // non-empty by a compile-time assertion in the macro rather than by
        // this loop, which is here to show that nothing has been arranged to
        // pass it.
        assert_eq!(Refusal::ALL.len(), 11);
        for refusal in Refusal::ALL {
            let packed = refusal.packed();
            assert!(packed < 0, "{}", refusal.message());
            let (domain, _) = error::unpack(packed).expect("a refusal is an error");
            assert_eq!(domain, error::ARGUMENT, "{}", refusal.message());
            assert!(!refusal.message().is_empty());
        }
    }
}
