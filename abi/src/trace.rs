// SPDX-License-Identifier: Apache-2.0 OR MIT
//! One frame's waits, recorded so that the failure with no log line leaves one.
//!
//! # What this is answerable to
//!
//! [`crate::sync`]'s header names three outcomes of a wait and says of the
//! third: *a hang writes no log line*. That module refuses the hang it can see
//! coming — a wait above anything any producer has undertaken to signal — and
//! says plainly what it cannot catch: a producer that promised a value and then
//! died before reaching it, and a cycle across two submissions. Both of those
//! are hangs on values that were admissible when they were admitted, and both
//! leave the same nothing behind.
//!
//! This is that nothing, replaced by a record. A [`Trace`] entry whose
//! [`Waited::released_at`] is [`UNSIGNALLED`] is a wait this frame entered and
//! did not get out of, named, with its value and with the timeline whose
//! producer owed it. It does not prevent the hang; it is what makes the hang
//! readable afterwards, which is the whole of what `E3-B05`'s negative — *no
//! implicit wait appears in a frame trace* — can rest on.
//!
//! # The three clauses, as decisions rather than fields
//!
//! **Every wait, not a sample.** [`Trace::entering`] records every wait a
//! submission carries. At the bound it does not skip: it counts, in
//! [`Trace::dropped_waits`], and [`Trace::complete`] goes false. A trace that
//! could silently drop one would make *names every wait* a sentence with no
//! observation behind it — the reader would hold a complete-looking record of an
//! incomplete frame, which is worse than holding nothing precisely because it is
//! usable.
//!
//! **Its value and who signalled it.** The value is [`Waited::value`]. *Who* is
//! [`Waited::timeline`], and that is a decision worth the paragraph it costs.
//! `sync`'s first premise is that **a timeline has exactly one producer** — the
//! module refuses two, and says a stage whose signal could come from either of
//! two components needs a second timeline rather than a second writer. So the
//! timeline identifier *is* the producer's identity, and it is an identity that
//! means something across a ring for the same reason: it is the number both
//! peers put in a [`Submission`] and the number a receiver resolves against the
//! timelines it holds. A separate *who* field would be a second statement of a
//! fact the record already carries, which is what [`Submission::wait_count`]
//! declines to store for exactly this reason. It is not a pointer — nothing in
//! this crate holds one — and it is not a string, because a name a peer chooses
//! is a name a peer can choose twice.
//!
//! What the identifier alone cannot say is whether anybody actually signalled,
//! so [`Waited::released_at`] carries the value that timeline had reached when
//! the wait ended. Identity and evidence together: *timeline 0x11 owed 9, and
//! timeline 0x11 had got to 9.*
//!
//! **Byte-identical for one seed.** There is no clock in this record and no
//! duration. That is not an omission, it is the clause: a field that moved run to
//! run would make the byte comparison fail for a reason that is not a defect, and
//! then the comparison would be relaxed, and then it would be worth nothing.
//! Frame *time* is `E3-B05d`, which is a measurement on a named machine. *What
//! would reverse this:* `E3-B05d` needing a per-wait duration, at which point the
//! durations go in a second record keyed by the entry ordinal and this one keeps
//! its byte identity.
//!
//! Nothing here iterates a set. The entries are a sequence in the order the frame
//! entered them, a submission's slots are walked by index, and the timelines a
//! decoder checks against arrive as a slice. There is no `BTreeMap`, because
//! there is nothing keyed: RFC 0004 is satisfied by there being no iteration
//! order to seed rather than by choosing the ordered container.
//!
//! # Where this holds, and where it does not
//!
//! An entry is built from a [`Wait`], and a [`Wait`] exists only because some
//! timeline admitted it. So **an implicit wait cannot appear in a trace**: there
//! is no constructor that takes a timeline identifier and a number, and a
//! receiver that invented a dependency would have to invent a `Wait` first, which
//! `sync`'s `proof` module makes unavailable. That is the tie between this file
//! and `E3-B05c`, and it is the direction that holds.
//!
//! The direction that does **not** hold from here: nothing in a type makes a
//! frame hand every submission to a recorder. What is held is narrower and worth
//! stating exactly — the chain's own two waiting doors,
//! [`compositor_waits_and_signals`](crate::sync::Chain::compositor_waits_and_signals)
//! and [`present_waits`](crate::sync::Chain::present_waits), take the trace and
//! record into it, so a frame driven
//! through the chain that holds section 08's sentence cannot enter a wait the
//! trace does not name. A submission built straight off [`Submission::EMPTY`] is
//! outside that, and closing it would mean [`Timeline::wait`] itself taking a
//! recorder — a `&mut` argument on the one function every admission goes through,
//! in a crate whose records are meant to carry none. *What would reverse this:* a
//! second door to a wait, at which point the recorder belongs at the admission
//! and not at the chain.
//!
//! # The bound, and what breaks its derivation
//!
//! [`TRACE_WAITS`] is [`STAGES_PER_FRAME`] times [`MAX_WAITS`], and RFC 0101 is
//! why that sentence is followed by this one. That entry's finding is that a
//! constant derived from a count is only as good as the claim that the count is
//! what is being counted, and that the claim decays by addition, silently, and
//! surfaces three subsystems away from the addition.
//!
//! So, said where it can be checked: the derivation is *the stages of section
//! 08's sentence, each submitting once, each submission framing what the record
//! frames.* It breaks the day a frame has a fourth waiting stage, or the day one
//! stage submits twice — a compositor that composites in two passes, or a display
//! with two present engines. Both are addition, and neither touches this file.
//!
//! What is different here from RFC 0101's two bounds is *where the decay shows
//! up*. Theirs reported a full table as an unrelated component's refused spawn.
//! This one reports it in the record whose subject it is: `dropped_waits` is
//! non-zero, `complete` is false, and the reader holding the trace is the reader
//! who needed to know. A bound that is wrong here is a bound that says so.
//!
//! # Fixed width, and why this is in the wire crate
//!
//! A trace is read by somebody other than the component that wrote it — a
//! supervisor deciding a timeout (`E3-B05e`), or a boot reading a component's own
//! subtree back (`E3-B05f`). That is a record crossing a boundary, which is what
//! this crate is for. So it is fixed-width, little-endian by hand, with no
//! `usize` on the wire and no cast of bytes to a struct, on [`crate::store`]'s
//! discipline — and [`Trace::decode`] refuses any byte this build does not read,
//! so a peer built later than this one is visible rather than silently
//! misunderstood. R04.
//!
//! Every wait in an arriving trace goes back through [`Timeline::wait`], the same
//! function a local caller reaches, so there is no decode-side copy of what a
//! wait is. `docs/postmortem/0001` — one grammar split between a reader and a
//! composer, every tree green, the merge unable to boot — is what that is
//! insurance against.
//!
//! There is no floating point in this module. A timeline value is an ordinal and
//! a count of nothing; a drop count is a count of waits. RFC 0004.

// `Chain` is named all over the documentation above and below and is deliberately
// *not* imported: it is referenced by path in the doc links so that this module's
// own code owes nothing to the chain. The recording is a `&mut Trace` argument the
// chain happens to pass; nothing here calls into it, and an import would suggest
// otherwise.
use crate::sync::{
    MAX_WAITS, Refusal, Submission, Timeline, UNSIGNALLED, Wait, held, u32_at, u64_at,
};

/// The stages of the chain `docs/design/ring-scene-boot.html` section 08 names.
///
/// Three, and [`TRACE_WAITS`] is derived from it rather than from a literal — see
/// the module's *the bound, and what breaks its derivation*.
/// Unit: stages.
pub const STAGES_PER_FRAME: usize = 3;

/// How many waits one frame's trace names.
///
/// Derived, and the derivation is stated in the module header along with what
/// breaks it. A frame that enters more is not truncated: the surplus is counted
/// in [`Trace::dropped_waits`] and [`Trace::complete`] goes false.
/// Unit: waits.
pub const TRACE_WAITS: usize = STAGES_PER_FRAME * MAX_WAITS;

/// Bytes one [`Waited`] occupies on the wire.
///
/// Two values, a timeline and a stage: 8, 8, 4 and 4. The `u64`s first so that an
/// entry whose own start is eight-byte aligned has both of them aligned, for
/// [`crate::sync`]'s reason — the alignment buys a reader's arithmetic rather
/// than a compiler's, which is the only kind this crate is allowed to want.
/// Unit: bytes.
pub const ENTRY_BYTES: usize = 24;

/// Bytes the [`Trace`] header occupies: a count, a drop count, and the bytes
/// after them that this build does not read.
/// Unit: bytes.
const TRACE_HEADER_BYTES: usize = 16;

/// Where the entries start.
/// Unit: bytes from the start of the record.
const ENTRIES_AT: usize = TRACE_HEADER_BYTES;

/// Where the count of dropped waits sits.
/// Unit: bytes from the start of the record.
const DROPPED_AT: usize = 4;

/// Bytes a [`Trace`] occupies, whatever it carries.
///
/// A header and [`TRACE_WAITS`] entries, and **no tail** — which is a difference
/// from [`crate::sync::SUBMISSION_BYTES`] worth stating, because that record has
/// one. A submission is written into a channel's inline arena beside the scene
/// deltas it synchronises, so its width is rounded to a cache line and the
/// remainder is where a field goes. A trace is not: it is read, not streamed
/// beside anything, so a tail here would be slack rather than a place. The field
/// that goes next goes in the header bytes this build does not read, and the
/// assertions at the foot of this file are what keep that true rather than
/// convenient.
/// Unit: bytes.
pub const TRACE_BYTES: usize = TRACE_HEADER_BYTES + TRACE_WAITS * ENTRY_BYTES;

/// Which stage of the chain entered a wait.
///
/// Numbered from one so that **zero is not a stage**, which is what stops an
/// untouched slot of a fresh mapping from decoding as *the application waited for
/// nothing* — [`crate::sync::NO_TIMELINE`]'s argument, applied to the one field
/// of an entry that is not a value.
///
/// Numbering from one also puts the niche of `Option<Waited>` at zero, so
/// [`Trace::EMPTY`] is a zero initialiser rather than a constant copied out of
/// `.rodata` at run time. RFC 0100 is where that stopped being a curiosity: a
/// component's image is bounded, and one enum numbered from zero cost 131 KiB.
///
/// It is a role and not a component identity, and the difference is the point: a
/// compositor waiting on eight clients is one waiter and eight timelines, so what
/// distinguishes those eight entries is which timeline they name, not which
/// component entered them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Stage {
    /// Signals N when its deltas are ready. Waits on nothing in the chain as
    /// section 08 states it, which is why
    /// [`application_signals`](crate::sync::Chain::application_signals) takes no
    /// trace.
    Application = 1,
    /// Waits N and signals M when composited.
    Compositor = 2,
    /// Waits M. Signals nothing and holds no timeline.
    Present = 3,
}

impl Stage {
    /// Every stage, in chain order.
    /// Unit: none — stages.
    pub const ALL: [Self; STAGES_PER_FRAME] = [Self::Application, Self::Compositor, Self::Present];

    /// The number this stage is on the wire.
    ///
    /// The discriminant itself, so the number a stage is in this enum and the
    /// number it is on the wire are one value rather than two with a `match`
    /// between them. RFC 0100's second reason for numbering from one.
    /// Unit: none — a stage tag, not a quantity.
    #[must_use]
    pub const fn number(self) -> u32 {
        self as u32
    }

    /// The stage `number` names.
    ///
    /// # Errors
    ///
    /// [`Refusal::NoStage`] for zero — which is what a zeroed record holds — and
    /// for anything past the three. A stage this build does not know is refused
    /// and never ignored, so a peer that has a fourth is visible rather than
    /// silently read as one of these. R04.
    pub const fn of(number: u32) -> Result<Self, Refusal> {
        match number {
            1 => Ok(Self::Application),
            2 => Ok(Self::Compositor),
            3 => Ok(Self::Present),
            _ => Err(Refusal::NoStage),
        }
    }
}

/// One wait a frame entered, and what became of it.
///
/// Fields are private and there is one constructor, which takes a [`Wait`]. A
/// [`Wait`] exists only because a timeline admitted it, so there is no way to
/// write an entry for a wait no producer ever undertook to satisfy — the module's
/// *where this holds, and where it does not*.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Waited {
    /// Which stage entered it. First, so the niche is here.
    waiter: Stage,
    /// The timeline waited on, which names its one producer.
    timeline: u32,
    /// The value waited for.
    value: u64,
    /// The value that timeline had reached when this wait ended, or
    /// [`UNSIGNALLED`] for a wait this frame never got out of.
    released_at: u64,
}

impl Waited {
    /// The entry for `wait`, as entered by `waiter` against its `producer`.
    ///
    /// `released_at` is filled here and not left open when the wait is already
    /// satisfied by the timeline as it stands. A stage that is behind submits a
    /// wait that is satisfied the moment it is entered — [`crate::sync`] says so
    /// and declines to refuse it — and an entry that recorded that as *never
    /// released* would report the ordinary case as the failure this file exists to
    /// make visible. A trace whose outstanding count is a false positive is a
    /// trace nobody reads twice.
    const fn entered(waiter: Stage, wait: &Wait, producer: &Timeline) -> Self {
        let released_at =
            if wait.satisfied_by(producer) { producer.signalled() } else { UNSIGNALLED };
        Self { waiter, timeline: wait.timeline(), value: wait.value(), released_at }
    }

    /// An entry for a wait whose producer this side does not hold.
    ///
    /// Recorded open, because this side cannot say what that timeline has
    /// reached. An honest *unknown* is an unreleased entry; the alternative is a
    /// dropped wait, and a dropped wait is the one thing the record may not do.
    const fn unattributed(waiter: Stage, wait: &Wait) -> Self {
        Self { waiter, timeline: wait.timeline(), value: wait.value(), released_at: UNSIGNALLED }
    }

    /// Which stage entered this wait.
    #[must_use]
    pub const fn waiter(&self) -> Stage {
        self.waiter
    }

    /// The timeline waited on — and therefore who was going to signal it, a
    /// timeline having exactly one producer.
    /// Unit: none — a timeline identifier, not a quantity.
    #[must_use]
    pub const fn timeline(&self) -> u32 {
        self.timeline
    }

    /// The value waited for.
    /// Unit: none — a timeline value, which is an ordinal.
    #[must_use]
    pub const fn value(&self) -> u64 {
        self.value
    }

    /// The value the timeline had reached when this wait ended, or
    /// [`UNSIGNALLED`] for a wait still outstanding when the frame ended.
    ///
    /// [`UNSIGNALLED`] is the hang, after the fact. It is the one thing in this
    /// record a reader chasing a frozen screen is looking for, and the reason that
    /// constant means *not a value* rather than *zero* is that it can serve here
    /// without a second field saying whether this one is real.
    /// Unit: none — a timeline value, which is an ordinal.
    #[must_use]
    pub const fn released_at(&self) -> u64 {
        self.released_at
    }

    /// Did this wait end within the frame?
    #[must_use]
    pub const fn released(&self) -> bool {
        self.released_at != UNSIGNALLED
    }

    /// The entry as it crosses: two values, a timeline, a stage.
    fn encode(&self) -> [u8; ENTRY_BYTES] {
        let mut out = [0u8; ENTRY_BYTES];
        out[0..8].copy_from_slice(&self.value.to_le_bytes());
        out[8..16].copy_from_slice(&self.released_at.to_le_bytes());
        out[16..20].copy_from_slice(&self.timeline.to_le_bytes());
        out[20..24].copy_from_slice(&self.waiter.number().to_le_bytes());
        out
    }
}

/// What one frame waited for.
///
/// Fixed-width, with no clock in it. The entries fill from the front and the used
/// ones are a prefix, so [`Trace::len`] is counted from the array for
/// [`Submission::wait_count`]'s reason rather than stored beside it: a second
/// statement of a derivable fact is a thing two writers disagree about while both
/// pass their own tests.
///
/// `dropped_waits` is the exception and is **not** derivable — it is the number of
/// waits this frame entered that the record could not hold, and there is nowhere
/// else that fact could live. A count and not a flag, because *how far short* is
/// what tells a reader whether the bound is one out or an order out. RFC 0101.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trace {
    /// The waits, in the order the frame entered them.
    entries: [Option<Waited>; TRACE_WAITS],
    /// Waits entered past what this record frames. Never a silent drop.
    /// Unit: waits.
    dropped_waits: u32,
}

impl Trace {
    /// A frame that has entered nothing.
    ///
    /// Zeroes, and [`Stage`] is numbered from one so that this is true rather than
    /// nearly true. RFC 0100.
    pub const EMPTY: Self = Self { entries: [None; TRACE_WAITS], dropped_waits: 0 };

    /// Record every wait `submission` carries, as entered by `waiter`.
    ///
    /// `known` is the timelines this side holds, scanned linearly for
    /// [`Submission::decode`]'s reason — the chain has two and a compositor's set
    /// is the clients it is compositing, and a hash map's iteration order is
    /// seeded per process, so a record that depended on it would not be
    /// byte-identical for one seed. RFC 0004.
    ///
    /// Infallible on purpose. A recorder that could refuse is a recorder a caller
    /// is tempted to stop calling on the path where it matters, and the one thing
    /// that must not happen is a wait entered outside the record. The bound is
    /// handled by counting; the unknown producer is handled by recording the wait
    /// open. Neither is handled by dropping a wait.
    pub fn entering(&mut self, waiter: Stage, submission: &Submission, known: &[Timeline]) {
        let count = submission.wait_count();
        for index in 0..count {
            let Some(wait) = submission.wait(index) else { continue };
            let entry = match held(known, wait.timeline()) {
                Ok(producer) => Waited::entered(waiter, &wait, &producer),
                Err(_) => Waited::unattributed(waiter, &wait),
            };
            self.push(entry);
        }
    }

    /// Record that `timeline` reached `landed`, releasing every wait in this trace
    /// that was waiting on it at or below that value.
    ///
    /// This is where *who signalled it* is answered: a landing is a producer
    /// having got somewhere, and the entries it closes are the waits it closed.
    /// Called from [`Chain::landed`](crate::sync::Chain::landed), which is the one
    /// place a landing is recorded as true.
    ///
    /// A wait already released keeps the value that released it. The first landing
    /// at or above a wait's value is the one that satisfied it, and overwriting
    /// with a later one would turn *who signalled it* into *who signalled last* —
    /// a different question, and not the one the exit asks.
    pub fn released(&mut self, timeline: u32, landed: u64) {
        for slot in &mut self.entries {
            if let Some(entry) = slot
                && entry.timeline == timeline
                && !entry.released()
                && entry.value <= landed
            {
                entry.released_at = landed;
            }
        }
    }

    /// How many waits this trace names.
    /// Unit: waits.
    #[must_use]
    pub const fn len(&self) -> usize {
        let mut at = 0;
        while at < TRACE_WAITS {
            if self.entries[at].is_none() {
                return at;
            }
            at += 1;
        }
        TRACE_WAITS
    }

    /// Did this frame enter no wait at all?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries[0].is_none()
    }

    /// The entry at `index`, or `None` past the end.
    #[must_use]
    pub const fn entry(&self, index: usize) -> Option<Waited> {
        if index >= TRACE_WAITS {
            return None;
        }
        self.entries[index]
    }

    /// Waits this frame entered that this record could not hold.
    ///
    /// Non-zero means the bound's derivation has stopped being true, and the
    /// record says so in the place the reader who needs to know is already
    /// looking. RFC 0101.
    /// Unit: waits.
    #[must_use]
    pub const fn dropped_waits(&self) -> u32 {
        self.dropped_waits
    }

    /// Does this trace name every wait its frame entered?
    ///
    /// The question `E3-B05`'s negative has to ask before it can be believed: *no
    /// implicit wait appears in this trace* says nothing about a trace that is
    /// missing waits for a reason of its own.
    #[must_use]
    pub const fn complete(&self) -> bool {
        self.dropped_waits == 0
    }

    /// How many of this frame's waits were still outstanding when it ended.
    ///
    /// The hang, counted. Not refused and not an error here: a trace is taken at
    /// the end of a frame, and a stage not yet released is ordinary in a pipeline
    /// that is still running. What makes it evidence is that a reader can ask the
    /// question at all — before this record the answer was a process that was
    /// simply still there, and a bug report that said the screen froze.
    /// Unit: waits.
    #[must_use]
    pub const fn unreleased(&self) -> usize {
        let mut at = 0;
        let mut count = 0;
        while at < TRACE_WAITS {
            match self.entries[at] {
                Some(entry) => {
                    if !entry.released() {
                        count += 1;
                    }
                }
                None => return count,
            }
            at += 1;
        }
        count
    }

    /// Append, or count the drop.
    fn push(&mut self, entry: Waited) {
        let at = self.len();
        if at == TRACE_WAITS {
            // Saturating rather than wrapping: a count that wrapped to zero would
            // say the record is complete, which is the one answer this field
            // exists to prevent.
            self.dropped_waits = self.dropped_waits.saturating_add(1);
            return;
        }
        self.entries[at] = Some(entry);
    }

    /// The record as it crosses.
    ///
    /// Little-endian and by hand, so nothing here depends on the host's word size,
    /// alignment rules or byte order. Every byte no entry reaches stays zero, which
    /// is what makes two recorders of one frame produce one image and what gives
    /// [`Trace::decode`]'s whole-image comparison something to compare against.
    #[must_use]
    pub fn encode(&self) -> [u8; TRACE_BYTES] {
        let mut out = [0u8; TRACE_BYTES];
        out[0] = self.len() as u8;
        out[DROPPED_AT..DROPPED_AT + 4].copy_from_slice(&self.dropped_waits.to_le_bytes());
        for (index, slot) in self.entries.iter().enumerate() {
            if let Some(entry) = slot {
                let at = ENTRIES_AT + index * ENTRY_BYTES;
                out[at..at + ENTRY_BYTES].copy_from_slice(&entry.encode());
            }
        }
        out
    }

    /// Decode against the timelines this side holds.
    ///
    /// Every entry's wait goes back through [`Timeline::wait`], so a trace that
    /// arrived is admitted by exactly the function a local wait is, and there is no
    /// second statement of what a wait is to fall out of step with the first.
    ///
    /// # Errors
    ///
    /// [`Refusal::Full`] for a count this record cannot frame; [`Refusal::NoStage`]
    /// for a stage this build does not know; [`Refusal::NoSuchTimeline`] for a
    /// timeline this side does not hold; whatever the wait's admission refuses,
    /// including [`Refusal::Unreachable`]; [`Refusal::NotMonotonic`] for a release
    /// below the value it claims to have released; [`Refusal::Overcommitted`] for a
    /// release above what that timeline's producer ever undertook to signal; and
    /// [`Refusal::Reserved`] for any byte this build does not read.
    pub fn decode(raw: &[u8; TRACE_BYTES], known: &[Timeline]) -> Result<Self, Refusal> {
        let count = raw[0] as usize;
        if count > TRACE_WAITS {
            return Err(Refusal::Full);
        }
        let mut trace = Self::EMPTY;
        trace.dropped_waits = u32_at(raw, DROPPED_AT);
        for index in 0..count {
            let at = ENTRIES_AT + index * ENTRY_BYTES;
            let value = u64_at(raw, at);
            let released_at = u64_at(raw, at + 8);
            let id = u32_at(raw, at + 16);
            let waiter = Stage::of(u32_at(raw, at + 20))?;
            let timeline = held(known, id)?;
            // The wait, through the one door. A trace naming a wait no producer
            // could ever have satisfied is refused here for the reason a submission
            // carrying one is: this side would be recording, as history, something
            // that cannot have happened.
            let wait = timeline.wait(value)?;
            if released_at != UNSIGNALLED {
                if released_at < wait.value() {
                    return Err(Refusal::NotMonotonic);
                }
                if released_at > timeline.ceiling() {
                    return Err(Refusal::Overcommitted);
                }
            }
            trace.entries[index] = Some(Waited {
                waiter,
                timeline: wait.timeline(),
                value: wait.value(),
                released_at,
            });
        }
        // The catch-all, and the reason no zero region above is checked by hand:
        // the three header bytes after the count, the eight this build does not
        // read, and every entry past the count are zero in the rebuilt image, so
        // anything a peer wrote in them fails this comparison. A list of regions is
        // correct on the day it is written and silent afterwards; this is correct on
        // the day a field is added.
        if trace.encode() != *raw {
            return Err(Refusal::Reserved);
        }
        Ok(trace)
    }
}

// The widths, asserted beside the record rather than in a table somewhere else. An
// entry that grew a field and did not grow `ENTRY_BYTES` stops the build here
// rather than overlapping its neighbour.
const _: () = assert!(8 + 8 + 4 + 4 == ENTRY_BYTES);
const _: () = assert!(DROPPED_AT + 4 <= TRACE_HEADER_BYTES);
const _: () = assert!(ENTRIES_AT + TRACE_WAITS * ENTRY_BYTES == TRACE_BYTES);
// The count fits the byte it is written into, for `MAX_WAITS`' reason: a capacity
// past 255 would make `len` lossy on the wire, silently, on the day
// `STAGES_PER_FRAME` or `MAX_WAITS` moves.
const _: () = assert!(TRACE_WAITS <= u8::MAX as usize);
// An entry and an `Option` of one are the same width, which is what says the niche
// is being used rather than a discriminant word being added — so `Trace::EMPTY`
// is a zero initialiser and not a constant copied out of `.rodata` at run time.
// RFC 0100 is the incident: one enum's niche cost a component 131 KiB of image.
//
// What this assertion does **not** notice, said because it was mutated in and it
// did not: `Application = 0` keeps the width, because the niche simply moves to
// three. The property numbering from zero breaks is a different one — *zero is
// not a stage* — and what catches that is `Stage::of(0)` refusing, asserted twice
// in the tests below. Two properties, two instruments, and this one is width.
const _: () = assert!(core::mem::size_of::<Option<Waited>>() == ENTRY_BYTES);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync::Chain;
    use f_env::{Env, SeededEnv};

    /// The application's timeline, and the compositor's.
    const APP: u32 = 0x0000_0011;
    const COMP: u32 = 0x0000_0022;

    /// One seed, named once. Two runs of [`one_frame`] at this seed are what the
    /// exit's third clause is about.
    const SEED: u64 = 0x5eed_0b05;

    /// The seeds the vacuity check below runs, and it needs at least two that
    /// take different branches of the interleaving.
    const SEEDS: [u64; 5] = [SEED, 1, 2, 3, 4];

    /// One frame, driven from a seed through the real [`Chain`](crate::sync::Chain).
    ///
    /// The seed decides the two promised values and — the part that matters — the
    /// *interleaving*: whether the application's signal lands before the
    /// compositor enters its wait or after. Those two orders produce genuinely
    /// different traces, one where the compositor's entry is released at entry and
    /// one where it is released by a landing, so a recorder that was not a
    /// function of the frame alone has somewhere to show it.
    ///
    /// Returns the trace and, counted here rather than read off the record, how
    /// many waits the frame entered — so *names every wait* is checked against a
    /// second count rather than against the trace's own.
    fn one_frame(seed: u64) -> (Trace, usize) {
        let mut env = SeededEnv::new(seed, 7);
        let n = 1 + env.next_u64() % 64;
        let m = 1 + env.next_u64() % 64;
        let lands_before_the_wait = env.scheduler().choose(2) == 0;

        let mut chain = Chain::new(
            Timeline::declare(APP).expect("a timeline"),
            Timeline::declare(COMP).expect("a timeline"),
        )
        .expect("two distinct timelines are a chain");
        let mut trace = Trace::EMPTY;

        chain.application_signals(n).expect("the application promises and signals");
        if lands_before_the_wait {
            chain.landed(APP, n, &mut trace).expect("the application's signal lands");
        }
        chain
            .compositor_waits_and_signals(n, m, &mut trace)
            .expect("the compositor waits and signals");
        if !lands_before_the_wait {
            chain.landed(APP, n, &mut trace).expect("the application's signal lands");
        }
        chain.present_waits(m, &mut trace).expect("the present engine waits");
        chain.landed(COMP, m, &mut trace).expect("the compositor's signal lands");
        // Two: the compositor's wait on N and the present engine's wait on M. The
        // application waits on nothing, which is the chain's shape and not an
        // omission.
        (trace, 2)
    }

    /// The timelines the frame at `seed` ended with, for a decoder to check an
    /// arriving trace against.
    fn ended_with(seed: u64) -> [Timeline; 2] {
        let mut env = SeededEnv::new(seed, 7);
        let n = 1 + env.next_u64() % 64;
        let m = 1 + env.next_u64() % 64;
        [
            Timeline::adopt(APP, n, n).expect("the application's timeline"),
            Timeline::adopt(COMP, m, m).expect("the compositor's timeline"),
        ]
    }

    #[test]
    fn one_seed_gives_one_image_byte_for_byte() {
        // The exit's third clause, and it is a comparison of *bytes* and not of
        // values: two traces equal as values but encoding differently would be two
        // records a reader cannot diff, and diffing is the whole use of this one.
        let (first, entered) = one_frame(SEED);
        let (second, again) = one_frame(SEED);
        assert_eq!(entered, again);
        assert_eq!(first.encode(), second.encode(), "one seed, two images");
        // And not vacuously identical: the record holds this frame's waits, so a
        // recorder that recorded nothing would pass the comparison above and fail
        // here.
        assert_eq!(first.len(), entered, "the trace names every wait the frame entered");
        assert!(!first.is_empty());
        assert!(first.complete());

        // Different seeds are different frames, which is what says the image is a
        // function of the frame rather than a constant.
        //
        // The two loops are one loop that was split, and the reason is worth the
        // line: a frame that stopped reading its seed was mutated in and the
        // *round trip* went red first, on timelines that no longer matched, so
        // the assertion below — the one this paragraph is about — was never
        // reached. A guard a mutation cannot reach is a guard that is not there.
        let mut images = [[0u8; TRACE_BYTES]; SEEDS.len()];
        for (slot, seed) in images.iter_mut().zip(SEEDS) {
            let (trace, count) = one_frame(seed);
            assert_eq!(trace.len(), count);
            *slot = trace.encode();
        }
        assert!(
            images.iter().any(|image| *image != images[0]),
            "every seed produced one image, so this test is about a constant"
        );
        // And every one of them round-trips through its bytes, against the
        // timelines that frame ended with.
        for (image, seed) in images.iter().zip(SEEDS) {
            let (trace, _) = one_frame(seed);
            assert_eq!(Trace::decode(image, &ended_with(seed)), Ok(trace));
        }
    }

    #[test]
    fn a_wait_the_frame_never_got_out_of_is_named() {
        // The failure `crate::sync` says leaves no log line, leaving one. The
        // compositor waits on a value the application promised and does not reach:
        // admissible when it was admitted, so no refusal can catch it, and before
        // this record the only evidence was a frame that never appeared.
        let mut chain = Chain::new(
            Timeline::declare(APP).unwrap().promise(9).unwrap(),
            Timeline::declare(COMP).unwrap(),
        )
        .unwrap();
        let mut trace = Trace::EMPTY;
        chain.compositor_waits_and_signals(9, 4, &mut trace).unwrap();
        assert_eq!(trace.len(), 1);
        assert_eq!(trace.unreleased(), 1);
        let entry = trace.entry(0).unwrap();
        assert_eq!(entry.waiter(), Stage::Compositor);
        assert_eq!(entry.timeline(), APP, "who was going to signal it");
        assert_eq!(entry.value(), 9);
        assert_eq!(entry.released_at(), UNSIGNALLED);
        assert!(!entry.released());

        // And when it lands, the entry names the value that released it.
        chain.landed(APP, 9, &mut trace).unwrap();
        assert_eq!(trace.entry(0).unwrap().released_at(), 9);
        assert_eq!(trace.unreleased(), 0);
        // A later landing does not rewrite it: the first landing at or above the
        // value is the one that satisfied the wait, and *who signalled it* is not
        // *who signalled last*.
        chain.application_signals(20).unwrap();
        chain.landed(APP, 20, &mut trace).unwrap();
        assert_eq!(trace.entry(0).unwrap().released_at(), 9, "who signalled it, not who last did");
    }

    #[test]
    fn a_wait_already_satisfied_when_it_was_entered_is_not_a_hang() {
        // The other half of the boundary, stated because a record that reported
        // this as outstanding would have replaced a hang with a false positive. A
        // stage that is behind enters a wait that is satisfied the moment it is
        // entered, and `crate::sync` declines to refuse it.
        let application = Timeline::adopt(APP, 7, 9).unwrap();
        let compositor = Timeline::declare(COMP).unwrap().promise(4).unwrap();
        let known = [application, compositor];
        let submission = Submission::EMPTY.waiting(application.wait(3).unwrap()).unwrap();
        let mut trace = Trace::EMPTY;
        trace.entering(Stage::Compositor, &submission, &known);
        assert_eq!(trace.len(), 1);
        assert_eq!(trace.unreleased(), 0);
        assert_eq!(trace.entry(0).unwrap().released_at(), 7, "the value it was released at");
        // A wait *above* what has landed is outstanding, which is the same
        // boundary from the other side.
        let mut behind = Trace::EMPTY;
        let later = Submission::EMPTY.waiting(application.wait(8).unwrap()).unwrap();
        behind.entering(Stage::Compositor, &later, &known);
        assert_eq!(behind.unreleased(), 1);

        // A wait on a timeline this side does not hold is recorded open rather than
        // dropped: this side cannot say what that timeline has reached.
        let mut blind = Trace::EMPTY;
        blind.entering(Stage::Compositor, &submission, &[compositor]);
        assert_eq!(blind.len(), 1);
        assert_eq!(blind.unreleased(), 1);
    }

    #[test]
    fn a_wait_past_the_bound_is_counted_and_not_skipped() {
        // RFC 0101's rule where it applies to this file. The bound is derived from
        // `STAGES_PER_FRAME * MAX_WAITS`, the derivation decays by addition, and
        // what this asserts is that when it decays the record says so — because a
        // trace that silently dropped a wait would make `E3-B05`'s negative
        // unfalsifiable rather than false, which is the worse of the two.
        let mut known = [Timeline::adopt(1, 0, 9).unwrap(); MAX_WAITS];
        for (index, slot) in known.iter_mut().enumerate() {
            *slot = Timeline::adopt(index as u32 + 1, 0, 9).unwrap();
        }
        let mut full = Submission::EMPTY;
        for timeline in &known {
            full = full.waiting(timeline.wait(1).unwrap()).unwrap();
        }
        let mut trace = Trace::EMPTY;
        // `STAGES_PER_FRAME` full submissions fill the record exactly, which is the
        // derivation executed rather than restated.
        for _ in 0..STAGES_PER_FRAME {
            trace.entering(Stage::Compositor, &full, &known);
        }
        assert_eq!(trace.len(), TRACE_WAITS);
        assert!(trace.complete());
        assert_eq!(trace.dropped_waits(), 0);

        // One stage more, and the record says what it could not hold.
        trace.entering(Stage::Present, &full, &known);
        assert_eq!(trace.len(), TRACE_WAITS, "the record does not grow");
        assert!(!trace.complete(), "a full trace that claimed to be complete would be a lie");
        assert_eq!(trace.dropped_waits(), MAX_WAITS as u32, "how far short, not merely that it is");
        // And the count crosses, so a reader on the other side of a ring learns it
        // too rather than receiving a complete-looking record.
        let arrived = Trace::decode(&trace.encode(), &known).unwrap();
        assert_eq!(arrived.dropped_waits(), MAX_WAITS as u32);
        assert!(!arrived.complete());
    }

    #[test]
    fn nothing_in_a_trace_is_ignored() {
        // There is no byte of a trace that can be changed without either changing
        // what the trace says or being refused. A field this record read and then
        // dropped would show up here as a byte that can be flipped and still decode
        // to the value it started as. The ceilings are `u64::MAX` so that most
        // flips of a value are not refused as unreachable — which would make this
        // pass for a reason other than the one it is written for.
        let application = Timeline::adopt(APP, 9, u64::MAX).unwrap();
        let compositor = Timeline::adopt(COMP, 4, u64::MAX).unwrap();
        let known = [application, compositor];
        let submission = Submission::EMPTY
            .waiting(application.wait(9).unwrap())
            .unwrap()
            .waiting(compositor.wait(4).unwrap())
            .unwrap();
        let mut trace = Trace::EMPTY;
        trace.entering(Stage::Present, &submission, &known);
        let raw = trace.encode();
        assert_eq!(Trace::decode(&raw, &known), Ok(trace));
        for at in 0..TRACE_BYTES {
            let mut damaged = raw;
            damaged[at] ^= 0xFF;
            assert_ne!(
                Trace::decode(&damaged, &known),
                Ok(trace),
                "byte {at} of the trace is ignored"
            );
        }
    }

    #[test]
    fn a_byte_this_build_does_not_read_is_refused_and_not_dropped() {
        // The header bytes after the count and the drop count, and every entry past
        // the end, are exactly where a future field goes. A decoder that skipped
        // them would accept a record written by a peer that has that field: two
        // peers with different beliefs about one frame, and no way for either to
        // find out. R04.
        let application = Timeline::adopt(APP, 9, 9).unwrap();
        let known = [application];
        let submission = Submission::EMPTY.waiting(application.wait(9).unwrap()).unwrap();
        let mut trace = Trace::EMPTY;
        trace.entering(Stage::Application, &submission, &known);
        let raw = trace.encode();
        for at in (1..DROPPED_AT)
            .chain(DROPPED_AT + 4..TRACE_HEADER_BYTES)
            .chain(ENTRIES_AT + ENTRY_BYTES..TRACE_BYTES)
        {
            let mut damaged = raw;
            damaged[at] = 1;
            assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::Reserved), "byte {at}");
        }
        // A count the record cannot frame is refused rather than clamped.
        let mut damaged = raw;
        damaged[0] = TRACE_WAITS as u8 + 1;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::Full));
    }

    #[test]
    fn a_trace_is_admitted_by_the_same_rules_a_wait_is() {
        // Every refusal `crate::sync` has for a wait is a refusal for a trace entry
        // naming one, because the entry goes back through the same constructor.
        // This is the test that says so rather than a sentence claiming it.
        let application = Timeline::adopt(APP, 9, 9).unwrap();
        let known = [application];
        let submission = Submission::EMPTY.waiting(application.wait(9).unwrap()).unwrap();
        let mut trace = Trace::EMPTY;
        trace.entering(Stage::Compositor, &submission, &known);
        let raw = trace.encode();

        // A value above anything the producer undertook to signal.
        let mut damaged = raw;
        damaged[ENTRIES_AT] = 10;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::Unreachable));
        // The value every timeline starts at, which is not a wait.
        let mut damaged = raw;
        damaged[ENTRIES_AT] = 0;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::NoValue));
        // A timeline this side does not hold.
        assert_eq!(Trace::decode(&raw, &[]), Err(Refusal::NoSuchTimeline));
        // A stage this build does not know, including the zero a fresh mapping
        // holds.
        let mut damaged = raw;
        damaged[ENTRIES_AT + 20] = 4;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::NoStage));
        let mut damaged = raw;
        damaged[ENTRIES_AT + 20] = 0;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::NoStage));
        assert_eq!(Stage::of(0), Err(Refusal::NoStage));
        for stage in Stage::ALL {
            assert_eq!(Stage::of(stage.number()), Ok(stage));
        }
        // A release below the value it claims to have released is not a release.
        let mut damaged = raw;
        damaged[ENTRIES_AT + 8] = 1;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::NotMonotonic));
        // And a release above what the producer ever undertook to signal is a
        // landing that cannot have happened.
        let mut damaged = raw;
        damaged[ENTRIES_AT + 8] = 200;
        assert_eq!(Trace::decode(&damaged, &known), Err(Refusal::Overcommitted));
    }
}
