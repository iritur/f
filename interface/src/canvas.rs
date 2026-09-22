// SPDX-License-Identifier: Apache-2.0 OR MIT
//! What a self-rendering surface declares besides its pixels, and the floor
//! below which it is not a canvas but an escape.
//!
//! Section 13 of `docs/design/ring-scene-boot.html` calls this the case that
//! sank every predecessor. An audio timeline or a 3-D viewport draws its own
//! pixels, and if such a node becomes an opaque rectangle then every projection
//! except the local display goes blind exactly where the application's real work
//! happens — at which point the thesis is true of toolbars and dialogs and of
//! nothing anybody opened the application for. The design's answer is that a
//! canvas declares *both* its rendering *and* a semantic model of its content,
//! and it states the test so that it can fail: **can an agent select the clip
//! between 4.2 s and 6.8 s on track 3, and can a screen reader describe it,
//! without either one seeing a single pixel?**
//!
//! That sentence is a test in this module, with those numbers, and it is the
//! exit of `E3-D02` in executable form. It is worth more than five tests that
//! paraphrase it, because it is the design document's own falsification: if it
//! ever goes red, the thing that has failed is the pillar and not the test.
//!
//! # The floor
//!
//! The hard question this module exists to answer is not *how does a timeline
//! declare itself* — it is **what must any canvas declare to be admitted at
//! all**. A canvas that declares nothing is the opaque rectangle, and a canvas
//! that declares almost nothing is the opaque rectangle with a caption. So the
//! floor is stated as four clauses, and all four are enforced rather than
//! recommended:
//!
//! 1. **Which surface.** The declaration names the [`Role::Canvas`] node it is
//!    about, and that node is in the tree and is a canvas.
//! 2. **In what dimension.** The ordered dimension its content is placed in, and
//!    the scale of that dimension — here a [`TimeBase`]. A number with no
//!    declared base is a number no projection can say out loud.
//! 3. **What is selected in it.** A [`Selection`], which may be
//!    [`Selection::NOTHING`] — but *nothing is selected* is a statement, and
//!    saying nothing is not.
//! 4. **Where every occupant it declares sits, and at least one occupant, and
//!    nothing under it that cannot be placed at all.** Every [`Role::Clip`] and
//!    [`Role::Marker`] the tree declares under that canvas has a placement in
//!    the dimension; a canvas that places none at all is refused outright; and
//!    the canvas's subtree holds nothing but lanes, clips and markers. This is
//!    the clause that does the work, and it is
//!    the one `node.rs` cannot enforce: a canvas may satisfy
//!    [`crate::node::check`] — children exist, nothing is misplaced — and still
//!    be opaque, because knowing that a clip exists is not knowing when it is.
//!
//! The first three are the arguments of [`Arrangement::declaring`], so they
//! cannot be omitted or defaulted. The fourth is checked against the tree by
//! [`Arrangement::admit`], because it is a statement about two things at once.
//!
//! ## Why clause 4 has a second half
//!
//! Because without it the clause is met by declaring nothing, and RFC 0078
//! contains the argument against itself: *the cheapest possible compliance with
//! a rule about children is to declare children, and a rule whose cheapest
//! compliance is worthless is a rule that teaches applications to comply
//! worthlessly*. That is said there about [`crate::node::Defect::EmptyCanvas`],
//! and the first version of this module then made the identical mistake one
//! level up — a canvas declaring one empty [`Role::Track`] and placing nothing
//! satisfied every clause and was admitted as a full [`Participating`]. That is
//! the whole sequence drawn in pixels with a caption under it: one node, no
//! content, four clauses green. The cheapest escape must not also be the
//! cheapest compliance, so [`Escape::NothingPlaced`] refuses it.
//!
//! The price is named rather than hidden. **An application whose canvas is
//! genuinely empty — a new project, three lanes, nothing on them — is refused,
//! and has to declare that node as the [`Role::Group`] it currently is until it
//! has content.** That is the trade RFC 0077 already made when it refused a
//! childless canvas, taken one step further for the same reason, and it is not
//! free: the node's role changes the moment the first clip lands. Its
//! [`NodeId`] does not, which is what makes the change survivable, and RFC 0078
//! carries the reversal condition — the first application for which that role
//! change costs a projection something real.
//!
//! ## Why clause 4 is also about what is not a clip
//!
//! Because the clause counts clips and markers, and the vocabulary does not make
//! a canvas's children be clips and markers. [`Role::List`], [`Role::Group`],
//! [`Role::Item`], [`Role::Text`], [`Role::Image`] and [`Role::Command`] all
//! satisfy `accepts_parent(Some(Canvas))`, so the same argument that closed the
//! zero-occupant case arrives one level down: place one token clip, meet every
//! clause, and declare the content the canvas actually draws as a list hanging
//! off the canvas, placed nowhere and asked nothing. The floor would be a form
//! filled in with a clip.
//!
//! So the canvas's subtree is closed to the three roles the dimension has a
//! place for — a lane, a clip, a marker — and anything else under it is
//! [`Escape::Unplaceable`]. The price, again named rather than hidden: **a
//! canvas may not carry a caption, a legend, an overlay button or a group of
//! them among its descendants.** Those are real things applications draw over
//! timelines, and the declaration for them is a sibling of the canvas rather
//! than a child — which is where a projection that lays out a surface would
//! rather find them anyway, since a node inside a canvas is a node the
//! application claims to be drawing itself. A name for the canvas is a
//! [`Role::Label`] beside it pointing with
//! [`crate::node::Relation::LabelledBy`], which is how the vocabulary already
//! spells naming. RFC 0078 carries the reversal condition: the first canvas
//! whose overlay genuinely belongs inside it.
//!
//! # Why the floor is a type and not a rule
//!
//! Every query in this module is a method on [`Participating`], and the only way
//! to obtain one is [`Arrangement::admit`]. There is no way to ask which clip is
//! at 5 s of a canvas that has not met the floor, because the value that answers
//! the question cannot be constructed. A projection that wants to present a
//! canvas must therefore handle [`Escape`] — and the name of that type is the
//! reading section 13 asks for: **every escape into canvas is a bug report about
//! the vocabulary rather than an application's choice**, so every way of failing
//! to participate is spelled as one word, in one enum, whether it failed when it
//! was written or when it was checked.
//!
//! A convention would have been cheaper and would have failed the way
//! conventions fail: the declaration that omits the placements is the one
//! written last, under pressure, by somebody who has the pixels working. The
//! escape hatch is never a feature anybody asks for — it is the shape of what is
//! left when a deadline removes everything that was not enforced.
//!
//! # Where the pixels are, and why this module can hold one without looking
//!
//! Section 13's sentence has two halves, and the second is the one a module
//! building the first will forget: a canvas declares *both* its custom rendering
//! *and* a semantic model of its content. The rendering is not invented here,
//! because the vocabulary already has the word. [`crate::node::Content::Media`]
//! is *pixels or samples the application holds elsewhere, named by capability*,
//! and a canvas carrying one is saying **these pixels are mine, here is the
//! handle**. [`Participating::rendering`] is how a projection asks for it.
//!
//! Nothing here resolves that handle, decodes it, or derives an answer from it,
//! and that is the property rather than the limitation: every query below is
//! answered from the declaration, so a consumer that cannot render — a screen
//! reader, an agent, a test, a remote client — gets the same answers as the
//! display, and the display keeps pixels nobody else has to understand. The two
//! halves touch at exactly one point, which is that declaring the first half
//! obliges nothing and declaring *neither* half is refused: a rendering with
//! nothing placed under it is the opaque rectangle wearing a capability, and
//! that is what clause 4's second half is for.
//!
//! # Addressable is not scriptable, and they are asked separately
//!
//! Locating a clip and being able to do something to it are two facts, and a
//! module that demonstrated only the first would have met half of what the exit
//! asks. [`Participating::operable`] is the second: for a node this canvas
//! declares, it answers what an agent may invoke on it — the capability the node
//! carries, whether the application currently declares it operable, or
//! [`Operable::Inert`] for one carrying nothing. The same answer rides on every
//! [`Phrase`], so a consumer that linearised the canvas can act on what it heard
//! without walking the tree again. That is the difference between a description
//! and a script.
//!
//! **It is asked of a canvas, and this module offers no way to ask it of a
//! node.** The answer is computed from the node's own declaration, and that
//! computation is private: there is no `Operable::of(node)` for a caller to
//! reach for, because a module publishing one would be publishing exactly the
//! answer [`Participating::operable`] refuses — what may be invoked on a node no
//! canvas admitted. For one round that is what this module was: the scoped
//! method, and the unscoped read public beside it with the refusal documented
//! over it. The same rule reaches the other questions —
//! [`Participating::lane_of`] and the three queries over a lane refuse a node or
//! a lane this canvas does not declare, rather than answering [`Sole::Nothing`],
//! which is a real answer about a real lane and not a way of saying *not mine*.
//!
//! **Carrying an intent is deliberately not a fifth clause of the floor.** A
//! read-only timeline is real — a review copy, a locked lane, a render preview —
//! and `node.rs` spells read-only as the absence of an intent. A floor demanding
//! one would refuse an honest declaration in order to catch a dishonest one,
//! which is the trade this module refuses everywhere else. What it does instead
//! is make the difference *sayable*: an inert occupant answers `Inert`, not
//! silence, so a projection can tell *nothing to invoke here* from *nobody
//! asked*.
//!
//! # Why the time base is the application's, and why it is not milliseconds
//!
//! A timeline's time base is a real decision and not a formatting detail, which
//! is why it is clause 2 of the floor and not a constant in this file.
//!
//! Start with what is not available: 4.2 and 6.8 are not expressible in this
//! tree. RFC 0004 forbids the two floating-point types outright — the two
//! architectures do not agree on them — so every time here is an integer, and an
//! integer is a time only once something says what one of them is worth. The
//! lazy answer is the millisecond, and it is wrong for exactly the case this
//! module exists for. Audio at 48 kHz has a sample every 20.83 µs; video at
//! 30000/1001 frames a second has a frame every 33.3667 ms. A base that cannot
//! name a sample boundary makes every cut an approximation, and the
//! approximation is invisible: the agent asks for the clip between 4.2 s and
//! 6.8 s, gets one that begins a sample away from where the application believes
//! it begins, and nobody finds out until something is rendered and clicks.
//!
//! So the base travels with the declaration, as ticks per second, and
//! [`TimeBase::FLICKS`] is offered because it has the property worth wanting: at
//! 705 600 000 ticks a second, one frame at 24, 25, 30, 48, 50, 60, 90, 100 or
//! 120 frames a second — and at NTSC's 30000/1001 — is a whole number of ticks,
//! and so is one sample at 8, 16, 22.05, 24, 32, 44.1, 48, 88.2 or 96 kHz.
//! [`TimeBase::names`] is that property, asked as a question, and it takes the
//! rate as **a numerator and a denominator** because the rate that makes the
//! question worth asking is not an integer. A predicate that could only be given
//! `29_970` would answer *false* for NTSC — a rate this base names exactly — and
//! would send an application off to declare a base it does not need. Choosing a
//! base is choosing what can be named exactly, and an application whose content
//! this one does not divide should declare its own rather than round into ours.
//!
//! [`Ticks`] is signed, which is a decision and not a default: a count-in, a
//! pre-roll and a clip dragged left of the origin are all real, and a base that
//! cannot express time before zero forces the application to move the origin,
//! which changes every other number it has already declared.
//!
//! # Why a time is stated twice, in two units
//!
//! The declaration holds ticks; a description holds seconds. That is duplication
//! only if both are authored, and only one is: [`Spoken`] is computed from the
//! ticks and the base at the moment a projection asks, by
//! [`TimeBase::seconds_from_ticks`], and it exists because a screen reader does
//! not know what a flick is and should not have to. The conversion is exact
//! whenever the base divides the value — which is what choosing a base is for —
//! and truncates toward zero otherwise, which is said here because a description
//! that rounds is still a description, while a *declaration* that rounded would
//! be a lie every projection would then repeat faithfully.
//!
//! # Why the selection is a stretch of the dimension, not a set of clips
//!
//! [`crate::node::StateSet::SELECTED`] already exists, and a selection stored in
//! both places would be one fact spelled twice, which RFC 0077 refuses elsewhere
//! for good reasons. It is not spelled twice here, because these are two
//! different facts. *This clip is selected* is a property of a clip, and the
//! vocabulary holds it. *4.2 s to 6.8 s on this lane is selected* is a property
//! of the **dimension**; it may cover no clip at all, or half of one, and there
//! is nowhere in the vocabulary to put it — which is precisely why a timeline
//! sank predecessors that had only node state. A ripple delete over an empty
//! stretch is a real operation on a real selection containing nothing.
//!
//! The two meet at [`Participating::selected`], which answers *which occupants
//! does this stretch touch* — and that is the join an agent uses: select a
//! range, learn an identity, invoke the capability that identity carries. The
//! agent never names a pixel and never names an index.
//!
//! # What a viewport would need, and why it is not built here
//!
//! Section 13 names two canvases and this module builds one. A viewport declares
//! objects, a camera and a selection, and the first two are not [`Extent`]s.
//!
//! The floor's four clauses hold for it unchanged, which is the useful part of
//! having stated them abstractly: a viewport names its surface, declares the
//! dimension its objects are placed in, declares what is selected, and places
//! every object it declares. What differs is clause 2 and only clause 2 — a
//! bounded region of three ordered dimensions rather than one — and with it
//! every query built on [`Extent::covers`] and [`Extent::overlaps`].
//!
//! It is not built for the reason `ladder.rs` gives for the third thing a rung
//! does not carry: the guess would be frozen. There is no corpus of declared
//! viewports, a three-dimensional extent invented now would have to be satisfied
//! by a compositor that does not exist, and a wrong guess in this crate is worse
//! than an absent one because everything downstream would have to honour it. The
//! camera is worse still, and it is worth naming the difficulty rather than
//! leaving it to be discovered: **a camera is a projection**, and part III's
//! whole claim is that the system owns projection. A declared camera is either a
//! coordinate — which section 11 forbids in the same sentence that forbids
//! `x = 340` — or a semantic statement, *looking at this object, from its
//! front*, which is the right shape and whose vocabulary nobody has. A viewport
//! also linearises differently: the order a screen reader reads a timeline in is
//! the order its author declared it, while a scene's is a graph of *in front of*
//! and *behind*, which the vocabulary would express with
//! [`crate::node::Relation`]s and not with a sequence.
//!
//! A trait over *any* canvas kind was refused, and the reason is worth keeping.
//! Such a trait must name the type of *where an occupant sits*, and that type is
//! exactly what differs between the two cases — so it is either an associated
//! type, which makes the dimension open and hands every projection an extent it
//! may not understand, or it is erased behind a box, which this crate has no
//! allocator for. An open dimension is the open vocabulary again, arriving
//! through the type system. When there are two kinds, the shared part will be
//! the floor, and the floor is four sentences rather than a trait.
//!
//! # What this module is not
//!
//! It is not a transport, and its capacities say so. [`PLACED_MAX`] is 64, which
//! is a screen of a timeline and not a timeline: a two-hour sequence has
//! thousands of clips, and an application with more occupants than fit must
//! declare the window it is showing. That windowing is a real decision this
//! module deliberately does not take, because it belongs with the delta protocol
//! that does not exist yet — and it is not a canvas problem in any case, since a
//! list of ten thousand rows asks the vocabulary the same question. RFC 0078
//! records it as the first thing a corpus will press on.

use crate::node::{
    CapRef, Content, Defect, Node, NodeId, Quantity, Role, StateSet, Text, Unit, check, children,
    find,
};

/// The most occupants one canvas may place.
///
/// Sixty-four is what a person can look at, not what a sequence can contain.
/// The bound is honest about what this module is: a declaration structure in a
/// crate with no allocator and no wire, sized so that an [`Arrangement`] stays a
/// couple of kilobytes and can live on a stack. An application that needs more
/// is declaring a window of its content rather than all of it, and *which*
/// window is a question for the transport. See the module's last section.
pub const PLACED_MAX: usize = 64;

/// How many decimal places of a second a description carries.
///
/// Three: a millisecond. Below it no description is actionable — a screen reader
/// saying *four point two zero zero zero zero one seconds* has said less than
/// one saying *4.2* — and above it a projection would be reading out digits that
/// exist only because the base does not divide. The *declaration* keeps every
/// tick either way; this is the scale of what is said about it, and nothing
/// computes from it.
pub const SPOKEN_DECIMALS: u8 = 3;

/// The finest scale a conversion will accept, in decimal places of a second.
///
/// Nine is a nanosecond, which is finer than one tick of any base anybody has
/// used, and it is where the arithmetic in [`TimeBase::ticks_from_seconds`]
/// stops being provably free of overflow in a 64-bit integer. Refusing past it
/// is cheaper than a saturating answer somebody would have to notice.
pub const DECIMALS_MAX: u8 = 9;

/// Why this canvas is not a participant.
///
/// One enum for three kinds of failure — a declaration that is malformed, a
/// declaration that is incomplete, and a question this canvas has no standing
/// to answer — because section 13 reads the first two the same way: a canvas
/// that cannot be asked what is in it is an escape, and *how* it got that way is
/// an implementation detail of the mistake. Every variant names the node it is
/// about wherever there is one, so that the answer to *which* does not need a
/// second pass.
///
/// The third kind is four variants and not one, which is worth counting here
/// because the argument for folding them in rests on the count.
/// [`NotOfThisCanvas`](Self::NotOfThisCanvas) and [`NotALane`](Self::NotALane)
/// are asked of a canvas about somebody else's node; [`NoRoom`](Self::NoRoom) is
/// about the buffer a caller offered; [`Unsayable`](Self::Unsayable) is about a
/// conversion a description asked for. None of them is a property of any
/// declaration, and they live here rather than in a second error type for the
/// reason this crate gives for everything it declines to say twice: a caller
/// already handles `Escape`, and a second error type is a second mechanism where
/// one was enough. `NotALane` is both kinds at once — it refuses a selection
/// that names a foreign lane and a query that asks about one — which is the
/// clearest argument there is against splitting the enum along that line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Escape {
    /// A base of zero ticks a second. A dimension with no scale is not a
    /// dimension, and every number placed in it would mean nothing.
    NotATimeBase,
    /// An extent whose end does not follow its start. A point in the dimension
    /// is a [`Role::Marker`], which the vocabulary already has a word for.
    NotAnExtent,
    /// More placements than [`PLACED_MAX`].
    TooManyPlacements,
    /// One occupant placed twice. Two answers to *when is this* is no answer,
    /// and it is refused where it is written rather than where it is read.
    PlacedTwice(NodeId),
    /// The declaration names a node that is not a [`Role::Canvas`] in this tree.
    NotACanvas(NodeId),
    /// A lane that is not a [`Role::Track`] of this canvas — confined to by a
    /// selection, or asked a question about. Another canvas's track is refused
    /// by this too: it is a lane, and it is not one this canvas may answer for.
    NotALane(NodeId),
    /// A placement names something that is not a clip or a marker of this
    /// canvas.
    NotAnOccupant(NodeId),
    /// A clip placed at an instant, or a marker placed over a span. A clip with
    /// no duration and a marker with one are both somebody meaning the other.
    Mistimed(NodeId),
    /// **The floor.** A clip or a marker this canvas declares that the
    /// arrangement places nowhere: the node exists, the pixels exist, and where
    /// it is does not. This is the escape [`crate::node::check`] cannot see.
    Unplaced(NodeId),
    /// **The floor's other half.** A canvas that places nothing at all, which
    /// meets the clause above by having nothing to meet it with. It is the
    /// cheapest escape on offer — one empty lane under a canvas that draws the
    /// whole sequence itself — and refusing it costs a genuinely empty canvas
    /// too. See the module's *Why clause 4 has a second half*.
    NothingPlaced(NodeId),
    /// **The floor's scope.** A node under this canvas whose role the dimension
    /// cannot hold: anything that is not a lane, a clip or a marker. The
    /// vocabulary lets a [`Role::List`] or a [`Role::Group`] sit under any
    /// parent, so without this a canvas places one token clip and hangs the
    /// content it actually draws off itself as a list, which clause 4 never
    /// looks at. See the module's *Why clause 4 is also about what is not a
    /// clip*.
    Unplaceable(NodeId),
    /// A question about a node this canvas does not declare. Not a defect in
    /// anything: an answer this canvas has no standing to give, said out loud
    /// rather than returned as an absence a caller would read as *no*.
    NotOfThisCanvas(NodeId),
    /// A linearisation was offered less room than the content needs. A
    /// description that stops halfway says the content ends where the buffer
    /// did, so it is refused rather than truncated — [`Participating::phrases`]
    /// says how much room is enough.
    NoRoom,
    /// A time this base can hold but a description cannot express at
    /// [`SPOKEN_DECIMALS`].
    ///
    /// Unreachable at [`TimeBase::FLICKS`] for any tick count an `i64` holds,
    /// and that is a property of the base rather than of this module: at
    /// 705 600 000 ticks a second the whole-seconds term tops out near
    /// 1.3 × 10¹³, nowhere near overflowing. It is reachable at a coarse base
    /// the *application* declares — at one tick a second it takes some
    /// 9.2 × 10¹⁵ seconds, about 292 million years. The variant stays because
    /// clause 2 hands the base to the application, so the bound is theirs to
    /// exceed and not ours to assume; and the time is named rather than silently
    /// dropped because a phrase with its time missing is the opaque rectangle in
    /// miniature.
    Unsayable(NodeId),
    /// The tree does not pass the vocabulary's own check, so the question does
    /// not arise. The two floors compose: this module asks *where is everything*
    /// of a tree `node.rs` has already agreed is a tree, and a canvas whose
    /// identities are ambiguous cannot be asked anything meaningful.
    Vocabulary(Defect),
}

/// A count of ticks in a canvas's declared [`TimeBase`]. Signed, on purpose.
///
/// This is the unit a declaration is written in, and it is deliberately not
/// seconds: seconds with a scale would make every application that wanted a
/// sample boundary pick its own scale anyway, and then two canvases in one tree
/// would disagree about what a number meant. The base says what a tick is worth,
/// once, for everything placed in it.
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
    #[must_use]
    pub const fn count(self) -> i64 {
        self.0
    }
}

/// What one tick is worth: the scale of a canvas's dimension.
///
/// The field is private and the only constructor refuses zero, so a base that
/// names nothing cannot be built. See the module documentation for why this is
/// declared per canvas rather than fixed here, and why milliseconds are the
/// wrong answer for the one case this module exists for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TimeBase {
    /// How many ticks make one second.
    ticks_per_second: u32,
}

impl TimeBase {
    /// 705 600 000 ticks a second: the base that names a frame and a sample
    /// exactly.
    ///
    /// Every common frame rate and every common audio rate divides it — see
    /// [`names`](Self::names), which is that property asked as a question, and
    /// the module documentation for the list. At this base an `i64` of ticks
    /// covers some four centuries either side of the origin, which is more
    /// sequence than anybody has.
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
    #[must_use]
    pub const fn ticks_per_second(self) -> u32 {
        self.ticks_per_second
    }

    /// Does this base name every boundary of something happening
    /// `numerator / denominator` times a second, exactly?
    ///
    /// The question an application should ask of a base before declaring it: a
    /// base that does not divide the content's own rate will put a boundary
    /// between two samples, and nothing downstream can tell that it happened.
    ///
    /// # Why the rate is a fraction and not a number
    ///
    /// Because the rate that decides whether this predicate is worth having is
    /// not an integer. NTSC video runs at 30000/1001 frames a second, which is
    /// the headline case for [`FLICKS`](Self::FLICKS): 705 600 000 × 1001 is
    /// 30000 × 23 543 520, so one frame is a whole number of ticks. Asked as a
    /// single integer that question cannot be put at all — a caller rounds to
    /// 29 970, 705 600 000 does not divide by it, and the predicate answers
    /// *false* about a rate this base names exactly. The wrong answer costs in
    /// the expensive direction, telling an application to invent a base it does
    /// not need; and RFC 0078 arms one of its reversal conditions on this very
    /// predicate, so a predicate that misfires here misfires a reversal.
    ///
    /// A rate with a zero on either side of the line is not a rate, and answers
    /// `false` rather than dividing by zero — and rather than being folded into
    /// the divisibility question, where it would come out right by accident.
    #[must_use]
    pub const fn names(self, numerator: u32, denominator: u32) -> bool {
        if numerator == 0 || denominator == 0 {
            return false;
        }
        // One boundary every `denominator / numerator` seconds is
        // `ticks_per_second * denominator / numerator` ticks. Both factors are a
        // `u32`, so the product is inside a `u64` by construction.
        let ticks_per_boundary = self.ticks_per_second as u64 * denominator as u64;
        ticks_per_boundary.is_multiple_of(numerator as u64)
    }

    /// The tick at `scaled` divided by ten to the power of `decimals` seconds,
    /// truncated toward zero.
    ///
    /// This is how a caller who speaks seconds — an agent, a test, a script —
    /// reaches a declaration written in ticks, and it is the one place *4.2 s*
    /// becomes a number in this tree: `(42, 1)`, or `(4_200, 3)`, and never a
    /// floating-point literal.
    ///
    /// Returns `None` past [`DECIMALS_MAX`], or when the product does not fit an
    /// `i64` — a time this base cannot hold, which is an answer and not a
    /// rounding.
    #[must_use]
    pub const fn ticks_from_seconds(self, scaled: i64, decimals: u8) -> Option<Ticks> {
        let unit = match pow10(decimals) {
            Some(unit) => unit,
            None => return None,
        };
        let per_second = self.ticks_per_second as i64;
        let from_whole = match (scaled / unit).checked_mul(per_second) {
            Some(ticks) => ticks,
            None => return None,
        };
        // The remainder is smaller than `unit`, which is at most ten to the
        // ninth, and `per_second` is at most a `u32` — so this product is inside
        // an `i64` by construction and the check is belt and braces.
        let from_rest = match (scaled % unit).checked_mul(per_second) {
            Some(ticks) => ticks / unit,
            None => return None,
        };
        match from_whole.checked_add(from_rest) {
            Some(ticks) => Some(Ticks(ticks)),
            None => None,
        }
    }

    /// The same instant in seconds, carrying `decimals` decimal places,
    /// truncated toward zero.
    ///
    /// The result is a [`Quantity`] in [`Unit::Seconds`], which is what the
    /// vocabulary already speaks — so a projection that can say a volume can say
    /// a time, with no second mechanism and no floating-point type.
    ///
    /// Returns `None` past [`DECIMALS_MAX`] or on overflow, for the reason
    /// above. Overflow needs a coarse base and an enormous tick count — see
    /// [`Escape::Unsayable`], which is what a linearisation turns it into.
    #[must_use]
    pub const fn seconds_from_ticks(self, ticks: Ticks, decimals: u8) -> Option<Quantity> {
        let unit = match pow10(decimals) {
            Some(unit) => unit,
            None => return None,
        };
        let per_second = self.ticks_per_second as i64;
        let whole = match (ticks.0 / per_second).checked_mul(unit) {
            Some(scaled) => scaled,
            None => return None,
        };
        // As above: the remainder is smaller than the base, the base is a `u32`,
        // and `unit` is at most ten to the ninth.
        let rest = match (ticks.0 % per_second).checked_mul(unit) {
            Some(scaled) => scaled / per_second,
            None => return None,
        };
        match whole.checked_add(rest) {
            Some(scaled) => Some(Quantity::scaled(scaled, decimals, Unit::Seconds)),
            None => None,
        }
    }
}

/// Ten to the power of `decimals`, or `None` past [`DECIMALS_MAX`].
///
/// A free function rather than a method because it is arithmetic about decimal
/// scales and not about time, and because the bound it enforces is what makes
/// both conversions above provably free of overflow in an `i64`.
const fn pow10(decimals: u8) -> Option<i64> {
    if decimals > DECIMALS_MAX {
        return None;
    }
    let mut value: i64 = 1;
    let mut left = decimals;
    while left > 0 {
        value *= 10;
        left -= 1;
    }
    Some(value)
}

/// A stretch of the dimension: from a tick, up to but not including another.
///
/// # Why the interval is half-open
///
/// Because boundaries are where timelines break. A clip ending at 4.2 s and one
/// starting at 4.2 s do not overlap, and the sample at exactly 4.2 s belongs to
/// the second — which is the only convention under which butt-joined clips are
/// neither double-counted nor separated by a gap of one tick that nobody
/// declared. All four predicates below read that way, and a reader who changes
/// one of them has to change the rest.
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
    /// occupant with no duration is a [`Role::Marker`] — the vocabulary has the
    /// word, and a zero-length clip is somebody who did not reach for it.
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

    /// How long it is, in ticks.
    #[must_use]
    pub const fn duration(self) -> Ticks {
        Ticks(self.end.0.saturating_sub(self.start.0))
    }

    /// Is `instant` inside it? Half-open: the end is not.
    #[must_use]
    pub const fn contains(self, instant: Ticks) -> bool {
        self.start.0 <= instant.0 && instant.0 < self.end.0
    }

    /// Does it contain the whole of `other`?
    #[must_use]
    pub const fn covers(self, other: Self) -> bool {
        self.start.0 <= other.start.0 && other.end.0 <= self.end.0
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
/// not the same shape: a [`Role::Clip`] fills a stretch and a [`Role::Marker`]
/// names an instant. [`Arrangement::admit`] refuses each placed as the other,
/// which is [`Escape::Mistimed`] — not pedantry, since *what is at 5 s* has a
/// different answer depending on which the author meant.
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
    /// Both kinds answer, which is why it is a method here rather than a
    /// `match` written out at each call site: an instant inside a stretch is
    /// inside it. The first version of this module had a `let … else` at one
    /// call site that answered *no* for every marker without saying so, and a
    /// predicate written once cannot disagree with itself in one place.
    #[must_use]
    pub const fn touches(self, extent: Extent) -> bool {
        match self {
            Self::At(instant) => extent.contains(instant),
            Self::Between(own) => own.overlaps(extent),
        }
    }
}

/// One occupant, and where it sits.
///
/// # Why this does not say which lane
///
/// Because the tree already does. A clip's lane is its [`Node`]'s parent, and
/// restating it here would be a second copy of a fact that can disagree with the
/// first — which is the failure mode RFC 0077 refuses when it refuses a time
/// range on `Node`, running in the other direction. The join is [`NodeId`] and
/// it goes both ways: the arrangement says *when*, the tree says *what* and
/// *where among the lanes*, and neither restates the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Placement {
    /// Which node. It must be a clip or a marker of this canvas.
    pub occupant: NodeId,
    /// Where it sits in the dimension.
    pub span: Span,
}

impl Placement {
    /// A clip, filling `extent`.
    #[must_use]
    pub const fn over(occupant: NodeId, extent: Extent) -> Self {
        Self { occupant, span: Span::Between(extent) }
    }

    /// A marker, naming `instant`.
    #[must_use]
    pub const fn at(occupant: NodeId, instant: Ticks) -> Self {
        Self { occupant, span: Span::At(instant) }
    }
}

/// What is selected: a stretch of the dimension, optionally confined to one
/// lane.
///
/// See the module documentation for why this is not a set of node identities.
/// The short version is that a set of identities cannot express a selection over
/// a stretch with nothing in it, and that is an operation every timeline has.
///
/// One stretch, not several. A multi-range selection — a ripple delete across
/// three lanes and two stretches — is real, and is refused for version 1 because
/// a bounded array of stretches is a capacity decision with no corpus to size it
/// against while the exit needs exactly one. RFC 0078 carries the reversal
/// condition: the first application whose selection cannot be declared.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    /// The stretch, if anything is selected.
    span: Option<Extent>,
    /// The lane it is confined to, if it is confined to one.
    lane: Option<NodeId>,
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
    pub const fn on(self, lane: NodeId) -> Self {
        Self { span: self.span, lane: Some(lane) }
    }

    /// The selected stretch, if there is one.
    #[must_use]
    pub const fn span(self) -> Option<Extent> {
        self.span
    }

    /// The lane it is confined to, if it is.
    #[must_use]
    pub const fn lane(self) -> Option<NodeId> {
        self.lane
    }

    /// Is nothing selected?
    #[must_use]
    pub const fn is_nothing(self) -> bool {
        self.span.is_none()
    }
}

/// The answer to *which one is here*, which has three cases and not two.
///
/// # Why this is not an `Option`
///
/// Because an `Option` collapses the two answers that matter most into one word.
/// *Nothing is at 6.4 s on this lane* and *two things are, and I will not guess
/// which you meant* are opposite facts about a timeline, and an agent acting on
/// them alike deletes the wrong clip in one of the two cases. The first version
/// of this module returned `None` for both, and the test named after the
/// ambiguity then passed identically when the second clip was simply absent —
/// an assertion that could not fail for the reason it claimed.
///
/// [`Several`](Self::Several) is not a defect. Clips on one lane overlap because
/// a crossfade is exactly that, and the honest next question is
/// [`Participating::occupants_over`], which enumerates them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sole {
    /// Nothing of that lane is there.
    Nothing,
    /// Exactly one, and this is it.
    One(NodeId),
    /// Two or more. Ask [`Participating::occupants_over`] for all of them.
    Several,
}

impl Sole {
    /// The identity, for a caller that genuinely wants both refusals collapsed.
    ///
    /// Offered so that collapsing is something a call site *does*, visibly,
    /// rather than something the return type did for it silently.
    #[must_use]
    pub const fn one(self) -> Option<NodeId> {
        match self {
            Self::One(id) => Some(id),
            Self::Nothing | Self::Several => None,
        }
    }
}

/// What an agent may invoke on a node this canvas declares.
///
/// The *scriptable* half of the exit, and a type rather than an `Option<CapRef>`
/// because there are three answers and a projection has to tell them apart. A
/// node with a capability the application is currently declining to run is not a
/// node with no capability — the first will work later and the second never
/// will, and an agent treating them alike either gives up too early or retries
/// forever.
///
/// [`crate::node::StateSet::ENABLED`] is what separates the first two, and that
/// reading is `node.rs`'s own: a node carrying an intent and no state is *not
/// operable*, because the safe default in the only direction that matters is
/// that nothing may be invoked until the application says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Operable {
    /// It declares a capability and declares itself enabled: this is what to
    /// invoke.
    Invocable(CapRef),
    /// It declares a capability and does not currently declare itself enabled.
    /// Not now, which is not the same as not ever.
    Disabled(CapRef),
    /// It declares no capability at all. Addressable, describable, and not
    /// scriptable — which is how `node.rs` spells read-only, and is a
    /// legitimate thing for a clip on a locked lane to be.
    Inert,
}

impl Operable {
    /// What a node's own declaration says about operating it.
    ///
    /// **Not public, and the privacy is the scoping.** This is a two-field read
    /// of a node that knows nothing about any canvas, so a public version of it
    /// would be this module answering *what may be invoked on this* for a node
    /// no canvas admitted — which is the answer [`Participating::operable`]
    /// exists to refuse, offered under another name thirty lines above the
    /// refusal. It was public once, and the refusal was documented over it.
    ///
    /// It is `pub(crate)` rather than private because
    /// [`crate::reader`](crate::reader) asks it of nodes of a **whole tree** it
    /// has itself been admitted against, which is a wider scope than a canvas
    /// has and a narrower one than none. That widening is deliberate and it is
    /// bounded by the same rule: every caller below holds a value that proves
    /// something was checked, and there is no route from outside this crate to
    /// a bare [`Node`]'s answer.
    ///
    /// Its three callers are [`Participating::operable`], which checks scope
    /// before it reads anything; `Participating::say`, which runs over nodes the
    /// canvas has already been admitted against; and `reader::Reader::say`,
    /// which runs over the tree its [`Reader`](crate::reader::Reader) was
    /// admitted against.
    pub(crate) const fn of(node: &Node) -> Self {
        match node.intent {
            None => Self::Inert,
            Some(intent) => {
                if node.state.holds(StateSet::ENABLED) {
                    Self::Invocable(intent)
                } else {
                    Self::Disabled(intent)
                }
            }
        }
    }

    /// The capability to invoke, if there is one to invoke now.
    #[must_use]
    pub const fn invocable(self) -> Option<CapRef> {
        match self {
            Self::Invocable(intent) => Some(intent),
            Self::Disabled(_) | Self::Inert => None,
        }
    }
}

/// What a canvas declares about its content: the three clauses of the floor a
/// declaration can state on its own, and the placements the fourth is checked
/// against.
///
/// This type is a *claim*, not an admission. It says which canvas, in what
/// dimension, with what selected, and where each occupant it knows about sits —
/// and none of that has been checked against a tree yet. [`admit`](Self::admit)
/// is where it meets one, and [`Participating`] is what comes back.
///
/// It is a couple of kilobytes and it is `Copy`, for the reason [`Node`] is: it
/// is optimised for being read and argued with rather than for being sent. The
/// builders consume and return it, so a declaration is a value and not a
/// mutation, and the wire form — when there is a wire — belongs in `f_abi`
/// carrying placements by reference into an arena, exactly as `node.rs` says of
/// its own text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Arrangement {
    /// Clause 1: the canvas this is about.
    canvas: NodeId,
    /// Clause 2: what one tick of the dimension is worth.
    base: TimeBase,
    /// Clause 3: what is selected in it.
    selection: Selection,
    /// Clause 4's raw material: where each occupant sits. Slots past `len` are
    /// never read.
    placed: [Option<Placement>; PLACED_MAX],
    /// How many of them are real.
    len: u8,
}

impl Arrangement {
    /// Declare a canvas: which one, in what dimension, with what selected.
    ///
    /// There is no constructor that omits any of the three, and no `Default`.
    /// That is the floor being structural rather than advisory: a canvas whose
    /// dimension has no scale, or whose selection was never stated, is not an
    /// arrangement that can be fixed later — it is the opaque rectangle, and the
    /// type refuses to be that.
    ///
    /// Clause 4 cannot be stated here, because it is a statement about this
    /// declaration *and* a tree. [`admit`](Self::admit) is where it is made,
    /// including the half this constructor could have checked and deliberately
    /// does not: that something is placed at all. Refusing it here would make
    /// the builders unusable, since a declaration is assembled one placement at
    /// a time and begins empty.
    #[must_use]
    pub const fn declaring(canvas: NodeId, base: TimeBase, selection: Selection) -> Self {
        Self { canvas, base, selection, placed: [None; PLACED_MAX], len: 0 }
    }

    /// The same declaration, with one more occupant placed.
    ///
    /// # Errors
    ///
    /// [`Escape::TooManyPlacements`] past [`PLACED_MAX`], and
    /// [`Escape::PlacedTwice`] for an occupant this arrangement already places —
    /// refused here, where the author is present, rather than at admission,
    /// where they are not.
    pub const fn with_placement(mut self, placement: Placement) -> Result<Self, Escape> {
        if self.len as usize >= PLACED_MAX {
            return Err(Escape::TooManyPlacements);
        }
        let mut at = 0;
        while at < self.len as usize {
            if let Some(seen) = self.placed[at]
                && seen.occupant.value() == placement.occupant.value()
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
    /// A selection changes on every drag and the placements do not, so this is
    /// the redeclaration an application makes constantly. It is a builder rather
    /// than a setter because the type is a value: two selections are two
    /// declarations, and a projection that holds the old one still holds
    /// something true about the frame it came from.
    #[must_use]
    pub const fn with_selection(self, selection: Selection) -> Self {
        Self { canvas: self.canvas, base: self.base, selection, placed: self.placed, len: self.len }
    }

    /// The canvas this is about.
    #[must_use]
    pub const fn canvas(&self) -> NodeId {
        self.canvas
    }

    /// What one tick is worth here.
    #[must_use]
    pub const fn base(&self) -> TimeBase {
        self.base
    }

    /// What is selected.
    #[must_use]
    pub const fn selection(&self) -> Selection {
        self.selection
    }

    /// How many occupants it places.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Does it place none?
    ///
    /// Legal to build, and never admitted. [`admit`](Self::admit) refuses it
    /// with [`Escape::NothingPlaced`] once the tree is a tree and the named node
    /// is a canvas of it, and with whatever is wrong *before* that when
    /// something is — a tree that fails [`check`], a node that is not a canvas,
    /// a selection on a foreign lane. First cause, not first consequence, which
    /// is what `a_tree_the_vocabulary_refuses_is_not_a_participant` asserts and
    /// what two earlier versions of this sentence got wrong in both directions.
    /// A canvas that places nothing has declared nothing; see the module's *Why
    /// clause 4 has a second half*.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every placement, in declaration order.
    pub fn placements(&self) -> impl Iterator<Item = Placement> + '_ {
        self.placed[..self.len as usize].iter().copied().flatten()
    }

    /// Where this arrangement puts `occupant`, if it puts it anywhere.
    #[must_use]
    pub fn placement_of(&self, occupant: NodeId) -> Option<Placement> {
        self.placements().find(|placement| placement.occupant == occupant)
    }

    /// Meet a tree, and be admitted as a participant or refused as an escape.
    ///
    /// This is where the floor is enforced and where the only [`Participating`]
    /// in existence comes from. It checks, in order: that the tree is a tree at
    /// all; that the named canvas is one; that a confined selection names a lane
    /// of it; that every placement names an occupant of it, placed as its role
    /// permits; that nothing under the canvas is of a role its dimension cannot
    /// hold; and finally the clause that matters — that **something is placed,
    /// and every clip and every marker the tree declares under this canvas is
    /// placed**.
    ///
    /// # Why it checks the vocabulary first
    ///
    /// Two reasons, and the second is why it is not merely tidy.
    /// [`crate::node::check`] is what makes *belongs to this canvas* decidable:
    /// it bounds the parent walk, so a tree whose parents form a cycle is
    /// refused before anything here walks one. And an arrangement over a tree
    /// with two nodes of one identity would answer questions with whichever it
    /// found first, which is worse than refusing.
    ///
    /// # Errors
    ///
    /// The first [`Escape`] found, for the reason `check` returns the first
    /// [`Defect`]: a declaration with two faults is one somebody is still
    /// writing.
    ///
    /// # Why this is quadratic, and why that is right here
    ///
    /// It resolves identities by scanning, because this crate has no hash set —
    /// RFC 0004 — and because a projection is not meant to call it every frame
    /// any more than it calls `check` every frame. This is for authors, for
    /// tests, and for the moment a canvas is first declared; a projection
    /// validates incrementally at the delta that moved a clip.
    pub fn admit<'a>(&'a self, nodes: &'a [Node]) -> Result<Participating<'a>, Escape> {
        if let Err(defect) = check(nodes) {
            return Err(Escape::Vocabulary(defect));
        }

        match find(nodes, self.canvas) {
            Some(node) if node.role == Role::Canvas => {}
            _ => return Err(Escape::NotACanvas(self.canvas)),
        }

        if let Some(lane) = self.selection.lane
            && !is_lane_of(nodes, lane, self.canvas)
        {
            return Err(Escape::NotALane(lane));
        }

        for placement in self.placements() {
            let Some(node) = find(nodes, placement.occupant) else {
                return Err(Escape::NotAnOccupant(placement.occupant));
            };
            if !occupies(nodes, node, self.canvas) {
                return Err(Escape::NotAnOccupant(node.id));
            }
            let agrees = matches!(
                (node.role, placement.span),
                (Role::Clip, Span::Between(_)) | (Role::Marker, Span::At(_))
            );
            if !agrees {
                return Err(Escape::Mistimed(node.id));
            }
        }

        // The floor's scope, and the reason it is checked before the two halves
        // below rather than after them: those two are about *the clips and
        // markers* under this canvas, and this is what stops that being a set an
        // author chooses. `Role::List`, `Role::Group` and the rest accept any
        // parent the vocabulary allows, so the content a canvas actually draws
        // could hang off it as a list — placed nowhere, asked nothing, and
        // invisible to a clause that only counts clips — while one token clip
        // satisfied everything below. That is RFC 0078's own argument about
        // vacuous compliance, one level down from where it was made.
        placeable_below(nodes, self.canvas)?;

        // The floor, first half. A declaration that places nothing complies with
        // the loop below by giving it nothing to object to, and that is the
        // cheapest escape available: one empty lane under a canvas drawing the
        // whole sequence itself. It is refused before the loop rather than
        // inside it, because there is no one node to name — the canvas is the
        // thing that said nothing.
        if self.len == 0 {
            return Err(Escape::NothingPlaced(self.canvas));
        }

        // The floor, second half. Everything above refuses a declaration that
        // says something wrong; this refuses one that does not say enough, which
        // is the failure section 13 is actually about.
        for node in nodes {
            if occupies(nodes, node, self.canvas) && self.placement_of(node.id).is_none() {
                return Err(Escape::Unplaced(node.id));
            }
        }

        Ok(Participating { nodes, arrangement: self })
    }
}

/// Is everything under `parent` something the dimension can hold — or which
/// node is the first that is not?
///
/// A lane, a clip and a marker are the three roles a canvas's dimension has a
/// place for, and this walks the whole subtree rather than the canvas's own
/// children because the smuggling works at any depth: a list under a clip is the
/// same escape one level further down. The recursion terminates because
/// [`check`] has already refused a parent chain that does not reach a root.
///
/// # Errors
///
/// [`Escape::Unplaceable`], naming the first such node in declaration order.
fn placeable_below(nodes: &[Node], parent: NodeId) -> Result<(), Escape> {
    for child in children(nodes, parent) {
        match child.role {
            Role::Track | Role::Clip | Role::Marker => {}
            _ => return Err(Escape::Unplaceable(child.id)),
        }
        placeable_below(nodes, child.id)?;
    }
    Ok(())
}

/// Is `lane` a track of `canvas` in this tree?
fn is_lane_of(nodes: &[Node], lane: NodeId, canvas: NodeId) -> bool {
    matches!(find(nodes, lane), Some(node) if node.role == Role::Track && node.parent == canvas)
}

/// Is `node` a clip or a marker that belongs to `canvas`?
///
/// The vocabulary's placement rules make this two steps and never more: a clip
/// sits on a track and a track sits on a canvas, and a marker sits on either. So
/// this is the whole of *belongs to*, and it is written here rather than in
/// `node.rs` because it is a question about one canvas rather than about the
/// tree.
fn occupies(nodes: &[Node], node: &Node, canvas: NodeId) -> bool {
    if node.role != Role::Clip && node.role != Role::Marker {
        return false;
    }
    if node.parent == canvas {
        return true;
    }
    let Some(lane) = find(nodes, node.parent) else { return false };
    lane.role == Role::Track && lane.parent == canvas
}

/// A canvas that has met the floor, and the only thing in this module that can
/// be asked a question.
///
/// Holding one of these is a proof rather than a convenience: it cannot be
/// built except by [`Arrangement::admit`], so every query below is asking
/// something of a canvas that has declared its content. That is the whole
/// mechanism by which *a canvas may keep its pixels and may not keep its
/// content* is a property of the types rather than a sentence in a style guide.
#[derive(Clone, Copy, Debug)]
pub struct Participating<'a> {
    /// The declared tree, checked.
    nodes: &'a [Node],
    /// The arrangement over it, admitted.
    arrangement: &'a Arrangement,
}

impl<'a> Participating<'a> {
    /// The canvas this is about.
    #[must_use]
    pub const fn canvas(&self) -> NodeId {
        self.arrangement.canvas
    }

    /// The admitted declaration.
    #[must_use]
    pub const fn arrangement(&self) -> &'a Arrangement {
        self.arrangement
    }

    /// The tree it was admitted against.
    #[must_use]
    pub const fn nodes(&self) -> &'a [Node] {
        self.nodes
    }

    /// The handle to the pixels this canvas draws itself, if it declared one.
    ///
    /// The other half of section 13's sentence, and the half a declaration is
    /// most likely to have and least likely to be asked for: an application that
    /// renders a timeline holds that rendering somewhere, and
    /// [`crate::node::Content::Media`] is the vocabulary's word for *it is mine,
    /// here is the capability that names it*.
    ///
    /// Nothing in this module resolves it, decodes it, or derives an answer from
    /// it, and that independence is a property worth testing rather than an
    /// omission to apologise for: every query here returns exactly what it would
    /// return if this were `None`, so a consumer that cannot render is not a
    /// consumer that gets less. Where the two halves do meet is
    /// [`Escape::NothingPlaced`] — a canvas may not declare a rendering and
    /// place nothing under it.
    #[must_use]
    pub fn rendering(&self) -> Option<CapRef> {
        match find(self.nodes, self.arrangement.canvas)?.content {
            Content::Media(handle) => Some(handle),
            Content::None | Content::Text(_) | Content::Value(_) => None,
        }
    }

    /// Which lane an occupant of this canvas is on, or `Ok(None)` for one that
    /// sits on the canvas itself.
    ///
    /// Answered from the tree, because the tree is where it is said. This is the
    /// join working in the direction the arrangement deliberately does not
    /// carry.
    ///
    /// # Errors
    ///
    /// [`Escape::NotOfThisCanvas`] for a node this canvas does not declare. The
    /// two answers that separates are *it sits on the canvas itself* and *it is
    /// not mine to say about* — this canvas's marker and another canvas's clip —
    /// and while this returned an `Option` it spelled both of them `None`, which
    /// is the collapse [`Sole`] exists to refuse, happening one method over.
    pub fn lane_of(&self, occupant: NodeId) -> Result<Option<NodeId>, Escape> {
        if !self.declares(occupant) {
            return Err(Escape::NotOfThisCanvas(occupant));
        }
        Ok(self.lane_of_declared(occupant))
    }

    /// The same question, asked of a node this canvas is already known to
    /// declare.
    ///
    /// Every caller of this is walking placements, and a placement is in the
    /// arrangement only because [`Arrangement::admit`] found its occupant under
    /// this canvas — so the scope check above would be asking again what
    /// admission already answered, once per placement.
    fn lane_of_declared(&self, occupant: NodeId) -> Option<NodeId> {
        let node = find(self.nodes, occupant)?;
        let parent = find(self.nodes, node.parent)?;
        (parent.role == Role::Track).then_some(parent.id)
    }

    /// Does this canvas declare `node` — is it the canvas, one of its lanes, or
    /// one of its occupants?
    ///
    /// The scope of every question here, and what each of them is refused by. A
    /// canvas answering about a node from elsewhere in the tree would be
    /// answering for a surface it has no standing over, and that answer would
    /// look exactly like a real one.
    ///
    /// [`operable`](Self::operable) and [`lane_of`](Self::lane_of) ask this of
    /// the node they are about. The three queries over a lane ask the narrower
    /// half of it — that the lane is a [`Role::Track`] of this canvas — because
    /// *what is on this lane* is not a question a marker or a clip can answer
    /// either, and the word for the refusal is [`Escape::NotALane`]. Once, only
    /// `operable` asked anything: a lane query given a stranger, a command or
    /// another canvas's track answered [`Sole::Nothing`], which is the one
    /// answer [`Sole`] exists to keep apart from a refusal.
    #[must_use]
    pub fn declares(&self, node: NodeId) -> bool {
        let canvas = self.arrangement.canvas;
        if node == canvas || is_lane_of(self.nodes, node, canvas) {
            return true;
        }
        matches!(find(self.nodes, node), Some(found) if occupies(self.nodes, found, canvas))
    }

    /// What an agent may invoke on a node of this canvas.
    ///
    /// **The scriptable half of the exit.** Addressability got the agent an
    /// identity; this is what it does with one. The answer comes from the node's
    /// own declaration — the capability it carries and whether the application
    /// currently declares it operable — so an agent that located a clip by time
    /// and lane can act on it without a coordinate, a pixel or an index, and a
    /// projection can tell the three cases apart. See [`Operable`].
    ///
    /// # Errors
    ///
    /// [`Escape::NotOfThisCanvas`] for anything this canvas does not declare.
    /// This is the only place in the module that reads a node's declaration and
    /// answers what may be invoked on it — the read itself is private — so there
    /// is no unscoped version of this answer to reach for instead. The claim is
    /// that narrow on purpose, because two things next to it are still true and
    /// should not be: a caller can write [`Operable::Invocable`] for itself out
    /// of a capability it already holds, and can read [`Node::intent`], which
    /// `node.rs` publishes. Both of those are a caller stating a fact about a
    /// node. Neither is this module turning a node into an answer for a surface
    /// that did not declare it, which is the answer an agent acts on.
    pub fn operable(&self, node: NodeId) -> Result<Operable, Escape> {
        if !self.declares(node) {
            return Err(Escape::NotOfThisCanvas(node));
        }
        match find(self.nodes, node) {
            Some(found) => Ok(Operable::of(found)),
            None => Err(Escape::NotOfThisCanvas(node)),
        }
    }

    /// Every occupant of `lane` that shares any tick with `span`, in declaration
    /// order.
    ///
    /// Markers are not among them. A marker names an instant rather than
    /// occupying it — a playhead over a clip has not made the clip two things —
    /// so *what is here* is a question about clips.
    ///
    /// That is deliberately not the rule [`selected`](Self::selected) follows,
    /// and the difference is the difference between the questions: *what
    /// occupies this stretch* is about what fills it, while *what does this
    /// selection touch* is about everything placed inside it, cues included,
    /// because a ripple delete over a stretch moves the cues in it too.
    ///
    /// # Errors
    ///
    /// [`Escape::NotALane`] for anything that is not a [`Role::Track`] of this
    /// canvas. See [`declares`](Self::declares).
    pub fn occupants_over(
        &self,
        lane: NodeId,
        span: Extent,
    ) -> Result<impl Iterator<Item = NodeId> + '_, Escape> {
        if !is_lane_of(self.nodes, lane, self.arrangement.canvas) {
            return Err(Escape::NotALane(lane));
        }
        let placed = self.arrangement.placements();
        let over = placed
            .filter(move |placement| match placement.span {
                Span::Between(extent) => {
                    extent.overlaps(span) && self.lane_of_declared(placement.occupant) == Some(lane)
                }
                Span::At(_) => false,
            })
            .map(|placement| placement.occupant);
        Ok(over)
    }

    /// The occupant of `lane` that contains the whole of `span` — the question
    /// section 13 asks.
    ///
    /// Three answers, not two: see [`Sole`]. [`Sole::Several`] is what a
    /// crossfade produces and is a different word from [`Sole::Nothing`],
    /// because an agent told *the clip* when there are two has been given a
    /// wrong answer confidently, and one told *nothing is there* when two things
    /// are has been given a different wrong answer. The honest next call is
    /// [`occupants_over`](Self::occupants_over).
    ///
    /// # Errors
    ///
    /// [`Escape::NotALane`] for anything that is not a [`Role::Track`] of this
    /// canvas — which is a third refusal on top of [`Sole`]'s two, and belongs
    /// outside it: *nothing is there* and *two things are* are answers about a
    /// lane, and this is the absence of one to answer about.
    pub fn occupant_between(&self, lane: NodeId, span: Extent) -> Result<Sole, Escape> {
        self.sole(lane, |extent| extent.covers(span))
    }

    /// The occupant of `lane` at `instant` — *what is under the playhead*.
    ///
    /// Three answers, on [`occupant_between`](Self::occupant_between)'s
    /// reasoning exactly.
    ///
    /// # Errors
    ///
    /// [`Escape::NotALane`], as above.
    pub fn occupant_at(&self, lane: NodeId, instant: Ticks) -> Result<Sole, Escape> {
        self.sole(lane, |extent| extent.contains(instant))
    }

    /// The one clip of `lane` whose extent satisfies `wanted`, or why there is
    /// not one.
    ///
    /// Both public queries are this with a different predicate, so the counting
    /// — and the refusal to guess once the count is two, and the refusal to
    /// answer at all about a lane that is not this canvas's — is written once
    /// and cannot drift between them.
    fn sole(&self, lane: NodeId, wanted: impl Fn(Extent) -> bool) -> Result<Sole, Escape> {
        if !is_lane_of(self.nodes, lane, self.arrangement.canvas) {
            return Err(Escape::NotALane(lane));
        }
        let mut only = Sole::Nothing;
        for placement in self.arrangement.placements() {
            let Span::Between(extent) = placement.span else { continue };
            if !wanted(extent) || self.lane_of_declared(placement.occupant) != Some(lane) {
                continue;
            }
            only = match only {
                Sole::Nothing => Sole::One(placement.occupant),
                Sole::One(_) | Sole::Several => return Ok(Sole::Several),
            };
        }
        Ok(only)
    }

    /// Which occupants the declared selection touches.
    ///
    /// The join between a selection over the dimension and the identities the
    /// vocabulary addresses: an agent selects a stretch, this says what is in
    /// it, and what it does next is invoke the capability one of those nodes
    /// carries — [`operable`](Self::operable) is that step. Empty when nothing
    /// is selected, and empty when the selected stretch is empty, which are
    /// different facts that [`Selection::is_nothing`] tells apart.
    ///
    /// Markers are included, unlike in
    /// [`occupants_over`](Self::occupants_over), because a marker is placed in
    /// the dimension and a selection is a stretch of it: a cue at 5 s is inside
    /// a selection of 4.2 s to 6.8 s by every reading an editor has. The first
    /// version of this dropped every marker silently, which also made
    /// [`Phrase::in_selection`] false for a playhead the selection plainly
    /// covered.
    pub fn selected(&self) -> impl Iterator<Item = NodeId> + '_ {
        self.arrangement
            .placements()
            .filter(move |placement| self.within_selection(*placement))
            .map(|placement| placement.occupant)
    }

    /// Does the declared selection touch this placement?
    fn within_selection(&self, placement: Placement) -> bool {
        let Some(span) = self.arrangement.selection.span else { return false };
        if !placement.span.touches(span) {
            return false;
        }
        match self.arrangement.selection.lane {
            Some(lane) => self.lane_of_declared(placement.occupant) == Some(lane),
            None => true,
        }
    }

    /// How many phrases a full description of this canvas takes.
    ///
    /// Call it to size the buffer [`linearise`](Self::linearise) is given. It is
    /// a walk rather than a stored count because the tree is the authority on
    /// what is in the canvas and this module refuses to keep a second copy of
    /// that.
    #[must_use]
    pub fn phrases(&self) -> usize {
        1 + self.below(self.arrangement.canvas)
    }

    /// How many nodes sit under `parent`, at any depth.
    fn below(&self, parent: NodeId) -> usize {
        children(self.nodes, parent).map(|child| 1 + self.below(child.id)).sum()
    }

    /// Say the whole canvas, in reading order, into `into`; answer how many
    /// phrases that took.
    ///
    /// This is the screen-reader half of section 13's test, and the claim it
    /// makes is narrow and checkable: **a consumer that never renders anything
    /// can name every occupant of a canvas, say when it is, say which lane it is
    /// on, and know what may be invoked on it**, from the declaration alone.
    ///
    /// # What reading order is, and what it is not
    ///
    /// It is the tree's declaration order, depth first: the canvas, then each
    /// lane, then what is on it. That is the order the **author** declared, and
    /// it is deliberately not the order the clock runs. Nothing in
    /// [`admit`](Arrangement::admit) requires a tree to declare its clips
    /// chronologically and nothing here sorts them, because a timeline read in
    /// time order interleaves every lane and leaves a screen reader announcing
    /// three lanes' worth of clips with no structure to hang them on.
    ///
    /// A consumer that wants chronological order can have it without this
    /// module's help, because every phrase carries its own [`Spoken`] time — and
    /// sorting needs room to put the result, which this crate does not have and
    /// a consumer does. What would be wrong is claiming an order this does not
    /// produce, so the claim is stated in the direction it is true and there is
    /// a test that declares its clips backwards to hold it there.
    ///
    /// It does not produce a sentence. Assembling words is a projection's job
    /// and a locale's, and a module that emitted English here would be a module
    /// every non-English projection had to work around. What it produces is what
    /// a sentence is assembled *from*.
    ///
    /// # Errors
    ///
    /// [`Escape::NoRoom`] when `into` is shorter than
    /// [`phrases`](Self::phrases) — a description that stops halfway says the
    /// content ends where the buffer did — and [`Escape::Unsayable`] for a time
    /// that will not fit [`SPOKEN_DECIMALS`].
    pub fn linearise(&self, into: &mut [Phrase]) -> Result<usize, Escape> {
        let canvas = match find(self.nodes, self.arrangement.canvas) {
            Some(node) => node,
            None => return Err(Escape::NotACanvas(self.arrangement.canvas)),
        };
        let mut at = 0;
        self.say(canvas, 0, into, &mut at)?;
        self.say_below(canvas.id, 1, into, &mut at)?;
        Ok(at)
    }

    /// One node, said.
    ///
    /// The content is copied across whole rather than converted. An earlier
    /// version narrowed it to [`Text`] and mapped the other three kinds to the
    /// empty string, so a clip declared the honest way — `Content::Media`,
    /// samples the application holds elsewhere — linearised to a nameless phrase
    /// with a time and nothing else. There is no conversion here now, and so
    /// there is no conversion to forget to extend: a fifth `Content` kind
    /// arrives in every phrase without this function being touched.
    fn say(
        &self,
        node: &Node,
        depth: u8,
        into: &mut [Phrase],
        at: &mut usize,
    ) -> Result<(), Escape> {
        if *at >= into.len() {
            return Err(Escape::NoRoom);
        }
        let placement = self.arrangement.placement_of(node.id);
        let when = match placement {
            Some(placement) => self.spoken(node.id, placement.span)?,
            None => Spoken::Untimed,
        };
        into[*at] = Phrase {
            node: node.id,
            role: node.role,
            content: node.content,
            depth,
            selected: node.state.holds(StateSet::SELECTED),
            in_selection: matches!(placement, Some(placement) if self.within_selection(placement)),
            when,
            operable: Operable::of(node),
        };
        *at += 1;
        Ok(())
    }

    /// Everything under `parent`, in declaration order, depth first.
    fn say_below(
        &self,
        parent: NodeId,
        depth: u8,
        into: &mut [Phrase],
        at: &mut usize,
    ) -> Result<(), Escape> {
        for child in children(self.nodes, parent) {
            self.say(child, depth, into, at)?;
            self.say_below(child.id, depth + 1, into, at)?;
        }
        Ok(())
    }

    /// A placement, in the unit a description speaks.
    fn spoken(&self, node: NodeId, span: Span) -> Result<Spoken, Escape> {
        let base = self.arrangement.base;
        match span {
            Span::At(instant) => match base.seconds_from_ticks(instant, SPOKEN_DECIMALS) {
                Some(when) => Ok(Spoken::At(when)),
                None => Err(Escape::Unsayable(node)),
            },
            Span::Between(extent) => {
                let from = base.seconds_from_ticks(extent.start(), SPOKEN_DECIMALS);
                let to = base.seconds_from_ticks(extent.end(), SPOKEN_DECIMALS);
                match (from, to) {
                    (Some(from), Some(to)) => Ok(Spoken::Between(from, to)),
                    _ => Err(Escape::Unsayable(node)),
                }
            }
        }
    }
}

/// When a phrase is, in seconds, for a consumer that does not know what a tick
/// is worth.
///
/// Three variants because a description has three cases and not two: the canvas
/// and its lanes are not placed in the dimension at all, a marker is a point in
/// it, and a clip is a stretch. Every [`Quantity`] here is in
/// [`Unit::Seconds`] at [`SPOKEN_DECIMALS`], which is what makes *4.2 s*
/// sayable by something that has never heard of this canvas's base.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Spoken {
    /// Not placed in the dimension: the canvas itself, or a lane, or anything
    /// else the tree declares under them.
    Untimed,
    /// At an instant.
    At(Quantity),
    /// From the first, up to the second.
    Between(Quantity, Quantity),
}

/// One line of a description: what a consumer that renders nothing has to work
/// with.
///
/// # Why it carries the content rather than only an identity
///
/// Because a phrase that carried an identity alone would force every consumer to
/// walk the tree again to say anything, and the point of a linearisation is that
/// the consumer does not have to understand the tree. The cost is that a phrase
/// is a couple of hundred bytes, for the reason a [`Node`] is — this is a thing
/// to be read rather than sent, and the day it is sent the text goes by
/// reference into an arena.
///
/// It carries the whole [`Content`] and not a string. A narrower field has to
/// decide what to do with the three kinds that are not text, and every answer to
/// that is a loss: the audio clip whose content is [`Content::Media`] is the
/// *canonical* clip rather than an edge case, and a linearisation that dropped
/// its capability would demonstrate the screen-reader half for the one content
/// kind a timeline mostly does not have.
///
/// # Why two flags for selection
///
/// [`selected`](Self::selected) is what the node says about itself.
/// [`in_selection`](Self::in_selection) is whether the canvas's declared
/// selection covers where it sits. They are two different claims, they can
/// disagree — an application that has moved the selection and not yet
/// redeclared the node states is in an ordinary intermediate frame, not a
/// defect — and a projection that saw only one of them would be guessing about
/// the other. Nothing here reconciles them, deliberately: a module that picked
/// a winner would be making a decision the application did not delegate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phrase {
    /// Which node this is about. Stable, so an agent can address what it hears.
    pub node: NodeId,
    /// What it is.
    pub role: Role,
    /// What it holds, whole and unconverted.
    pub content: Content,
    /// How deep under the canvas it sits. The canvas itself is zero.
    pub depth: u8,
    /// The node declares itself selected.
    pub selected: bool,
    /// The canvas's declared selection covers where this sits.
    pub in_selection: bool,
    /// When it is.
    pub when: Spoken,
    /// What may be invoked on it. Carried so that a consumer which linearised
    /// the canvas can act on what it heard without walking the tree again —
    /// which is the difference between a description and a script.
    pub operable: Operable,
}

impl Phrase {
    /// A phrase nobody wrote: what a caller fills a buffer with before handing
    /// it to [`Participating::linearise`].
    ///
    /// Its role is [`Role::Surface`] because the vocabulary has no *no role* and
    /// this crate will not invent one for a filler — the honest reading is that
    /// the slots past what `linearise` returns are not phrases at all, and a
    /// consumer that reads them is reading past the end of what was said.
    pub const UNSAID: Self = Self {
        node: NodeId::UNNAMED,
        role: Role::Surface,
        content: Content::None,
        depth: 0,
        selected: false,
        in_selection: false,
        when: Spoken::Untimed,
        operable: Operable::Inert,
    };

    /// What it says, for a consumer that wanted words and can be told there are
    /// none.
    ///
    /// A convenience over [`content`](Self::content), and deliberately an
    /// `Option`: a media clip has no words, which is a fact about the
    /// vocabulary's single content field rather than something to paper over
    /// with an empty string. A projection that needs a name for one reads the
    /// [`Role::Label`] pointed at it by
    /// [`crate::node::Relation::LabelledBy`].
    #[must_use]
    pub const fn says(&self) -> Option<Text> {
        match self.content {
            Content::Text(text) => Some(text),
            Content::None | Content::Value(_) | Content::Media(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::{
        CapRef, Constraints, Content, Defect, Flow, Node, NodeId, Quantity, Reading, Role,
        StateSet, Text, Unit, canvas_escapes, check, children, find,
    };

    const SURFACE: NodeId = NodeId::new(300);
    const TRANSPORT: NodeId = NodeId::new(310);
    const PLAY: NodeId = NodeId::new(311);
    const SEQUENCE: NodeId = NodeId::new(320);
    const DIALOGUE: NodeId = NodeId::new(321);
    const AMBIENCE: NodeId = NodeId::new(322);
    const MUSIC: NodeId = NodeId::new(323);
    const BED: NodeId = NodeId::new(324);
    const FOLEY: NodeId = NodeId::new(325);
    const STEPS: NodeId = NodeId::new(326);
    const DOOR: NodeId = NodeId::new(327);
    const RAIN: NodeId = NodeId::new(328);
    const PLAYHEAD: NodeId = NodeId::new(329);
    /// A list of takes, declared under a canvas by an application that would
    /// rather not place anything.
    const TAKES: NodeId = NodeId::new(330);
    const TAKE_ONE: NodeId = NodeId::new(331);
    /// A caption, declared under a clip, which is the same escape one level
    /// further down.
    const CAPTION: NodeId = NodeId::new(332);
    /// A second canvas on the same surface, with a lane and a clip of its own.
    const REEL: NodeId = NodeId::new(340);
    const REEL_LANE: NodeId = NodeId::new(341);
    const REEL_CLIP: NodeId = NodeId::new(342);

    /// The canvas's own rendering: the pixels the application draws, named and
    /// never resolved here.
    const RENDERING: CapRef = CapRef::new(0x0300);
    /// What operating the transport's play button does. It is enabled and it
    /// carries this, so a scope-blind reading of that node answers *invocable* —
    /// which is what makes the refusal in
    /// `what_may_be_invoked_is_asked_of_a_canvas_and_not_of_a_node` about scope
    /// rather than about the node having nothing to say.
    const PLAY_INTENT: CapRef = CapRef::new(0x0301);
    /// What operating the other canvas's clip does.
    const REEL_INTENT: CapRef = CapRef::new(0x0340);
    /// The samples behind the music bed — the honest form of a clip's content.
    const BED_SAMPLES: CapRef = CapRef::new(0x03b0);
    /// What operating the music bed does.
    const BED_INTENT: CapRef = CapRef::new(0x0306);
    /// What operating the door slam does.
    const DOOR_INTENT: CapRef = CapRef::new(0x0309);
    /// What operating the rain would do, if the application were letting it.
    const RAIN_INTENT: CapRef = CapRef::new(0x030a);

    fn text(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits within TEXT_MAX"))
    }

    /// A time in tenths of a second, as ticks of the canvas's base.
    ///
    /// The scale is in the name because `CLAUDE.md` asks for that wherever the
    /// writer knows what is being counted, and here it does: these are the
    /// numbers section 13's sentence is written in, and `42` is 4.2 s.
    fn seconds_x10(value: i64) -> Ticks {
        TimeBase::FLICKS.ticks_from_seconds(value, 1).expect("a time this base can name")
    }

    /// A stretch, in tenths of a second.
    fn span_x10(from: i64, to: i64) -> Extent {
        Extent::new(seconds_x10(from), seconds_x10(to)).expect("a span that advances")
    }

    /// The timeline, as nodes.
    ///
    /// Three lanes rather than the two `node.rs`'s tests needed, because the
    /// question is about *track 3* and a case with no third lane would let the
    /// query pass by having nowhere else to go. `Music` carries a clip with the
    /// same extent as the answer for the same reason: a query that ignored the
    /// lane would return it, and a test that could not tell the two apart would
    /// be asserting less than the sentence it is named after.
    ///
    /// Four declarations here are deliberately unalike, because a fixture where
    /// every node is the same shape proves only that the module works on nodes
    /// that are the same shape:
    ///
    /// - the canvas carries [`Content::Media`], which is section 13's *custom
    ///   rendering* — the half of the sentence a fixture full of `Content::None`
    ///   passes by never declaring;
    /// - `bed` carries media too, because a music bed *is* samples, and a clip
    ///   whose content is text is the unusual case rather than the normal one;
    /// - `ambience` carries no intent, so something in the tree is addressable
    ///   and not scriptable, and the two adjectives cannot be satisfied by one
    ///   assertion;
    /// - `rain` carries an intent and is not enabled, so *not now* has a case
    ///   distinct from *not ever*.
    fn timeline() -> [Node; 13] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_content(text("Sequence"))
                .with_layout(Constraints::flowing(Flow::Block)),
            Node::new(TRANSPORT, SURFACE, Role::Group)
                .with_content(text("Transport"))
                .with_layout(Constraints::flowing(Flow::Inline)),
            Node::new(PLAY, TRANSPORT, Role::Command)
                .with_content(text("Play"))
                .with_state(StateSet::ENABLED)
                .with_intent(PLAY_INTENT),
            // The canvas. Its pixels are the application's — named, not inlined,
            // never resolved here — and everything below it is the content it is
            // not allowed to keep.
            Node::new(SEQUENCE, SURFACE, Role::Canvas)
                .with_content(Content::Media(RENDERING))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0302))
                .with_layout(Constraints::flowing(Flow::Own).growing(1)),
            Node::new(DIALOGUE, SEQUENCE, Role::Track)
                .with_content(text("Dialogue"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0303)),
            // No intent: a clip on a locked lane. Addressable, describable, and
            // nothing an agent may invoke — which `node.rs` spells as the
            // absence of a capability and this module has to be able to say.
            Node::new(AMBIENCE, DIALOGUE, Role::Clip)
                .with_content(text("ambience"))
                .with_state(StateSet::ENABLED),
            Node::new(MUSIC, SEQUENCE, Role::Track)
                .with_content(text("Music"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0305)),
            Node::new(BED, MUSIC, Role::Clip)
                .with_content(Content::Media(BED_SAMPLES))
                .with_state(StateSet::ENABLED)
                .with_intent(BED_INTENT),
            Node::new(FOLEY, SEQUENCE, Role::Track)
                .with_content(text("Foley"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0307)),
            Node::new(STEPS, FOLEY, Role::Clip)
                .with_content(text("steps"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0308)),
            Node::new(DOOR, FOLEY, Role::Clip)
                .with_content(text("door-slam"))
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(DOOR_INTENT),
            // Busy and not enabled: the application is rendering it and is
            // declining to operate it meanwhile.
            Node::new(RAIN, FOLEY, Role::Clip)
                .with_content(text("rain"))
                .with_state(StateSet::BUSY)
                .with_intent(RAIN_INTENT),
            Node::new(PLAYHEAD, SEQUENCE, Role::Marker)
                .with_content(text("Playhead"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x030b)),
        ]
    }

    /// The same timeline, as an arrangement: the four clauses, met.
    ///
    /// `Dialogue` deliberately runs out at 2 s, so there is a stretch of the
    /// dimension with nothing in it — which is what the selection test needs and
    /// what a set of selected identities could not express.
    fn arrangement() -> Arrangement {
        let placements = [
            Placement::over(AMBIENCE, span_x10(0, 20)),
            Placement::over(BED, span_x10(42, 68)),
            Placement::over(STEPS, span_x10(0, 42)),
            Placement::over(DOOR, span_x10(42, 68)),
            Placement::over(RAIN, span_x10(68, 100)),
            Placement::at(PLAYHEAD, seconds_x10(50)),
        ];
        let mut declared = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in placements {
            declared = declared.with_placement(placement).expect("within PLACED_MAX");
        }
        declared
    }

    /// A canvas, a marker on it, and nothing else: the smallest tree that can
    /// carry a time no description is able to hold.
    fn coarse_tree() -> [Node; 3] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_layout(Constraints::flowing(Flow::Block)),
            Node::new(SEQUENCE, SURFACE, Role::Canvas).with_content(Content::Media(RENDERING)),
            Node::new(PLAYHEAD, SEQUENCE, Role::Marker).with_content(text("Playhead")),
        ]
    }

    /// A declaration at one tick a second, placing the playhead past where a
    /// description in seconds can reach.
    fn coarse_arrangement() -> Arrangement {
        let base = TimeBase::new(1).expect("a base, and the coarsest one there is");
        Arrangement::declaring(SEQUENCE, base, Selection::NOTHING)
            .with_placement(Placement::at(PLAYHEAD, Ticks::new(i64::MAX)))
            .expect("one placement")
    }

    /// The nth occupant of a full screen. Written once, because the arrangement
    /// and the tree below have to agree about it or the bound is exercised
    /// against nothing.
    fn full_screen_occupant(nth: usize) -> NodeId {
        NodeId::new(1_000 + nth as u64)
    }

    /// Sixty-four placements of distinct occupants: the bound, filled exactly.
    fn a_full_screen() -> Arrangement {
        let mut declared = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for nth in 0..PLACED_MAX {
            let at = nth as i64;
            let placement = Placement::over(full_screen_occupant(nth), span_x10(at, at + 1));
            declared = declared.with_placement(placement).expect("sixty-four fit");
        }
        declared
    }

    /// The tree those sixty-four placements are about: one canvas, one lane, and
    /// a clip for every occupant `a_full_screen` places.
    ///
    /// Without it `PLACED_MAX` is exercised against the builder and nothing
    /// else, because ids that exist in no tree never reach `admit`.
    fn a_full_screen_tree() -> [Node; 67] {
        let surface = Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
            .with_layout(Constraints::flowing(Flow::Block));
        let mut nodes = [surface; 67];
        nodes[1] =
            Node::new(SEQUENCE, SURFACE, Role::Canvas).with_content(Content::Media(RENDERING));
        nodes[2] = Node::new(DIALOGUE, SEQUENCE, Role::Track).with_content(text("Dialogue"));
        for nth in 0..PLACED_MAX {
            nodes[3 + nth] = Node::new(full_screen_occupant(nth), DIALOGUE, Role::Clip);
        }
        nodes
    }

    /// One surface, two canvases: a sequence, and a reel with a lane and a clip
    /// of its own.
    ///
    /// The fixture every question about scope needs. A stranger id and a
    /// transport button are easy to refuse; the case that decides whether the
    /// scope is real is a node of exactly the right shape — a `Role::Track`, a
    /// `Role::Clip` that is enabled and carries an intent — belonging to a
    /// different surface, because that is the answer a canvas would otherwise
    /// give confidently and have no standing to give.
    fn two_canvases() -> [Node; 7] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_content(text("Sequence"))
                .with_layout(Constraints::flowing(Flow::Block)),
            Node::new(SEQUENCE, SURFACE, Role::Canvas).with_content(Content::Media(RENDERING)),
            Node::new(DIALOGUE, SEQUENCE, Role::Track).with_content(text("Dialogue")),
            Node::new(AMBIENCE, DIALOGUE, Role::Clip).with_content(text("ambience")),
            Node::new(REEL, SURFACE, Role::Canvas),
            Node::new(REEL_LANE, REEL, Role::Track).with_content(text("Reel")),
            Node::new(REEL_CLIP, REEL_LANE, Role::Clip)
                .with_content(text("take"))
                .with_state(StateSet::ENABLED)
                .with_intent(REEL_INTENT),
        ]
    }

    /// The sequence of `two_canvases`, declared: one lane, one clip, placed. The
    /// floor exactly, so that what the tests below assert is the scope and not
    /// the declaration.
    fn one_of_two() -> Arrangement {
        Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(AMBIENCE, span_x10(0, 20)))
            .expect("room for one")
    }

    /// A canvas that places one token clip and hangs the content it actually
    /// draws off itself as a list, where no clause counts it.
    fn content_smuggled() -> [Node; 6] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_layout(Constraints::flowing(Flow::Block)),
            Node::new(SEQUENCE, SURFACE, Role::Canvas).with_content(Content::Media(RENDERING)),
            Node::new(DIALOGUE, SEQUENCE, Role::Track).with_content(text("Dialogue")),
            Node::new(AMBIENCE, DIALOGUE, Role::Clip).with_content(text("ambience")),
            Node::new(TAKES, SEQUENCE, Role::List).with_content(text("Takes")),
            Node::new(TAKE_ONE, TAKES, Role::Item).with_content(text("take 1")),
        ]
    }

    /// **The exit, and section 13's own falsification.**
    ///
    /// *Can an agent select the clip between 4.2 s and 6.8 s on track 3, and can
    /// a screen reader describe it, without either one seeing a single pixel?*
    ///
    /// Nothing in this test constructs, reads or asserts a pixel. The canvas
    /// declares a rendering — it is a canvas, it draws itself — and the test
    /// never resolves the handle; every fact asserted below comes from the
    /// declaration, which is the whole of what the question is asking.
    #[test]
    fn an_agent_selects_the_clip_between_4_2_s_and_6_8_s_on_track_3_and_a_reader_describes_it() {
        let tree = timeline();
        let declared = arrangement();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");

        // *Without seeing a single pixel.* The pixels exist and are declared:
        // this canvas draws its own and says so with a capability. Nothing below
        // resolves it, and nothing below would answer differently if it were
        // absent — which is the half of section 13's sentence a fixture that
        // declared no rendering at all could never have shown.
        assert_eq!(canvas.rendering(), Some(RENDERING), "the pixels stay the application's");

        // *Track 3.* The agent has no pixels, so the lane is the third one the
        // canvas declares, resolved from the tree — and the identity it gets
        // back is what it addresses from here, because an ordinal is exactly the
        // brittle thing `NodeId` exists to replace.
        let lane = children(&tree, SEQUENCE)
            .filter(|node| node.role == Role::Track)
            .nth(2)
            .expect("a third lane")
            .id;
        assert_eq!(lane, FOLEY);

        // *Between 4.2 s and 6.8 s.* The agent speaks seconds; the canvas
        // declared what a tick is worth; the conversion is exact and there is no
        // floating-point value anywhere in it.
        let span = Extent::new(seconds_x10(42), seconds_x10(68)).expect("a span that advances");
        let answer = canvas.occupant_between(lane, span).expect("a lane of this canvas");
        assert_eq!(answer, Sole::One(DOOR), "one clip covers it, and it is named");
        let clip = answer.one().expect("exactly one");

        // And the lane was load-bearing: another lane holds a clip over exactly
        // the same stretch, and the query did not return it.
        assert_eq!(canvas.occupant_between(MUSIC, span), Ok(Sole::One(BED)));

        // *Select.* What comes back is a node of the tree, so what an agent does
        // next is invoke the capability that node carries — an authorised call
        // and not a click at a coordinate. This is the *scriptable* half, and it
        // is asked of the module rather than of the fixture: the answer is
        // computed from the declaration, and an occupant with nothing to invoke
        // gets a different word rather than the same one.
        let node = find(&tree, clip).expect("the clip is a node");
        assert_eq!(node.role, Role::Clip);
        assert_eq!(node.parent, lane);
        assert_eq!(canvas.lane_of(clip), Ok(Some(FOLEY)));
        assert_eq!(canvas.operable(clip), Ok(Operable::Invocable(DOOR_INTENT)));
        assert_eq!(canvas.operable(AMBIENCE), Ok(Operable::Inert), "addressable, not scriptable");
        let rain = canvas.operable(RAIN);
        assert_eq!(rain, Ok(Operable::Disabled(RAIN_INTENT)), "not now, which is not not ever");
        assert_eq!(
            canvas.operable(PLAY),
            Err(Escape::NotOfThisCanvas(PLAY)),
            "and a canvas refuses to answer for somebody else's surface"
        );

        // The same identity again by the other route: declare the selection an
        // agent would make, and ask what it touches.
        let selecting = declared.with_selection(Selection::over(span).on(lane));
        let selected = selecting.admit(&tree).expect("the selection names a lane of this canvas");
        let mut touched = selected.selected();
        assert_eq!(touched.next(), Some(DOOR));
        assert_eq!(touched.next(), None, "the stretch touches one clip on that lane");

        // *And can a screen reader describe it.* A linearisation names every
        // occupant, in reading order, with its time in seconds — from the
        // declaration, with nothing rendered.
        let mut said = [Phrase::UNSAID; 16];
        let count = selected.linearise(&mut said).expect("room enough to say it");
        assert_eq!(count, selected.phrases());
        let said = &said[..count];

        let spoken = said.iter().find(|phrase| phrase.node == DOOR).expect("the clip is named");
        assert_eq!(spoken.content, text("door-slam"));
        assert_eq!(spoken.says().expect("this one has words").as_str(), "door-slam");
        assert_eq!(spoken.role, Role::Clip);
        assert_eq!(
            spoken.when,
            Spoken::Between(
                Quantity::scaled(4_200, 3, Unit::Seconds),
                Quantity::scaled(6_800, 3, Unit::Seconds)
            ),
            "4.2 s and 6.8 s, as integers with their scale beside them"
        );
        assert!(spoken.selected, "the node says it is selected");
        assert!(spoken.in_selection, "and the declared selection covers where it sits");
        assert_eq!(
            spoken.operable,
            Operable::Invocable(DOOR_INTENT),
            "and what to invoke rides on the phrase, so hearing it is enough to act on it"
        );

        // The lane is named too, and said before what is on it, so a reader can
        // say *on Foley* without being told which lane separately.
        let lane_said = said.iter().find(|phrase| phrase.node == lane).expect("the lane is named");
        assert_eq!(lane_said.content, text("Foley"));
        assert_eq!(lane_said.when, Spoken::Untimed, "a lane is not placed in the dimension");
        assert!(lane_said.depth < spoken.depth);
        let lane_at = said.iter().position(|phrase| phrase.node == lane).expect("the lane");
        let clip_at = said.iter().position(|phrase| phrase.node == DOOR).expect("the clip");
        assert!(lane_at < clip_at, "a lane is said before what is on it");

        // The decoy is described too, and described as not selected — which is
        // the difference between a description and a rendering: it says what is
        // true of everything, not what is visible. It is also the media clip, so
        // what a reader is handed is a capability and not an empty string.
        let bed = said.iter().find(|phrase| phrase.node == BED).expect("the other lane's clip");
        assert!(!bed.selected);
        assert!(!bed.in_selection, "the selection was confined to one lane");
        assert_eq!(bed.content, Content::Media(BED_SAMPLES));
    }

    #[test]
    fn a_canvas_the_vocabulary_accepts_can_still_be_an_escape() {
        // The floor, and the reason this module exists at all. `node.rs` is
        // satisfied by this tree — the canvas has children, nothing is
        // misplaced, and its own escape metric reports zero — and the canvas is
        // *still* opaque if nobody said when the door slam is. Knowing that a
        // clip exists is not knowing where it is, and every question section 13
        // asks is about where.
        let tree = timeline();
        check(&tree).expect("the vocabulary is satisfied");
        assert_eq!(canvas_escapes(&tree), 0);

        let mut incomplete = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in arrangement().placements().filter(|placed| placed.occupant != DOOR) {
            incomplete = incomplete.with_placement(placement).expect("within PLACED_MAX");
        }
        // And the refusal is the whole enforcement: there is no `Participating`,
        // so there is nothing to ask, and the query in the test above cannot be
        // written against this declaration at all. That is a property of the
        // type rather than something to assert — an `assert!(… .is_err())` used
        // to follow this line, and no edit could redden it that did not redden
        // this line first.
        assert_eq!(incomplete.admit(&tree).err(), Some(Escape::Unplaced(DOOR)));
    }

    #[test]
    fn a_canvas_that_declares_one_lane_and_places_nothing_is_the_cheapest_escape() {
        // The hole the first version of this module left, and the one an
        // application finds first because it costs one node. Take the timeline
        // down to a canvas and a single empty lane: `node.rs` is satisfied —
        // `EmptyCanvas` wants *a* child and there is one — and an arrangement
        // that places nothing then met clause 4 by leaving it nothing to be met
        // with. That is the whole sequence in pixels with a caption under it,
        // admitted as a full participant.
        let tree = timeline();
        let one_empty_lane = &tree[..5];
        check(one_empty_lane).expect("the vocabulary is satisfied by one empty lane");
        assert_eq!(canvas_escapes(one_empty_lane), 0, "and its own metric sees nothing wrong");
        assert_eq!(one_empty_lane[3].id, SEQUENCE);
        assert_eq!(one_empty_lane[4].role, Role::Track);

        let silent = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        assert!(silent.is_empty());
        assert_eq!(
            silent.admit(one_empty_lane).err(),
            Some(Escape::NothingPlaced(SEQUENCE)),
            "a canvas that places nothing has declared nothing"
        );

        // The refusal is about the arrangement and not about the tree being
        // small: the same empty declaration over the whole timeline is refused
        // by the same word, rather than by `Unplaced` naming one node and
        // implying the rest are fine.
        assert_eq!(silent.admit(&tree).err(), Some(Escape::NothingPlaced(SEQUENCE)));

        // And one real placement is the difference. This is deliberately the
        // weakest declaration that passes — one lane, one clip — because the
        // floor is a floor and not a quality bar, and saying so here is what
        // keeps RFC 0078's *compliance in letter and not in substance* an
        // honest reversal condition rather than a claim this test disproves.
        let lone = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(AMBIENCE, span_x10(0, 20)))
            .expect("room for one");
        let admitted = lone.admit(&tree[..6]).expect("one clip, placed, is the floor exactly");
        assert_eq!(admitted.occupant_at(DIALOGUE, seconds_x10(10)), Ok(Sole::One(AMBIENCE)));
    }

    #[test]
    fn the_pixels_are_declared_and_no_answer_comes_from_them() {
        // Section 13's sentence has two halves and this is the second held to
        // account. The canvas declares its own rendering; the module hands a
        // projection the handle; and nothing the module answers depends on what
        // is behind it.
        let tree = timeline();
        let declared = arrangement();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");
        assert_eq!(canvas.rendering(), Some(RENDERING));

        let mut said = [Phrase::UNSAID; 16];
        let count = canvas.linearise(&mut said).expect("room enough to say it");

        // The same tree with different pixels behind the canvas. Every phrase
        // but the canvas's own is identical, which is what *the content does not
        // come from the rendering* means once it is a fact rather than an
        // intention.
        let mut repainted = timeline();
        repainted[3] = repainted[3].with_content(Content::Media(CapRef::new(0x0fff)));
        let other = declared.admit(&repainted).expect("still a canvas that declared its content");
        assert_eq!(other.rendering(), Some(CapRef::new(0x0fff)));
        let mut also = [Phrase::UNSAID; 16];
        assert_eq!(other.linearise(&mut also), Ok(count));
        for (before, after) in said[1..count].iter().zip(also[1..count].iter()) {
            assert_eq!(before, after, "nothing below the canvas moved when the pixels did");
        }
        assert_eq!(said[0].node, SEQUENCE);
        assert_eq!(said[0].content, Content::Media(RENDERING));
        assert_eq!(also[0].content, Content::Media(CapRef::new(0x0fff)));

        // A canvas may also declare no rendering at all and answer the same
        // questions, which is the other direction of the same independence: the
        // semantic model is not derived from the pixels, so it does not need
        // them in order to exist.
        let mut unpainted = timeline();
        unpainted[3] = unpainted[3].with_content(Content::None);
        let plain = declared.admit(&unpainted).expect("a canvas that draws nothing of its own");
        assert_eq!(plain.rendering(), None);
        assert_eq!(plain.occupant_between(FOLEY, span_x10(42, 68)), Ok(Sole::One(DOOR)));
    }

    #[test]
    fn the_base_decides_whether_a_sample_boundary_survives() {
        // Why the time base is a decision and not a formatting detail. Sample
        // 201 of a 48 kHz stream — counted from zero, which is why this says the
        // index and not the ordinal: for a stream whose first sample is at t = 0
        // the 201st is at 200/48000 s, and an off-by-one in a sentence about
        // sample boundaries is the mistake this whole section is against — is at
        // 201/48000 s = 0.0041875 s. That is a number of ticks at one base and
        // is not at the other.
        //
        // The scale is in the name, as `CLAUDE.md` asks: ten-millionths of a
        // second, which is what seven decimal places of a second are. It was
        // `x10m` for one round, and `m` reads as *milli* at least as readily as
        // it reads as *million*.
        let sample_x1e7 = 41_875;

        let flicks = TimeBase::FLICKS;
        assert!(flicks.names(48_000, 1), "a sample");
        assert!(flicks.names(44_100, 1), "a sample of the other common rate");
        assert!(flicks.names(24, 1), "a film frame");
        let exact = flicks.ticks_from_seconds(sample_x1e7, 7).expect("a tick");
        let back = flicks.seconds_from_ticks(exact, 7).expect("and back again");
        assert_eq!(back.scaled, sample_x1e7, "the base names the boundary exactly");
        assert_eq!(back.unit, Unit::Seconds);

        // The headline case, and the reason the predicate takes a fraction. NTSC
        // runs at 30000/1001 frames a second, which flicks names exactly — and
        // which cannot be asked as a whole number at all. 29 970 is the rounded
        // *name* of that rate and is not the rate, so answering about it would
        // be answering a question nobody has.
        assert!(flicks.names(30_000, 1_001), "NTSC, asked as the rate it is");
        assert!(!flicks.names(29_970, 1), "and the rounded name of it is a different rate");
        assert_eq!(
            u64::from(flicks.ticks_per_second()) * 1_001 / 30_000,
            23_543_520,
            "one NTSC frame, in whole ticks"
        );

        // The lazy answer, and what it costs: the boundary lands between two
        // samples, the declaration is now approximate, and nothing downstream
        // can tell that it happened.
        let millis = TimeBase::new(1_000).expect("a base");
        assert!(!millis.names(48_000, 1));
        assert!(!millis.names(30_000, 1_001), "nor a frame of the video it would carry");
        let rounded = millis.ticks_from_seconds(sample_x1e7, 7).expect("a tick, of a sort");
        let lost = millis.seconds_from_ticks(rounded, 7).expect("and back again");
        assert_ne!(lost.scaled, sample_x1e7, "a millisecond base cannot name a sample");

        // A rate with a zero on either side of the line is not a rate.
        assert!(!flicks.names(0, 1));
        assert!(!flicks.names(24, 0));

        assert_eq!(seconds_x10(0), Ticks::ORIGIN, "zero is where the application put it");
        assert_eq!(TimeBase::new(0).err(), Some(Escape::NotATimeBase));
        assert_eq!(flicks.ticks_from_seconds(1, DECIMALS_MAX + 1), None);
    }

    #[test]
    fn a_point_in_the_dimension_is_a_marker_and_not_an_empty_clip() {
        // The vocabulary already has a word for a point, so an extent that does
        // not advance is refused where it is written rather than admitted and
        // then guessed about.
        let instant = seconds_x10(42);
        assert_eq!(Extent::new(instant, instant).err(), Some(Escape::NotAnExtent));
        assert_eq!(Extent::new(seconds_x10(68), instant).err(), Some(Escape::NotAnExtent));

        // And the two kinds do not stand in for one another: a clip placed at an
        // instant is somebody who meant a marker, which is a different claim
        // about what is at 4.2 s.
        let tree = timeline();
        let mut mistimed = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in arrangement().placements() {
            let placement =
                if placement.occupant == DOOR { Placement::at(DOOR, instant) } else { placement };
            mistimed = mistimed.with_placement(placement).expect("within PLACED_MAX");
        }
        assert_eq!(mistimed.admit(&tree).err(), Some(Escape::Mistimed(DOOR)));
    }

    #[test]
    fn a_selection_can_cover_a_stretch_that_holds_nothing() {
        // The argument for a selection over the dimension rather than a set of
        // identities, made as a test. `Dialogue` runs out at 2 s, so three to
        // four seconds on that lane is a real, declarable, empty selection — and
        // a set of selected clips has no way to be it.
        let tree = timeline();
        let declared = arrangement();

        let over_nothing = declared.with_selection(Selection::over(span_x10(30, 40)).on(DIALOGUE));
        let empty = over_nothing.admit(&tree).expect("a lane of this canvas");
        assert!(!empty.arrangement().selection().is_nothing(), "something is selected");
        assert_eq!(empty.selected().count(), 0, "and nothing is in it");

        // The same stretch on another lane is not empty, which is what makes the
        // first assertion a fact about the dimension rather than about the
        // numbers.
        let over_steps = declared.with_selection(Selection::over(span_x10(30, 40)).on(FOLEY));
        let busy = over_steps.admit(&tree).expect("a lane of this canvas");
        let mut touched = busy.selected();
        assert_eq!(touched.next(), Some(STEPS));
        assert_eq!(touched.next(), None);

        // Nothing selected is a declaration too, and it is not the same thing as
        // a selection that touches nothing.
        let quiet = declared.admit(&tree).expect("the canvas participates");
        assert!(quiet.arrangement().selection().is_nothing());
        assert_eq!(quiet.selected().count(), 0);
    }

    #[test]
    fn a_selection_over_the_dimension_touches_the_cues_inside_it() {
        // A marker is placed in the dimension, so a selection that is a stretch
        // of the dimension covers one sitting inside it. The playhead at 5.0 s
        // is inside 4.2 s to 6.8 s by every reading an editor has, and the first
        // version of this module dropped it — silently, and with a doc comment
        // on `in_selection` that was then false about the playhead.
        let tree = timeline();
        let across = arrangement().with_selection(Selection::over(span_x10(42, 68)));
        let canvas = across.admit(&tree).expect("the canvas participates");

        let mut touched = canvas.selected();
        assert_eq!(touched.next(), Some(BED), "a clip on one lane");
        assert_eq!(touched.next(), Some(DOOR), "a clip on another");
        assert_eq!(touched.next(), Some(PLAYHEAD), "and the cue between them");
        assert_eq!(touched.next(), None);

        // Half-open at both ends, like everything else here: the steps end where
        // the selection starts and are not in it.
        assert!(!canvas.selected().any(|node| node == STEPS));

        let mut said = [Phrase::UNSAID; 16];
        let count = canvas.linearise(&mut said).expect("room enough to say it");
        let playhead =
            said[..count].iter().find(|phrase| phrase.node == PLAYHEAD).expect("the playhead");
        assert!(playhead.in_selection, "and a reader is told so");

        // *What occupies this stretch* is still a question about clips: a
        // playhead over a clip has not made the clip two things. The two
        // questions differ on purpose, and this is where they differ.
        let mut over = canvas.occupants_over(FOLEY, span_x10(42, 68)).expect("a lane here");
        assert!(!over.any(|node| node == PLAYHEAD));
    }

    #[test]
    fn a_crossfade_is_two_occupants_and_the_answer_is_refused_rather_than_guessed() {
        // Overlap on one lane is not a defect — it is a crossfade, and a
        // vocabulary that refused it would have refused an hour of every film.
        // What is refused instead is a confident wrong answer: asked what is at
        // 6.4 s this says *two*, and asked what is over that stretch it says
        // which two.
        let tree = timeline();
        let mut crossfaded = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in arrangement().placements() {
            let placement = if placement.occupant == RAIN {
                Placement::over(RAIN, span_x10(60, 100))
            } else {
                placement
            };
            crossfaded = crossfaded.with_placement(placement).expect("within PLACED_MAX");
        }
        let canvas = crossfaded.admit(&tree).expect("a canvas that declared its content");

        // The pair this test is named after, and the pair that could not fail
        // while both refusals were spelled `None`: *two answers* and *no answer*
        // are different words, and the lane next door says the other one at the
        // same instant. An `assert_ne!` of the two used to follow them, which
        // was decorative — no edit reddens it that does not redden one of these
        // first, and the two exact answers say strictly more.
        let ambiguous = canvas.occupant_at(FOLEY, seconds_x10(64)).expect("a lane of this canvas");
        let vacant = canvas.occupant_at(DIALOGUE, seconds_x10(64)).expect("a lane of this canvas");
        assert_eq!(ambiguous, Sole::Several, "two things cover it");
        assert_eq!(vacant, Sole::Nothing, "that lane ran out");

        // A caller that genuinely wants them collapsed does the collapsing
        // itself, where the next reader can watch it happen.
        assert_eq!(ambiguous.one(), None);

        let mut over = canvas.occupants_over(FOLEY, span_x10(64, 65)).expect("a lane here");
        assert_eq!(over.next(), Some(DOOR));
        assert_eq!(over.next(), Some(RAIN));
        assert_eq!(over.next(), None);

        // The ambiguity is local: the stretch the exit asks about still has one
        // answer, because only one clip covers the whole of it.
        assert_eq!(canvas.occupant_between(FOLEY, span_x10(42, 68)), Ok(Sole::One(DOOR)));
    }

    #[test]
    fn a_marker_names_an_instant_and_does_not_occupy_it() {
        // What is under the playhead, asked of the declaration and answered
        // without a rendering. The playhead itself is not an answer to it: a
        // marker names a point rather than occupying it, or every question about
        // a timeline would have the playhead in its answer.
        let tree = timeline();
        let declared = arrangement();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");

        assert_eq!(canvas.occupant_at(FOLEY, seconds_x10(50)), Ok(Sole::One(DOOR)));
        let ran_out = canvas.occupant_at(DIALOGUE, seconds_x10(50));
        assert_eq!(ran_out, Ok(Sole::Nothing), "that lane ran out");
        // *On the canvas itself*, which is an answer — and is spelled
        // differently from the refusal a node of another canvas gets.
        assert_eq!(canvas.lane_of(PLAYHEAD), Ok(None));

        // Half-open, which is the convention butt-joined clips need: 4.2 s is
        // the first tick of the door slam and not the last of the steps.
        assert_eq!(canvas.occupant_at(FOLEY, seconds_x10(42)), Ok(Sole::One(DOOR)));
        let tick_before = Ticks::new(seconds_x10(42).count() - 1);
        assert_eq!(canvas.occupant_at(FOLEY, tick_before), Ok(Sole::One(STEPS)));

        // And a reader is told where the playhead is, in seconds, like
        // everything else.
        let mut said = [Phrase::UNSAID; 16];
        let count = canvas.linearise(&mut said).expect("room enough to say it");
        let playhead = said[..count]
            .iter()
            .find(|phrase| phrase.node == PLAYHEAD)
            .expect("the playhead is named");
        assert_eq!(playhead.when, Spoken::At(Quantity::scaled(5_000, 3, Unit::Seconds)));
        assert_eq!(playhead.role, Role::Marker);
    }

    #[test]
    fn reading_order_is_the_authors_order_and_not_the_clocks() {
        // The linearisation claims the tree's declaration order, and the only
        // way to hold it to that claim is a tree whose declaration order
        // disagrees with its times. Nothing in `admit` requires a canvas to
        // declare its clips chronologically, so here is one that does not: the
        // Foley lane is declared rain, door, steps and placed at 6.8, 4.2, 0.
        let mut tree = timeline();
        tree.swap(9, 11);
        assert_eq!(tree[9].id, RAIN);
        assert_eq!(tree[11].id, STEPS);
        check(&tree).expect("declaration order is not the vocabulary's business");

        let declared = arrangement();
        let canvas = declared.admit(&tree).expect("nor is it the floor's");
        let mut said = [Phrase::UNSAID; 16];
        let count = canvas.linearise(&mut said).expect("room enough to say it");
        let said = &said[..count];

        let at = |node: NodeId| {
            said.iter().position(|phrase| phrase.node == node).expect("every occupant is said")
        };
        assert!(at(RAIN) < at(DOOR) && at(DOOR) < at(STEPS), "the reader follows the author");

        // And the times say the opposite, which is the whole point: the order is
        // not a claim about the clock, and a consumer that wants chronology has
        // everything it needs to sort — without this module pretending it did.
        let started = |node: NodeId| match said[at(node)].when {
            Spoken::Between(from, _) => from.scaled,
            Spoken::At(_) | Spoken::Untimed => panic!("a clip is a stretch"),
        };
        assert!(started(RAIN) > started(DOOR) && started(DOOR) > started(STEPS));
    }

    #[test]
    fn a_media_clip_keeps_its_capability_through_a_linearisation() {
        // The canonical clip. `node.rs` defines `Content::Media` as pixels or
        // samples the application holds elsewhere, named by capability — which
        // is what an audio clip *is*, so a linearisation that dropped it would
        // be demonstrating the screen-reader half for the one content kind a
        // timeline mostly does not have.
        let tree = timeline();
        let declared = arrangement();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");
        let mut said = [Phrase::UNSAID; 16];
        let count = canvas.linearise(&mut said).expect("room enough to say it");
        let bed = said[..count].iter().find(|phrase| phrase.node == BED).expect("the bed is said");

        assert_eq!(bed.content, Content::Media(BED_SAMPLES), "the capability survives the trip");
        assert_eq!(bed.says(), None, "and it has no words, which is said rather than faked");
        assert_eq!(
            bed.when,
            Spoken::Between(
                Quantity::scaled(4_200, 3, Unit::Seconds),
                Quantity::scaled(6_800, 3, Unit::Seconds)
            ),
            "a clip with no words still has a when"
        );
        assert_eq!(bed.operable, Operable::Invocable(BED_INTENT), "and a what-to-invoke");

        // Every kind of content the vocabulary has reaches a phrase unchanged,
        // which takes a tree that carries all four: the timeline carries text
        // and media, so the other two go on two of its clips here. What stood
        // here instead was an exhaustive `match` said to make a fifth `Content`
        // kind a compile error — and it asserted nothing, because `say` copies
        // the field whole, so a fifth kind needs no edit there and the only
        // compile error would have been in this test.
        let level = Content::Value(Reading::plain(Quantity::scaled(500, 3, Unit::Ratio)));
        let mut mixed = timeline();
        mixed[5] = mixed[5].with_content(Content::None);
        mixed[11] = mixed[11].with_content(level);
        let other = declared.admit(&mixed).expect("what a clip holds is not the floor's business");
        let mut also = [Phrase::UNSAID; 16];
        let mixed_count = other.linearise(&mut also).expect("room enough to say it");
        for phrase in &also[..mixed_count] {
            let node = find(&mixed, phrase.node).expect("a phrase is about a node");
            assert_eq!(phrase.content, node.content, "the field is copied, not converted");
        }
        let mixed_said = &also[..mixed_count];
        let hush = mixed_said.iter().find(|phrase| phrase.node == AMBIENCE).expect("it is said");
        assert_eq!(hush.content, Content::None);
        assert_eq!(hush.says(), None, "a clip holding nothing has no words either");
        let level_said = mixed_said.iter().find(|phrase| phrase.node == RAIN).expect("it is said");
        assert_eq!(level_said.content, level, "and a reading survives the trip whole");
        assert_eq!(level_said.says(), None);
    }

    #[test]
    fn what_is_not_a_canvas_or_not_its_lane_or_not_its_occupant_is_refused() {
        let tree = timeline();

        let not_a_canvas = Arrangement::declaring(TRANSPORT, TimeBase::FLICKS, Selection::NOTHING);
        assert_eq!(not_a_canvas.admit(&tree).err(), Some(Escape::NotACanvas(TRANSPORT)));

        let elsewhere = arrangement().with_selection(Selection::over(span_x10(0, 10)).on(PLAY));
        assert_eq!(elsewhere.admit(&tree).err(), Some(Escape::NotALane(PLAY)));

        // A lane is not an occupant of the dimension: it is the other axis, and
        // the arrangement says nothing about where lanes are because the tree's
        // declaration order already does.
        let lane_placed = arrangement()
            .with_placement(Placement::over(DIALOGUE, span_x10(0, 10)))
            .expect("within PLACED_MAX");
        assert_eq!(lane_placed.admit(&tree).err(), Some(Escape::NotAnOccupant(DIALOGUE)));

        let stranger = NodeId::new(999);
        let unknown = arrangement()
            .with_placement(Placement::over(stranger, span_x10(0, 10)))
            .expect("within PLACED_MAX");
        assert_eq!(unknown.admit(&tree).err(), Some(Escape::NotAnOccupant(stranger)));

        // One occupant, one place, refused where it is written.
        assert_eq!(
            arrangement().with_placement(Placement::over(DOOR, span_x10(0, 10))).err(),
            Some(Escape::PlacedTwice(DOOR))
        );

        // And the scope of a question is the canvas that answers it. A lane, a
        // marker on the canvas and the canvas itself are inside it; the
        // transport button on the same surface is not.
        let whole = arrangement();
        let canvas = whole.admit(&tree).expect("the canvas participates");
        assert!(canvas.declares(SEQUENCE) && canvas.declares(FOLEY) && canvas.declares(PLAYHEAD));
        assert!(!canvas.declares(PLAY) && !canvas.declares(stranger));
        assert_eq!(canvas.operable(stranger), Err(Escape::NotOfThisCanvas(stranger)));
    }

    #[test]
    fn a_declaration_larger_than_a_screen_is_refused_where_it_is_written() {
        // `PLACED_MAX` is what RFC 0078's *what it makes hard* section is
        // entirely about, and a bound nothing ever reaches is a bound nobody has
        // checked. Sixty-four fit; the sixty-fifth is refused by the builder,
        // where the author is present to do something about it — which is the
        // moment the windowing question this module declines to answer becomes
        // theirs.
        let declared = a_full_screen();
        assert_eq!(declared.len(), PLACED_MAX);
        assert!(!declared.is_empty());

        let one_more = Placement::over(NodeId::new(2_000), span_x10(0, 1));
        assert_eq!(declared.with_placement(one_more).err(), Some(Escape::TooManyPlacements));

        // And the bound is met against a tree, not only against the builder.
        // Sixty-four ids that exist in no tree exercise `with_placement` and
        // nothing else, so the walk the module's *Why this is quadratic*
        // section defends — every occupant of the canvas, against every
        // placement — had never been run at the size it is defended at.
        let tree = a_full_screen_tree();
        check(&tree).expect("one lane carrying sixty-four clips is a tree");
        let canvas = declared.admit(&tree).expect("sixty-four placed occupants meet the floor");
        assert_eq!(canvas.phrases(), 66, "the canvas, its lane, and sixty-four clips");
        let last = full_screen_occupant(PLACED_MAX - 1);
        assert_eq!(canvas.occupant_at(DIALOGUE, seconds_x10(63)), Ok(Sole::One(last)));

        // Drop one of the sixty-four and the walk names it, with a full screen
        // on each side of the comparison.
        let dropped = full_screen_occupant(40);
        let mut short = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in declared.placements().filter(|placed| placed.occupant != dropped) {
            short = short.with_placement(placement).expect("sixty-three fit");
        }
        assert_eq!(short.admit(&tree).err(), Some(Escape::Unplaced(dropped)));
    }

    #[test]
    fn a_time_the_base_can_hold_and_a_description_cannot_is_named() {
        // `Escape::Unsayable` guards the conversion to seconds, and a guard is
        // worth having only if it can fire. It cannot at `FLICKS`, for any tick
        // count an `i64` holds — a fact about that base, asserted here so the
        // doc comment saying so is checked rather than believed.
        let huge = Ticks::new(i64::MAX);
        assert!(TimeBase::FLICKS.seconds_from_ticks(huge, SPOKEN_DECIMALS).is_some());

        // It fires at a base the application declared, which is the only reason
        // clause 2 hands the choice over at all: at one tick a second the
        // whole-seconds term is the tick count itself, and a thousand of those
        // do not fit.
        let coarse = TimeBase::new(1).expect("a base");
        assert_eq!(coarse.seconds_from_ticks(huge, SPOKEN_DECIMALS), None);

        let tree = coarse_tree();
        let declared = coarse_arrangement();
        let canvas = declared.admit(&tree).expect("the floor is met: something is placed");

        // The canvas is a participant — the declaration is complete — and the
        // description still refuses, naming the node whose time it could not say
        // rather than emitting a phrase with the time missing.
        let mut said = [Phrase::UNSAID; 4];
        assert_eq!(canvas.linearise(&mut said).err(), Some(Escape::Unsayable(PLAYHEAD)));
    }

    #[test]
    fn a_tree_the_vocabulary_refuses_is_not_a_participant() {
        // The two floors compose, and this one is below. A canvas with no
        // children at all is `node.rs`'s refusal, and asking this module where
        // everything is would be answering a question about a tree nobody has
        // agreed is a tree.
        let tree = timeline();
        let truncated = &tree[..4];
        assert_eq!(truncated[3].role, Role::Canvas);

        let declared = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        assert_eq!(
            declared.admit(truncated).err(),
            Some(Escape::Vocabulary(Defect::EmptyCanvas(SEQUENCE)))
        );
        assert_eq!(canvas_escapes(truncated), 1);

        // The vocabulary is checked before anything here, so what comes back is
        // the tree's defect and not this module's — even though the same
        // declaration would also have been refused for placing nothing. First
        // cause, not first consequence.
        assert_ne!(declared.admit(truncated).err(), Some(Escape::NothingPlaced(SEQUENCE)));
    }

    #[test]
    fn a_description_is_refused_rather_than_truncated() {
        // A description that stops halfway says the content ends where the
        // buffer did, which is a lie a screen reader would repeat faithfully —
        // the same reason `Text::new` refuses rather than truncating.
        let tree = timeline();
        let declared = arrangement();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");

        assert_eq!(canvas.phrases(), 10, "the canvas, three lanes, five clips and a marker");
        let mut cramped = [Phrase::UNSAID; 4];
        assert_eq!(canvas.linearise(&mut cramped).err(), Some(Escape::NoRoom));

        let mut said = [Phrase::UNSAID; 10];
        assert_eq!(canvas.linearise(&mut said), Ok(10));
        assert_eq!(said[0].node, SEQUENCE, "the canvas says itself first");
        assert_eq!(said[0].depth, 0);
        assert_eq!(said[0].when, Spoken::Untimed);
    }

    #[test]
    fn what_may_be_invoked_is_asked_of_a_canvas_and_not_of_a_node() {
        // The *scriptable* half, held to the scope it claims. `Operable::of` is
        // a two-field read of a node that knows nothing about any canvas, and
        // while it was public this module published a way to ask what may be
        // invoked on a node no canvas declared — which is the answer `operable`
        // exists to refuse, offered under another name thirty lines above the
        // refusal. It is private now, so every route to an `Operable` runs
        // through a canvas, and the node below is what that is worth.
        let tree = two_canvases();
        let declared = one_of_two();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");

        // A clip of the other canvas: right role, enabled, carrying an intent —
        // so a scope-blind reading of it answers *invocable*, and the refusal
        // below is the scope check rather than the node having nothing to say.
        let elsewhere = find(&tree, REEL_CLIP).expect("the other canvas declares it");
        assert_eq!(elsewhere.role, Role::Clip);
        assert_eq!(elsewhere.intent, Some(REEL_INTENT));
        assert!(elsewhere.state.holds(StateSet::ENABLED));
        assert!(!canvas.declares(REEL_CLIP));
        assert_eq!(canvas.operable(REEL_CLIP), Err(Escape::NotOfThisCanvas(REEL_CLIP)));
        assert_eq!(canvas.operable(REEL_LANE), Err(Escape::NotOfThisCanvas(REEL_LANE)));
        assert_eq!(canvas.operable(REEL), Err(Escape::NotOfThisCanvas(REEL)));

        // And the canvas that does declare it answers, which is what makes the
        // refusals above about standing rather than about this module being
        // unable to say anything. Both canvases are in one tree, so nothing but
        // the scope separates the two answers.
        let reel_declared = Arrangement::declaring(REEL, TimeBase::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(REEL_CLIP, span_x10(0, 20)))
            .expect("room for one");
        let reel = reel_declared.admit(&tree).expect("the other canvas declared its content too");
        assert_eq!(reel.operable(REEL_CLIP), Ok(Operable::Invocable(REEL_INTENT)));
        assert_eq!(reel.operable(AMBIENCE), Err(Escape::NotOfThisCanvas(AMBIENCE)));

        // The same on the exit's own surface: the transport's play button is
        // enabled and carries an intent, and the canvas beside it still will not
        // answer for it.
        let surface = timeline();
        let whole = arrangement();
        let sequence = whole.admit(&surface).expect("the canvas participates");
        let play = find(&surface, PLAY).expect("the transport declares it");
        assert_eq!(play.intent, Some(PLAY_INTENT));
        assert!(play.state.holds(StateSet::ENABLED));
        assert_eq!(sequence.operable(PLAY), Err(Escape::NotOfThisCanvas(PLAY)));
    }

    #[test]
    fn a_lane_of_another_canvas_is_refused_rather_than_answered_empty() {
        // `declares` says it is the scope of every question here, and for one
        // round it was the scope of exactly one: the three queries over a lane
        // filtered on *this lane* and never asked whose lane it was, so another
        // canvas's track, a command and a stranger all answered `Sole::Nothing`
        // — the empty answer `Sole` exists to keep apart from a refusal, handed
        // back for a question this canvas has no standing over.
        let tree = two_canvases();
        let declared = one_of_two();
        let canvas = declared.admit(&tree).expect("a canvas that declared its content");
        let span = span_x10(0, 20);

        // The other canvas's lane carries a clip over exactly this stretch, so
        // *nothing is there* would have been wrong twice over: not this canvas's
        // question, and not true either.
        let refused = Err(Escape::NotALane(REEL_LANE));
        assert_eq!(canvas.occupant_at(REEL_LANE, seconds_x10(10)), refused);
        assert_eq!(canvas.occupant_between(REEL_LANE, span), refused);
        match canvas.occupants_over(REEL_LANE, span) {
            Err(escape) => assert_eq!(escape, Escape::NotALane(REEL_LANE)),
            Ok(_) => panic!("a canvas enumerated another canvas's lane"),
        }
        assert_eq!(canvas.lane_of(REEL_CLIP), Err(Escape::NotOfThisCanvas(REEL_CLIP)));

        // A node of this canvas that is not a lane, and a node of no tree at
        // all. Both are refused by the same word, because *what is on this lane*
        // has no answer without a lane.
        let stranger = NodeId::new(999);
        assert_eq!(canvas.occupant_at(AMBIENCE, seconds_x10(10)), Err(Escape::NotALane(AMBIENCE)));
        assert_eq!(canvas.occupant_at(stranger, seconds_x10(10)), Err(Escape::NotALane(stranger)));

        // And this canvas's own lane answers — including answering *nothing*,
        // which is now a different word from the refusals above.
        assert_eq!(canvas.occupant_at(DIALOGUE, seconds_x10(10)), Ok(Sole::One(AMBIENCE)));
        assert_eq!(canvas.occupant_at(DIALOGUE, seconds_x10(30)), Ok(Sole::Nothing));
    }

    #[test]
    fn a_canvas_may_not_hang_its_content_where_the_dimension_cannot_reach() {
        // Clause 4 counts clips and markers, and the vocabulary does not make a
        // canvas's children be clips and markers: `Role::List` sits under any
        // parent that is not a root. So the cheapest escape left after the
        // zero-occupant one was to place a token clip, meet every clause, and
        // declare the content the canvas actually draws as a list hanging off
        // the canvas — placed nowhere, asked nothing, counted by nothing.
        let smuggled = content_smuggled();
        check(&smuggled).expect("the vocabulary is satisfied: a list may sit anywhere");
        assert_eq!(canvas_escapes(&smuggled), 0, "and its own metric sees nothing wrong");

        let declared = one_of_two();
        assert_eq!(declared.admit(&smuggled).err(), Some(Escape::Unplaceable(TAKES)));

        // The same declaration over the same tree without the smuggled subtree
        // is a participant, so the refusal is about what was hung under the
        // canvas and not about anything else in the declaration.
        assert!(declared.admit(&smuggled[..4]).is_ok());

        // And it is refused at any depth, because a caption under a clip is the
        // same escape one level further down.
        let deeper = [
            smuggled[0],
            smuggled[1],
            smuggled[2],
            smuggled[3],
            Node::new(CAPTION, AMBIENCE, Role::Label).with_content(text("ambience")),
        ];
        check(&deeper).expect("a label under a clip is a tree the vocabulary accepts");
        assert_eq!(declared.admit(&deeper).err(), Some(Escape::Unplaceable(CAPTION)));
    }

    #[test]
    fn a_count_in_sits_left_of_the_origin() {
        // `Ticks` is signed and the module argues for it at length: a count-in,
        // a pre-roll and a clip dragged left of zero are all real. An argument
        // nothing exercises is an argument, so here is the count-in — and with
        // it the case the conversions hide, which is that Rust's `/` and `%`
        // both truncate toward zero and `seconds_from_ticks` sums the two terms.
        let tree = timeline();
        let mut counted_in = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
        for placement in arrangement().placements() {
            let placement = if placement.occupant == AMBIENCE {
                Placement::over(AMBIENCE, span_x10(-20, 0))
            } else {
                placement
            };
            counted_in = counted_in.with_placement(placement).expect("within PLACED_MAX");
        }
        let canvas = counted_in.admit(&tree).expect("content before zero is content");

        assert_eq!(canvas.occupant_at(DIALOGUE, seconds_x10(-10)), Ok(Sole::One(AMBIENCE)));
        assert_eq!(canvas.occupant_between(DIALOGUE, span_x10(-20, -10)), Ok(Sole::One(AMBIENCE)));
        let at_origin = canvas.occupant_at(DIALOGUE, Ticks::ORIGIN);
        assert_eq!(at_origin, Ok(Sole::Nothing), "half-open on this side of zero too");

        // The conversion runs both ways on both sides of the origin.
        let before = seconds_x10(-20);
        assert_eq!(before.count(), -1_411_200_000, "two seconds of flicks, left of zero");
        let back = TimeBase::FLICKS.seconds_from_ticks(before, SPOKEN_DECIMALS);
        assert_eq!(back, Some(Quantity::scaled(-2_000, 3, Unit::Seconds)));
        let there = TimeBase::FLICKS.ticks_from_seconds(-4_200, 3);
        assert_eq!(there, Some(Ticks::new(-2_963_520_000)), "and -4.2 s is a tick count");

        // Toward zero, on the side where that is not the same as downward. This
        // is the one place a sign bug hides, because both terms truncate and are
        // then summed.
        let ragged = Ticks::new(before.count() + 1);
        assert_eq!(
            TimeBase::FLICKS.seconds_from_ticks(ragged, SPOKEN_DECIMALS),
            Some(Quantity::scaled(-1_999, 3, Unit::Seconds)),
            "a tick short of -2 s says -1.999 s and never -2.000 s"
        );

        // `Extent::duration` had no test at all, and a stretch that crosses the
        // origin is where a subtraction that lost the sign would show.
        let across = span_x10(-20, 42);
        assert_eq!(across.duration(), Ticks::new(4_374_720_000), "6.2 s of flicks");

        // And a reader is told the count-in is before zero, rather than being
        // told it starts at the origin like everything else.
        let mut said = [Phrase::UNSAID; 16];
        let count = canvas.linearise(&mut said).expect("room enough to say it");
        let hush = said[..count].iter().find(|phrase| phrase.node == AMBIENCE).expect("said");
        assert_eq!(
            hush.when,
            Spoken::Between(
                Quantity::scaled(-2_000, 3, Unit::Seconds),
                Quantity::scaled(0, 3, Unit::Seconds)
            )
        );
    }

    /// Every way this module refuses, produced by doing the thing that causes
    /// it.
    ///
    /// The match is exhaustive and that is the point of it: a sixteenth
    /// [`Escape`] does not compile until somebody writes the arm that produces
    /// one, which is the difference between a closed set of refusals and a list
    /// of words. Three of these had no test at all before — the bound RFC 0078's
    /// *what it makes hard* section is about, a conversion guard whose doc
    /// comment was wrong by nine orders of magnitude, and a refusal that did not
    /// exist yet — and each was unreachable in a way nothing failed over.
    ///
    /// Two ways of defeating it are worth naming, because the sentence here used
    /// to name only the first. One is an arm that hands back its argument
    /// without doing anything, which is visible in a diff in a way an absent
    /// test is not. The other was cheaper and was open: the list of escapes to
    /// walk was a hand-written array beside this function, and a new variant
    /// forced an arm here and forced no row there — so the arm became dead code
    /// and the suite stayed green. `after` is that list now, and it is a match
    /// on the argument too.
    fn produced(escape: Escape) -> Escape {
        let tree = timeline();
        let full = arrangement();
        match escape {
            Escape::NotATimeBase => TimeBase::new(0).expect_err("zero names nothing"),
            Escape::NotAnExtent => {
                Extent::new(seconds_x10(42), seconds_x10(42)).expect_err("a point is not a stretch")
            }
            Escape::TooManyPlacements => a_full_screen()
                .with_placement(Placement::over(NodeId::new(2_000), span_x10(0, 1)))
                .expect_err("the sixty-fifth does not fit"),
            Escape::PlacedTwice(_) => full
                .with_placement(Placement::over(DOOR, span_x10(0, 10)))
                .expect_err("one occupant, one place"),
            Escape::NotACanvas(_) => {
                let group = Arrangement::declaring(TRANSPORT, TimeBase::FLICKS, Selection::NOTHING);
                group.admit(&tree).expect_err("a group is not a canvas")
            }
            Escape::NotALane(_) => {
                let elsewhere = full.with_selection(Selection::over(span_x10(0, 10)).on(PLAY));
                elsewhere.admit(&tree).expect_err("a command is not a lane")
            }
            Escape::NotAnOccupant(_) => {
                let lane_placed = full
                    .with_placement(Placement::over(DIALOGUE, span_x10(0, 10)))
                    .expect("within PLACED_MAX");
                lane_placed.admit(&tree).expect_err("a lane is not placed in the dimension")
            }
            Escape::Mistimed(_) => {
                let mut mistimed =
                    Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
                for placement in full.placements() {
                    let placement = if placement.occupant == DOOR {
                        Placement::at(DOOR, seconds_x10(42))
                    } else {
                        placement
                    };
                    mistimed = mistimed.with_placement(placement).expect("within PLACED_MAX");
                }
                mistimed.admit(&tree).expect_err("a clip is not an instant")
            }
            Escape::Unplaced(_) => {
                let mut incomplete =
                    Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
                for placement in full.placements().filter(|placed| placed.occupant != DOOR) {
                    incomplete = incomplete.with_placement(placement).expect("within PLACED_MAX");
                }
                incomplete.admit(&tree).expect_err("one occupant placed nowhere")
            }
            Escape::NothingPlaced(_) => {
                let silent = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
                silent.admit(&tree).expect_err("a canvas that placed nothing")
            }
            Escape::Unplaceable(_) => {
                let smuggled = content_smuggled();
                let token = one_of_two();
                token.admit(&smuggled).expect_err("a list hung under a canvas")
            }
            Escape::NotOfThisCanvas(_) => full
                .admit(&tree)
                .expect("the canvas participates")
                .operable(PLAY)
                .expect_err("a node of somebody else's surface"),
            Escape::NoRoom => {
                let canvas = full.admit(&tree).expect("the canvas participates");
                let mut cramped = [Phrase::UNSAID; 1];
                canvas.linearise(&mut cramped).expect_err("ten phrases do not fit in one")
            }
            Escape::Unsayable(_) => {
                let coarse = coarse_tree();
                let declared = coarse_arrangement();
                let canvas = declared.admit(&coarse).expect("the floor is met");
                let mut said = [Phrase::UNSAID; 4];
                canvas.linearise(&mut said).expect_err("a time no description can hold")
            }
            Escape::Vocabulary(_) => {
                let bare = Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING);
                bare.admit(&tree[..4]).expect_err("a canvas with no children")
            }
        }
    }

    /// The escape after this one, or `None` at the end of the list.
    ///
    /// The list `produced` is walked over, written as a match on the argument
    /// so that adding a variant forces an arm here as well as there. What it
    /// still cannot force is that the new arm splices the variant into the chain
    /// rather than ending it — but that arm is one token, written next to the
    /// arm that produces the escape, rather than a row in an array somebody has
    /// to remember exists.
    fn after(escape: Escape) -> Option<Escape> {
        match escape {
            Escape::NotATimeBase => Some(Escape::NotAnExtent),
            Escape::NotAnExtent => Some(Escape::TooManyPlacements),
            Escape::TooManyPlacements => Some(Escape::PlacedTwice(DOOR)),
            Escape::PlacedTwice(_) => Some(Escape::NotACanvas(TRANSPORT)),
            Escape::NotACanvas(_) => Some(Escape::NotALane(PLAY)),
            Escape::NotALane(_) => Some(Escape::NotAnOccupant(DIALOGUE)),
            Escape::NotAnOccupant(_) => Some(Escape::Mistimed(DOOR)),
            Escape::Mistimed(_) => Some(Escape::Unplaced(DOOR)),
            Escape::Unplaced(_) => Some(Escape::NothingPlaced(SEQUENCE)),
            Escape::NothingPlaced(_) => Some(Escape::Unplaceable(TAKES)),
            Escape::Unplaceable(_) => Some(Escape::NotOfThisCanvas(PLAY)),
            Escape::NotOfThisCanvas(_) => Some(Escape::NoRoom),
            Escape::NoRoom => Some(Escape::Unsayable(PLAYHEAD)),
            Escape::Unsayable(_) => Some(Escape::Vocabulary(Defect::EmptyCanvas(SEQUENCE))),
            Escape::Vocabulary(_) => None,
        }
    }

    #[test]
    fn every_escape_is_producible_and_names_what_it_is_about() {
        let mut walking = Some(Escape::NotATimeBase);
        let mut walked = 0;
        while let Some(escape) = walking {
            assert_eq!(produced(escape), escape, "{escape:?} is reachable, and names its node");
            walked += 1;
            assert!(walked <= PLACED_MAX, "`after` is a chain and it has become a loop");
            walking = after(escape);
        }
        // The one hand-written number left. It is what refuses a chain that was
        // shortened; it is not what would catch a variant spliced nowhere, and
        // nothing on stable is, which is why `after` puts that arm next to the
        // arm that produces the escape.
        assert_eq!(walked, 15, "every variant the enum has, walked once");
    }
}
