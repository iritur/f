// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The semantic node vocabulary, version 1.
//!
//! `docs/design/ring-scene-boot.html` section 11 gives the struct and four
//! rules that are load-bearing rather than stylistic. This module is that
//! struct, those four rules made into types, and the refusal that the exit of
//! `E3-D01` asks for: **there is no role meaning *other***, and there is no
//! spelling of one. RFC 0077 is the argument; this is the artefact.
//!
//! # Why the vocabulary is closed, and what closed costs
//!
//! [`Role`] is an ordinary Rust enum with twenty-two variants and it is
//! deliberately **not** `#[non_exhaustive]`. That attribute is the escape hatch
//! arriving through the back door: it forces every projection ever written to
//! carry a wildcard arm, and a wildcard arm is where a node nobody understood
//! goes to be presented as a grey box. A closed enum means a projection that
//! compiles has decided what to do with every role in the vocabulary, and a
//! twenty-third role is a change that every projection's author is told about
//! by their compiler rather than by a user.
//!
//! The cost is real and is stated so nobody has to discover it: an application
//! with a genuinely novel control cannot invent a role. It must argue for one,
//! in an RFC, against a vocabulary that will say no most of the time. Section
//! 13 says that is the correct bias — the failure that overtook accessibility
//! annotation on the web was not too few roles, it was roles whose meanings
//! nobody agreed on, which is what an open vocabulary converges to.
//!
//! **Inside this crate, closedness rests on there being one list.** The
//! `vocabulary!` invocation below is the only place a role is written: the
//! enum, [`Role::ALL`], [`Role::COUNT`], the index, the name, the family, the
//! operability and the parent rule are all emitted from it. Two earlier
//! versions of this module kept `ALL` and `COUNT` by hand and tried to catch a
//! rogue variant with a test — first a loop, which cannot see what a list
//! omits, then an exhaustive match, which demands an arm and not an arm that
//! says anything. Neither worked, both are recorded where they were written,
//! and what replaced them is not a better test. It is the absence of a second
//! place to write a role.
//!
//! # Why twenty-two
//!
//! Section 13 states the two failure directions and does not give a number:
//! too small and applications escape into canvas, too large and the meanings go
//! ambiguous. Twenty is what the three interfaces in this module's tests cost —
//! a settings panel, a file browser, a timeline. Each puts a different pressure
//! on the vocabulary: the settings panel is where *layout is constraints* and
//! *style is tokens* are most tempting to break, the file browser punishes a
//! vocabulary with no honest notion of a column, and the timeline is the canvas
//! case. **Only the third is chosen for breaking a predecessor**, and an
//! earlier version of this sentence said all three were — section 13 says the
//! canvas sank every predecessor and says nothing of the kind about the other
//! two, so the stronger claim was this module's invention. The two roles in
//! reserve, [`Role::List`] and [`Role::Text`], are the ones a fourth interface
//! reaches for immediately: a menu is a list of operable items, and a document
//! is paragraphs.
//!
//! **The criterion is not how many of the three needed a role, and an earlier
//! version of this paragraph said it was.** It said every role only one of the
//! three needed was refused, which the code two hundred lines below contradicts:
//! thirteen of the twenty are reached by exactly one tree, three by two, and
//! four by all three. That census is not an embarrassment, it is what three
//! interfaces chosen for three *different* pressures produce — mostly disjoint
//! parts of the vocabulary over a common spine of four roles — and
//! `the_three_interfaces_reach_most_of_the_vocabulary` now asserts all four
//! numbers so the prose cannot drift away from them again. The criterion that
//! was actually applied is RFC 0077's: *can a projection do something with this
//! role that it could not do without it*. What that refuses is the role which
//! **reduces** — an appearance of a role already here — and every refusal in the
//! next paragraph is of that kind.
//!
//! The economies are worth naming, because they are where a fourth interface
//! will push back. A *link* is a [`Role::Command`] whose intent is a navigation
//! capability. A *progress bar* is a [`Role::Number`] with no intent — read-only
//! is what `intent: None` already means, so a second role for it would be one
//! fact spelled twice. An *icon* is a [`Role::Image`], or more often a style
//! token on the node it decorates. A *toolbar* is a [`Role::Group`] whose
//! children are commands, which a projection can see for itself. A *menu* is a
//! [`Role::List`] of operable [`Role::Item`]s. Each of those would be a role
//! here if the criterion were *does a toolkit have a name for it* rather than
//! *can a projection do something with it that it could not do without it*.
//!
//! **How the number is judged later is not by argument.** Section 13 converts
//! the question into a measurement: the **canvas-escape rate** across a corpus
//! of real applications, where every escape into [`Role::Canvas`] is read as a
//! bug report about the vocabulary rather than as an application's choice. It is
//! registered as `claims/0034-canvas-escape-rate.toml`, at one node in twenty,
//! as a **target** — the rate has never been measured, nothing in `E3` has, and
//! that file says so in its own first paragraph rather than leaving a reader to
//! infer it from a threshold.
//!
//! Two functions here relate to it and they count different things, which is
//! worth stating because the first version of this comment ran them together.
//! [`canvas_census`] is the per-tree half of the rate: how many nodes are
//! canvases, over how many nodes there are. [`canvas_escapes`] counts something
//! narrower — canvases that declared *nothing* — which is the floor being
//! evaded rather than the rate being high, and is zero for any tree that passed
//! [`check`]. The corpus the rate is actually taken over is owed by `E3-B06l`,
//! which the claim names.
//!
//! # The four rules, and where each is enforced
//!
//! **Intent is a capability reference, not a callback.** [`Node::intent`] is an
//! `Option<`[`CapRef`]`>` and there is nowhere in this module to put a function
//! pointer. That is what lets a projection describe an action without the author
//! writing a label, check whether a caller may invoke it, and log it.
//!
//! **Layout is constraints, never coordinates.** [`Constraints`] carries an
//! intrinsic size range, a growth share and a [`Flow`], and there is no field
//! anywhere in this module that could hold an *x*. There is no partial version
//! of this rule: one escape hatch and every projection except the local one
//! degrades silently, which is a failure nobody sees on the machine they wrote
//! the application on.
//!
//! **Style is a token set, never values.** [`TokenSet`] holds token *names*.
//! What a name resolves to is `token.rs`'s business — `E3-D03` owns the typed
//! token layer, its ranges, and what a hostile theme cannot break — and this
//! module deliberately cannot express a colour, a length or an opacity. The one
//! place somebody would smuggle one in is the name itself, so [`TokenName`]
//! refuses a name spelled like a value.
//!
//! **Identity is author-assigned and survives redesign.** [`NodeId`] is chosen
//! by the application and derived from nothing — not position, not role, not
//! content, none of which survives a redesign. This is what makes tests and
//! automation non-brittle, and it is also the join to every other module in this
//! crate: `canvas.rs` attaches an arrangement's time ranges to clips **by
//! `NodeId`**, and not by a field this struct grew. A vocabulary extended by
//! widening its node is a vocabulary that is not closed.
//!
//! # What this module is not
//!
//! It is not a wire format. A [`Node`] is some hundreds of bytes because it owns
//! its text and its tokens inline, which is the right shape for something that
//! is *read* — by a test, by a projection, by somebody arguing with it — and the
//! wrong shape for something that is *sent*. The day the tree crosses a ring,
//! the entry form belongs in `f_abi`, carrying text by reference into the inline
//! arena exactly as `Sqe` does, and this crate keeps the meaning.

/// The longest text a node may carry, in bytes of UTF-8.
///
/// **Nothing in the corpus pushed on this, and the first version of this comment
/// claimed one case did.** The longest string the three interfaces below produce
/// is `quarterly-report.pdf`, at twenty bytes; they would not notice sixty-four.
/// So this is not a bound derived from evidence, and a reader should not treat
/// it as one.
///
/// What it is instead is a bound on **what a node costs**, which is a real
/// constraint and the one this number was actually chosen against: [`Node`] owns
/// its text inline, so every node in every tree pays for the longest one any
/// node may carry. That is why it is not a POSIX component's 255. It is equally
/// why it is not the corpus's twenty with a margin: a bound fitted to three
/// examples is a bound the fourth interface breaks, and breaking it here means
/// refusing a name a real application has — this refuses rather than truncates,
/// because a truncated name in a *declaration* is a lie that every projection
/// then repeats faithfully.
///
/// **What would reverse it.** Either end, and each has a different answer. A
/// real corpus producing names that do not fit: the answer is the inline arena
/// the wire form needs anyway, not a larger array here. Or a [`Node`] that has
/// become too expensive to copy, at which point this is the first field to cut
/// and the arena is again the answer. Nothing between those two observations is
/// evidence about this number, and no test in this module asserts it is right.
pub const TEXT_MAX: usize = 192;

/// The longest token name, in bytes.
pub const TOKEN_NAME_MAX: usize = 24;

/// The most style tokens one node may wear.
///
/// Four. **The count under this sentence has now been wrong twice, in opposite
/// directions, so here it is as a test rather than as a recollection.** It said
/// no node in the three interfaces wears more than two; the maximum is **one**.
/// `settings_panel` is the only one of the three that styles anything at all —
/// four nodes, one token each — and `file_browser` and `timeline` contain no
/// `with_style` call. The two-token nodes are in `settings_panel_redesigned`,
/// which is the redesign fixture and is in no corpus any test counts. So this
/// bound is four times what the corpus needed, not twice, and it is not a
/// measured bound in any direction. `the_token_bound_is_four_times_the_corpus`
/// asserts all three numbers, so the next reader of this paragraph gets a
/// failing test rather than a third version of the sentence.
///
/// The argument for having a bound at all is the part worth keeping: a node
/// wearing many tokens is a node whose meaning is being assembled out of style,
/// which is exactly what a token set exists to prevent, and a design that needs
/// five is a design that wants a role. Where the line falls between four and
/// eight is a judgement nothing here has tested.
///
/// **What would reverse it:** a design system whose ordinary node genuinely
/// wears five. The question then is whether `token.rs` (`E3-D03`) has a
/// composition this set is standing in for — one token that means what five
/// were spelling — because that is the layer that owns what names resolve to,
/// and a larger array here would answer the symptom.
pub const TOKENS_MAX: usize = 4;

/// The most relations one node may declare.
///
/// There are exactly four relation kinds, and a node needing more than four
/// edges is a node doing more than one job.
///
/// **The second reason written here was borrowed and false**: that the bound is
/// what lets [`check`] resolve every edge without a set to remember things in.
/// It is not. `check` resolves an edge with `find`, a linear scan run once per
/// edge, and that needs no bound at any number of relations —
/// [`DEPTH_MAX`]'s analogous sentence is true and this one was copied from it.
/// The real second reason is duller: [`Relations`] is a fixed-size array in a
/// crate with no allocator, so *some* number has to be written, and this is the
/// number. `a_fifth_relation_is_refused` is where the refusal is exercised.
pub const RELATIONS_MAX: usize = 4;

/// The deepest a declared tree may nest.
///
/// Sixteen. Deeper is a structure no reader holds in their head and no screen
/// reader linearises usefully, and the bound is also how [`check`] finds a cycle
/// without a visited set: the parent walk terminates against it.
pub const DEPTH_MAX: usize = 16;

/// Why a declaration was refused at construction.
///
/// Every refusal here is a fixed capacity exceeded or a value smuggled into a
/// name. None of them is a condition a caller should recover from by trying
/// something slightly different — each says the declaration was wrong, which is
/// why they are refused where they are written rather than discovered where they
/// are projected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// Text longer than [`TEXT_MAX`] bytes. Not truncated: see that constant.
    TextTooLong,
    /// A token name that is empty, longer than [`TOKEN_NAME_MAX`], or not
    /// spelled as a name. See [`TokenName::new`] for the grammar, and for why
    /// the grammar is where *style is never values* is actually enforced.
    NotATokenName,
    /// A fifth relation. See [`RELATIONS_MAX`].
    TooManyRelations,
    /// A fifth style token. See [`TOKENS_MAX`].
    TooManyTokens,
}

/// A node's identity: author-assigned, stable, and outliving every redesign.
///
/// # Why the author assigns it and the system does not
///
/// A system-assigned identity would be derived from something — insertion order,
/// a path through the tree, a hash of the content — and every one of those is a
/// thing a visual redesign changes. The whole value of identity here is that
/// `E3-P05`'s exit can be met: a test asserts against a node, somebody moves
/// that node into a different panel, and the test still names the same thing.
/// That property cannot be bought from a derivation.
///
/// The cost is that identity becomes the application's responsibility and a
/// duplicate becomes a defect rather than an impossibility. [`check`] reports
/// it, which is the trade taken deliberately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeId(u64);

impl NodeId {
    /// Names no node.
    ///
    /// The parent of a root, and a value no relation may carry — a relation to
    /// nothing is a relation somebody meant to fill in. That second half is
    /// enforced by one rule rather than asked for by two: no node may *hold*
    /// this identity, so nothing resolves it, and an edge carrying it is a
    /// [`Defect::UnknownRelation`].
    pub const UNNAMED: Self = Self(0);

    /// The identity an author chose.
    ///
    /// **This does not refuse zero, and this sentence used to say it did.** It
    /// said a tree using zero as an identity fails [`check`] at its first
    /// parent walk. It did not: a node whose *parent* is unnamed is a root, so
    /// the walk ran zero times, and `[Node::new(NodeId::UNNAMED,
    /// NodeId::UNNAMED, Role::Surface)]` passed clean — after which `find`
    /// answered that node for every edge that meant *nobody*.
    ///
    /// A constructor that cannot fail is the right shape for a `const fn` an
    /// author writes inside an array literal, so the refusal is where the tree
    /// is read rather than where the number is written: [`check`] rejects a
    /// node that names nothing with [`Defect::Unnamed`].
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The underlying value, for a module that keys a side table off it — which
    /// is how everything else in this crate joins to this one.
    #[must_use]
    pub const fn value(self) -> u64 {
        self.0
    }

    /// Does this name a node at all?
    #[must_use]
    pub const fn is_named(self) -> bool {
        self.0 != Self::UNNAMED.0
    }
}

/// A reference to a capability: what a control *does*, stated so the system can
/// read it.
///
/// # Why this is not `f_abi::cap::Handle`, yet
///
/// Two reasons, and the second is the one that matters. The first is the
/// crate's: `interface/Cargo.toml` takes no dependencies so that this layer can
/// be abandoned without taking anything else down, and depending on the wire
/// crate is depending on the wire.
///
/// The second is that a handle is an index into *one component's* capability
/// table and means nothing outside that component. The tree is owned by the
/// system and read by projections that are not the declaring component — a
/// screen reader, an agent, a remote client — so what a node carries has to be
/// something the *owner of the tree* can resolve. Today nothing resolves it,
/// because nothing here is on a ring; the value is an opaque token an
/// application hands over, and this module only ever compares it for equality.
///
/// It is a `u32` deliberately, matching `f_abi::cap::Handle`'s width, so that
/// the day the tree crosses a ring the translation is a rename in one place
/// rather than a re-encoding in every projection. That day needs its own RFC,
/// because it is the day this crate stops being a leaf.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CapRef(u32);

impl CapRef {
    /// The reference an application handed over.
    #[must_use]
    pub const fn new(bits: u32) -> Self {
        Self(bits)
    }

    /// The underlying value, for whoever eventually resolves it.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }
}

/// Owned, bounded UTF-8: what a node says.
///
/// # Why a node owns its text instead of borrowing it
///
/// Because the system owns the tree. A node outlives the call that declared it,
/// and it outlives the application's own copy of whatever it was rendering; a
/// borrow would tie the lifetime of the whole semantic layer to the lifetime of
/// the frame that produced one label. The price is [`TEXT_MAX`] and a node that
/// is large to copy, which is the trade this module is willing to make because
/// it optimises for being read rather than for being sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Text {
    /// The bytes, zero-filled past `len` so that equality compares meaning
    /// rather than whatever the tail happened to hold.
    bytes: [u8; TEXT_MAX],
    /// How many of them are the text.
    len: u8,
}

impl Text {
    /// Nothing said.
    pub const EMPTY: Self = Self { bytes: [0; TEXT_MAX], len: 0 };

    /// Copy `text`, or refuse it whole.
    ///
    /// # Errors
    ///
    /// [`Refused::TextTooLong`] when `text` exceeds [`TEXT_MAX`] bytes. The
    /// refusal is the point: see that constant for why this does not truncate.
    pub const fn new(text: &str) -> Result<Self, Refused> {
        let src = text.as_bytes();
        if src.len() > TEXT_MAX {
            return Err(Refused::TextTooLong);
        }
        let mut bytes = [0u8; TEXT_MAX];
        let mut at = 0;
        while at < src.len() {
            bytes[at] = src[at];
            at += 1;
        }
        Ok(Self { bytes, len: src.len() as u8 })
    }

    /// What it says.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The error arm is unreachable: the only constructor copies a `&str`
        // whole, so the prefix is always a run of whole code points. It answers
        // with the empty string rather than panicking, because that is the right
        // failure for a library a projection calls once per node per frame.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or_default()
    }

    /// Bytes, not code points — the only honest count for a bounded buffer.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Does this node say nothing?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// What a [`Quantity`] counts.
///
/// Closed, and short for the reason [`Role`] is: a unit a projection does not
/// understand is a number it can only print, and printing a number is what this
/// layer exists to stop being the only option. Four is what the three interfaces
/// needed — a count of things, a ratio for a volume, a size in bytes, a position
/// in seconds — and a fifth needs the argument a fifth role needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// A count of things. The scale is always zero for a count: half an item is
    /// not a quantity, it is a mistake.
    Count,
    /// A proportion, where ten to the power of `scale_decimals` is the whole. A
    /// volume, a completion, a zoom.
    Ratio,
    /// Bytes. Not kibibytes and not a rendered string — how a size is *shown*
    /// is the projection's decision and depends on how much room it has.
    Bytes,
    /// Seconds, scaled. The unit the timeline's times are in, and therefore the
    /// unit `canvas.rs` reads them back in.
    Seconds,
}

/// A number, with its scale beside it, because this tree has no floating-point
/// types.
///
/// # Why the scale is a field and not part of a name
///
/// `CLAUDE.md` asks for a fixed point with its scale in the name —
/// `contrast_x1000`, `cost_us_x100` — and that rule works because whoever names
/// the field knows what is being counted. This module does not: the vocabulary
/// carries an application's numbers, and the application picks the scale. So the
/// scale travels with the value, as data. This is the one place in the crate
/// where that convention cannot apply, and saying so here is cheaper than having
/// the argument again at review.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quantity {
    /// The value, multiplied by ten to the power of `scale_decimals`.
    pub scaled: i64,
    /// How many decimal places `scaled` carries. Zero means `scaled` is the
    /// value.
    pub scale_decimals: u8,
    /// What is being counted.
    pub unit: Unit,
}

impl Quantity {
    /// A whole number of `unit`.
    #[must_use]
    pub const fn whole(value: i64, unit: Unit) -> Self {
        Self { scaled: value, scale_decimals: 0, unit }
    }

    /// A value carrying `scale_decimals` decimal places.
    #[must_use]
    pub const fn scaled(scaled: i64, scale_decimals: u8, unit: Unit) -> Self {
        Self { scaled, scale_decimals, unit }
    }
}

/// A number a node holds, and what a caller is allowed to do to it.
///
/// # Why the bounds live in the content and not in the constraints
///
/// Because they are not layout. The range of a volume control is a fact about
/// the volume: true in a screen reader that renders no pixels, and true for an
/// agent that never looks at one. Putting it in [`Constraints`] would make the
/// only complete description of a control depend on the one thing this layer
/// promises is a projection's business.
///
/// `step` is here for the same reason and earns its place separately: without it
/// an agent asking for a value between two settable ones gets a silent rounding,
/// and *any value in the range* is a lie for every control that moves in
/// increments.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Reading {
    /// What it is now.
    pub value: Quantity,
    /// The smallest settable value, if there is one.
    pub low: Option<Quantity>,
    /// The largest settable value, if there is one.
    pub high: Option<Quantity>,
    /// The increment between settable values, if the control moves in steps.
    pub step: Option<Quantity>,
}

impl Reading {
    /// A reading with no declared bounds — a size, a duration, a count.
    #[must_use]
    pub const fn plain(value: Quantity) -> Self {
        Self { value, low: None, high: None, step: None }
    }

    /// A reading a caller may move between `low` and `high`.
    #[must_use]
    pub const fn bounded(value: Quantity, low: Quantity, high: Quantity) -> Self {
        Self { value, low: Some(low), high: Some(high), step: None }
    }

    /// The same, moving in increments of `step`.
    #[must_use]
    pub fn stepped(self, step: Quantity) -> Self {
        Self { step: Some(step), ..self }
    }
}

/// What a node holds. Section 11's *text | value | media | none*.
///
/// # Why media is a capability reference
///
/// For the reason [`Node::intent`] is one. A picture inlined into a declaration
/// is bytes the system must copy, store and version on every redeclaration, and
/// they are bytes no projection can ask a question about. A reference names an
/// object the system already knows how to describe, permit and revoke — and it
/// is the same mechanism twice rather than a second mechanism, which is the test
/// this vocabulary applies to everything it considers adding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// Nothing. The node's meaning is its role, its state and its children,
    /// which is the common case for every container in the vocabulary.
    None,
    /// Human-readable text.
    Text(Text),
    /// A number, with its unit and whatever bounds apply to it.
    Value(Reading),
    /// Pixels or samples the application holds elsewhere, named by capability.
    Media(CapRef),
}

/// The interaction states a node may be in.
///
/// Exactly the five section 11 names, in a `u8` with three bits spare. The spare
/// bits are not an invitation: a sixth state is a change to what every
/// projection must be able to present, and that is an RFC. Two states were
/// refused, and the refusals are worth keeping.
///
/// *Focused* is refused because focus belongs to the projection rather than to
/// the declaration — one tree projected simultaneously to a display, a screen
/// reader and an agent has three focuses, and a field here could only hold one
/// of them, wrongly.
///
/// *Hidden* is refused because a node that should not be presented should not be
/// declared, and because the decision is not the application's to make: a screen
/// reader may usefully present what a display elides. The one case that looks
/// like hiding — a collapsed subtree — is the absence of
/// [`EXPANDED`](Self::EXPANDED) on the node that collapsed it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StateSet(u8);

impl StateSet {
    /// No state stated. On a node that carries intent this means *not
    /// operable*, which is the safe default in the only direction that matters:
    /// the alternative default is *anything may be invoked*.
    pub const NONE: Self = Self(0);

    /// The node may be operated. Meaningful only where [`Node::intent`] is
    /// present, and ignored elsewhere.
    pub const ENABLED: Self = Self(1 << 0);
    /// The node is part of the current selection.
    pub const SELECTED: Self = Self(1 << 1);
    /// The node's children are being presented. Its absence on a node that has
    /// children is a collapsed subtree, not an empty one.
    pub const EXPANDED: Self = Self(1 << 2);
    /// The node is working, and what it shows may be stale.
    pub const BUSY: Self = Self(1 << 3);
    /// The node's content has been refused. A projection is expected to say so
    /// without the author writing the sentence.
    pub const INVALID: Self = Self(1 << 4);

    /// Every bit this version defines. A set carrying a bit outside this came
    /// from a version this build does not speak.
    pub const KNOWN: Self = Self(
        Self::ENABLED.0 | Self::SELECTED.0 | Self::EXPANDED.0 | Self::BUSY.0 | Self::INVALID.0,
    );

    /// This set with `other`'s bits added.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// This set with `other`'s bits removed.
    #[must_use]
    pub const fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// Does this set hold every bit of `other`?
    #[must_use]
    pub const fn holds(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    /// The raw bits, for a projection that indexes a table with them.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// A set from bits that came from somewhere this build did not write.
    ///
    /// The other half of [`bits`](Self::bits), and the reason
    /// [`is_known`](Self::is_known) can answer at all. Every other constructor
    /// here builds from this version's own constants, so until this existed no
    /// value reachable through the public API could carry a bit outside
    /// [`KNOWN`](Self::KNOWN), `is_known` could not return `false`, and a
    /// predicate that cannot return `false` is documentation wearing a
    /// function's clothes. The reviewer who found that was right, and the choice
    /// it forced was between deleting the predicate and supplying the
    /// constructor its own documentation already presupposed.
    ///
    /// It keeps every bit it is given, including the ones it does not
    /// understand, rather than masking against `KNOWN`. Masking would make the
    /// loss silent and leave `is_known` answering about a value nobody sent —
    /// the bits are evidence that the sender and this build disagree, and
    /// evidence is not tidied away by the layer that noticed it.
    ///
    /// What to *do* about an unknown bit is deliberately not decided here: that
    /// is vocabulary version negotiation, which RFC 0077 names as a reversal
    /// condition and `E3-B06a` owns. This exists so the question can be asked on
    /// the day there is a decoder, rather than answered now by whoever happens
    /// to write the first one.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Is every bit set here one this version defines?
    #[must_use]
    pub const fn is_known(self) -> bool {
        self.0 & !Self::KNOWN.0 == 0
    }
}

/// An edge between two nodes that the tree's shape does not already say.
///
/// Section 11 names four, and these are those four. Parenthood is not among them
/// because parenthood is [`Node::parent`]; these are the edges that cross the
/// tree rather than describe it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// Another node supplies this one's name. This is the mechanism behind *a
    /// screen reader describes a control without the author writing a label*:
    /// the label is already a node, and this says which.
    LabelledBy(NodeId),
    /// Operating this node changes that one. A mute toggle and the volume it
    /// disables; a column header and the ordering it sets.
    Controls(NodeId),
    /// This node is responsible for that one although the tree does not nest
    /// them — a menu a button opens, a panel a tab reveals.
    Owns(NodeId),
    /// Completing this node leads to that one. The declared form of *what
    /// happens next*, which is what makes a multi-step flow legible to an agent
    /// that cannot see the animation.
    FlowsTo(NodeId),
}

impl Relation {
    /// The node at the far end, whichever kind of edge this is.
    #[must_use]
    pub const fn target(self) -> NodeId {
        match self {
            Self::LabelledBy(id) | Self::Controls(id) | Self::Owns(id) | Self::FlowsTo(id) => id,
        }
    }
}

/// The relations one node declares, bounded by [`RELATIONS_MAX`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relations {
    /// The edges, oldest first. Slots past `len` are never read.
    items: [Option<Relation>; RELATIONS_MAX],
    /// How many are real.
    len: u8,
}

impl Relations {
    /// No edges.
    pub const EMPTY: Self = Self { items: [None; RELATIONS_MAX], len: 0 };

    /// This set with one more edge.
    ///
    /// # Errors
    ///
    /// [`Refused::TooManyRelations`] past [`RELATIONS_MAX`].
    pub const fn push(mut self, relation: Relation) -> Result<Self, Refused> {
        if self.len as usize >= RELATIONS_MAX {
            return Err(Refused::TooManyRelations);
        }
        self.items[self.len as usize] = Some(relation);
        self.len += 1;
        Ok(self)
    }

    /// How many edges this node declares.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Does it declare none?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every edge, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = Relation> + '_ {
        self.items[..self.len as usize].iter().copied().flatten()
    }
}

/// How a node's children are arranged relative to one another.
///
/// Four, and none of them is a coordinate system. Direction is deliberately
/// relative to the reading order rather than to a screen: a vocabulary that said
/// *horizontal* would be a vocabulary that had already decided the script runs
/// left to right, which is the same category of mistake as `x = 340` and is much
/// harder to see.
///
/// A grid flow was refused. A table's alignment is a property of the table — its
/// rows carry equal numbers of cells, which [`check`] enforces — so a grid flow
/// would be a second way to say what the vocabulary already says, and the second
/// way is the one that ends up used for things that are not tables.
///
/// Alignment was refused as well: *start*, *centre*, *stretch* resolve against a
/// theme and a writing direction, which makes them style tokens rather than
/// structure. What is left is size and order, which is all a solver needs from a
/// declaration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flow {
    /// Children follow one another along the reading direction.
    Inline,
    /// Children follow one another across it.
    Block,
    /// Inline, and the projection may break the run wherever it must.
    Wrap,
    /// The node arranges its own children and a solver must not. The honest
    /// answer for a leaf, and the *only* answer for a [`Role::Canvas`] — which
    /// is what a canvas is: an application that has taken back arrangement and
    /// nothing else.
    Own,
}

/// What a node needs in order to be useful, stated so a solver can decide what
/// it gets.
///
/// # Why sizes are in ems and not in pixels
///
/// A pixel is a property of a display; an em is a property of the resolved text
/// style. A size expressed in ems therefore survives projection to a different
/// density, a different theme, and a client that has never heard of this
/// machine's display. It is the move [`TokenSet`] makes for colour, applied to
/// length, and it is why this struct can carry a number at all without breaking
/// *layout is never coordinates*: a number here is a **requirement**, and a
/// requirement is not a position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Constraints {
    /// The smallest size at which this node is still useful, in hundredths of
    /// the resolved em, measured along the parent's flow. Zero makes no demand.
    pub min_em_x100: u16,
    /// The largest size worth giving it, same units.
    /// [`UNBOUNDED`](Self::UNBOUNDED) means *as much as there is*.
    pub max_em_x100: u16,
    /// Share of whatever space is left over, relative to siblings. Zero means
    /// this node takes none of it, which is right for everything except the one
    /// thing on a surface that should absorb the window.
    pub grow: u16,
    /// How this node's own children are arranged.
    pub flow: Flow,
}

impl Constraints {
    /// As much room as there is.
    pub const UNBOUNDED: u16 = u16::MAX;

    /// No demand, no growth, no arrangement: what a leaf declares.
    pub const LEAF: Self =
        Self { min_em_x100: 0, max_em_x100: Self::UNBOUNDED, grow: 0, flow: Flow::Own };

    /// A container arranging its children with `flow`, making no size demand of
    /// its own.
    #[must_use]
    pub const fn flowing(flow: Flow) -> Self {
        Self { min_em_x100: 0, max_em_x100: Self::UNBOUNDED, grow: 0, flow }
    }

    /// The same, but needing at least `min_em_x100`.
    #[must_use]
    pub fn at_least(self, min_em_x100: u16) -> Self {
        Self { min_em_x100, ..self }
    }

    /// The same, but wanting no more than `max_em_x100`.
    #[must_use]
    pub fn at_most(self, max_em_x100: u16) -> Self {
        Self { max_em_x100, ..self }
    }

    /// The same, but taking `grow` shares of the leftover.
    #[must_use]
    pub fn growing(self, grow: u16) -> Self {
        Self { grow, ..self }
    }
}

/// The name of a style token. What it resolves to is `token.rs`'s business.
///
/// # Why a name, and why the grammar refuses some of them
///
/// *Style is semantic tokens, never values* is a rule with exactly one hole in
/// it, and the hole is the name. Nothing stops an author writing a token called
/// `ff0000` or `12px` or `50%` and a theme obliging, at which point the rule is a
/// convention and the claim that a theme reaches applications its author never
/// saw is decoration. So the grammar is the enforcement: a name is lowercase
/// ASCII letters, digits, `-` and `.`, and it **must begin with a letter**, and
/// it **may not be spelled entirely from the hexadecimal alphabet**. Both
/// clauses refuse at construction rather than at projection, where the author is
/// no longer present to be told.
///
/// The second clause is there because the first one does not reach the value
/// spelling authors reach for most. RFC 0077 lists `ff0000`, `12px` and `50%` as
/// the three this grammar refuses, and *begins with a letter* refuses only the
/// last two: `ff0000` begins with `f`, which is a letter, and a colour literal
/// that happens to start in the first six letters of the alphabet would have
/// walked straight through a grammar written to keep colour literals out. A name
/// whose every byte is a hex digit is a value wearing a name's clothes, and there
/// is no second reading of it.
///
/// What it costs, said rather than discovered: a genuine word spelled only from
/// `a`-`f` and the digits — `facade`, `decade`, `added` — is refused too, and the
/// author has to spell it differently. That is the whole of the collateral, and
/// it is accepted because a token vocabulary that needs one of those words has a
/// naming problem the grammar is not the right place to solve. **What would
/// reverse this clause:** a real application whose design system needs such a
/// word as a token name, which would mean the rule is obstructing naming rather
/// than enforcing the pillar, and the replacement is a check against the shapes
/// of an actual colour literal — three, four, six or eight digits — rather than
/// against the alphabet.
///
/// `E3-D03` owns what the names mean, which ranges are checked, and what a
/// hostile theme cannot do with them. This type exists so the two modules can
/// agree on one minimal shared thing without either waiting on the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenName {
    /// The bytes, zero-filled past `len`.
    bytes: [u8; TOKEN_NAME_MAX],
    /// How many of them are the name.
    len: u8,
}

impl TokenName {
    /// Check `name` against the grammar and keep it.
    ///
    /// # Errors
    ///
    /// [`Refused::NotATokenName`] for a name that is empty, longer than
    /// [`TOKEN_NAME_MAX`], contains anything outside `a-z`, `0-9`, `-` and `.`,
    /// does not start with a letter, or is spelled entirely from `0-9` and
    /// `a-f`. The last two clauses are the ones doing the work; see the type's
    /// documentation for why it takes both of them.
    pub const fn new(name: &str) -> Result<Self, Refused> {
        let src = name.as_bytes();
        if src.is_empty() || src.len() > TOKEN_NAME_MAX || !src[0].is_ascii_lowercase() {
            return Err(Refused::NotATokenName);
        }
        let mut bytes = [0u8; TOKEN_NAME_MAX];
        let mut at = 0;
        // Carried through the one pass rather than computed in a second one, so
        // that the two clauses of the grammar cannot come to disagree about
        // which bytes they read.
        let mut all_hex = true;
        while at < src.len() {
            let byte = src[at];
            let permitted =
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.';
            if !permitted {
                return Err(Refused::NotATokenName);
            }
            all_hex = all_hex && byte.is_ascii_hexdigit();
            bytes[at] = byte;
            at += 1;
        }
        if all_hex {
            return Err(Refused::NotATokenName);
        }
        Ok(Self { bytes, len: src.len() as u8 })
    }

    /// The name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        // The error arm is unreachable: the grammar admits ASCII only.
        core::str::from_utf8(&self.bytes[..self.len as usize]).unwrap_or_default()
    }

    /// Its length in bytes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Never, for a constructed name. Present because a length without it is
    /// half an interface.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// The style tokens a node wears, bounded by [`TOKENS_MAX`].
///
/// Opaque to this module on purpose: nothing here can resolve a token, order two
/// by precedence, or ask what colour one is. That is not an omission to be
/// filled in later — it is the reason a projection can hand one tree to a
/// high-contrast theme, a monochrome remote client and a screen reader that
/// renders nothing at all, and have all three be right.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TokenSet {
    /// The names, in declaration order. `token.rs` decides what order means.
    items: [Option<TokenName>; TOKENS_MAX],
    /// How many are real.
    len: u8,
}

impl TokenSet {
    /// Bare: the node takes whatever the theme gives a node of its role.
    pub const EMPTY: Self = Self { items: [None; TOKENS_MAX], len: 0 };

    /// This set with one more token.
    ///
    /// # Errors
    ///
    /// [`Refused::TooManyTokens`] past [`TOKENS_MAX`].
    pub const fn push(mut self, name: TokenName) -> Result<Self, Refused> {
        if self.len as usize >= TOKENS_MAX {
            return Err(Refused::TooManyTokens);
        }
        self.items[self.len as usize] = Some(name);
        self.len += 1;
        Ok(self)
    }

    /// How many tokens this node wears.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len as usize
    }

    /// Does it wear none?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Every token, in declaration order.
    pub fn iter(&self) -> impl Iterator<Item = TokenName> + '_ {
        self.items[..self.len as usize].iter().copied().flatten()
    }

    /// Is `name` among them?
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.iter().any(|token| token.as_str() == name)
    }
}

/// Which part of the vocabulary a role belongs to.
///
/// Four families, and the census is load-bearing rather than descriptive: a role
/// belonging to none of them would be a role with no answer to *what can a
/// projection do with this*, and cannot be written, because a `vocabulary!` line
/// without a family does not parse. A role that would need a fifth family is a
/// change to what this layer claims to be. The census is asserted by
/// `the_family_census_is_what_the_rfc_says`, so a twenty-third role fails a test
/// until somebody edits a number that RFC 0077 quotes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Holds other nodes and says how they relate. Nine roles.
    Structure,
    /// Says something and holds nothing. Four roles.
    Content,
    /// Carries intent: operating it changes the world. Five roles.
    Control,
    /// Places content in an ordered dimension the application owns. Four roles.
    Arrangement,
}

/// The vocabulary, written once so that it cannot be written twice.
///
/// # Why this is a macro, in a tree that mostly refuses them
///
/// Because the alternative was four sequences that had to agree. [`Role`]'s
/// variants, `Role::ALL`, `Role::COUNT` and `Role::index` were each written by
/// hand, and a role could be added to the enum and left out of the array: every
/// test in this module iterates `ALL`, and a loop over a list cannot see what
/// the list omits. Two adversarial reviews found that hole. The first was
/// answered with a test — an exhaustive match, one `const` arm per variant —
/// and the second found the hole still open underneath it, because an
/// exhaustive match demands an *arm*, not an arm that says anything.
/// `Role::Other => {}` compiles, and an empty arm is what somebody adding a
/// role in a hurry would write.
///
/// No test closes that. What closes it is that there is no second place to
/// write a role. One line below carries everything the rest of this module asks
/// a role for — what it is called, which family it belongs to, whether it may
/// carry an intent, and where it may sit — and `ALL`, `COUNT`, `index`, `name`,
/// `family`, `is_operable` and `accepts_parent` are all emitted from it. Adding
/// `Role::Other` means adding a line to a list somebody reads, in a diff that
/// also moves the family census `the_family_census_is_what_the_rfc_says`
/// asserts and trips `the_vocabulary_has_no_escape_hatch` by name. That is what
/// the exit of `E3-D01` wants: enlarging the vocabulary is a visible, arguable
/// act rather than something that can be smuggled past an array.
///
/// The order of the lines is the order of `ALL` and the value of `index`, and a
/// projection's dispatch table is indexed by it. The cost of the macro is that
/// one indirection stands between a reader and the enum, which is real and is
/// why nothing else in this crate is written this way: it is paid here because
/// here the second copy was the defect.
macro_rules! vocabulary {
    (
        $(
            $(#[$about:meta])*
            $variant:ident, $spelling:literal, $since:literal, $family:ident, $operable:literal,
            $parent:pat,
        )*
    ) => {
        /// What a node *is*. The closed vocabulary, version 1.
        ///
        /// Twenty-two variants: no `Other`, no `Custom`, no variant carrying a
        /// string, and no `#[non_exhaustive]`. The module documentation argues
        /// the size; RFC 0077 records what was refused to reach it, and what
        /// observation would say the number was wrong.
        ///
        /// Read the families rather than the list. [`Family::Structure`] is
        /// what a projection must preserve, [`Family::Content`] is what it must
        /// present, [`Family::Control`] is what it must make operable, and
        /// [`Family::Arrangement`] is the part an application draws itself
        /// while still saying what is in it.
        ///
        /// The variants, and everything this module answers about them, are
        /// emitted from the `vocabulary!` invocation that declares them, which
        /// is the only place a role is written.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Role {
            $($(#[$about])* $variant,)*
        }

        impl Role {
            /// How many roles version 1 of the vocabulary has.
            ///
            /// Counted from the list rather than written as a literal. It was a
            /// literal, under a comment claiming that writing the size down
            /// twice is what gave the closedness test its teeth. It did not:
            /// the second copy was an array, and a variant left out of an array
            /// is a variant no loop over that array can see. A number that is
            /// derived cannot disagree with what it is derived from. Changing
            /// this number means adding or removing a line of the vocabulary,
            /// and RFC 0077 says what that costs.
            pub const COUNT: usize = [$(stringify!($variant)),*].len();

            /// Every role, in declaration order, indexed by
            /// [`index`](Self::index).
            ///
            /// Emitted from the same list as the enum, so it holds every
            /// variant the enum has. Not because a test checks it — the test
            /// that used to is deleted, and deleting it is the repair — but
            /// because there is no way to write a variant this array does not
            /// get.
            ///
            /// **`pub(crate)`, and that is the whole of RFC 0083's *a second
            /// decoder*.** It was public, and a public array is a public
            /// ordinal-to-role map: `Role::ALL[ordinal as usize]` compiled in
            /// every crate, needed no admission, and panicked rather than
            /// refused when the ordinal was out of range — which is exactly the
            /// route a wire ordinal takes when somebody is in a hurry. What is
            /// public now is [`all`](Self::all), which cannot be indexed, and
            /// [`from_index`](Self::from_index), which answers `None`. The
            /// reversal condition is this line saying `pub` again.
            pub(crate) const ALL: [Self; Self::COUNT] = [$(Self::$variant),*];

            /// Every role, in declaration order.
            ///
            /// The iteration [`ALL`](Self::ALL) used to be, without the
            /// subscript. A caller that wants *all of them* gets all of them; a
            /// caller that wants *the one at ordinal `i`* is asking a different
            /// question and [`from_index`](Self::from_index) is where it is
            /// asked, because that is the question that has a wrong answer.
            pub fn all() -> impl Iterator<Item = Self> {
                Self::ALL.into_iter()
            }

            /// The role at that position in the vocabulary, if the vocabulary
            /// has one.
            ///
            /// **The one function that turns an ordinal into a [`Role`].** RFC
            /// 0083's *not representable* rests on there being exactly one, and
            /// this is it: `f_abi::semantic` admits an ordinal against the
            /// agreed version and hands it across the crate boundary as a
            /// number, because that crate cannot see this one, and this is
            /// where the number stops being a number.
            ///
            /// Derived from [`ALL`](Self::ALL) rather than written as a match,
            /// for [`from_name`](Self::from_name)'s reason and it is the same
            /// reason: there is no index this function accepts that is not
            /// already a role, so an integer cannot become an escape hatch by
            /// being passed through here. `None` and never a neutral role —
            /// the module's own *the vocabulary has no escape hatch* is the
            /// argument, and a fallback here would be the wildcard arm moved
            /// one function upstream of the projections that would have held
            /// it.
            ///
            /// It takes no version, and that is deliberate rather than
            /// forgotten: *which versions name this ordinal* is
            /// [`since`](Self::since) against the agreed version, asked by the
            /// `f_abi::semantic::Vocabulary` implementor before the ordinal
            /// gets here. A `from_index` that also took a version would be the
            /// admission written twice.
            #[must_use]
            pub fn from_index(index: usize) -> Option<Self> {
                Self::ALL.get(index).copied()
            }

            /// This role's position in [`ALL`](Self::ALL).
            ///
            /// The enum's own discriminant, which is the position of the line
            /// that declared the role, which is the position of its entry in
            /// `ALL`: one list, read three ways. Two earlier versions of this
            /// comment argued that a second hand-written sequence here was a
            /// check on the first. Neither was; there is no second sequence
            /// now.
            #[must_use]
            pub const fn index(self) -> usize {
                self as usize
            }

            /// The role's name, for a manifest, a log line, or a projection
            /// that prints one.
            ///
            /// Two roles spelled the same would make
            /// [`from_name`](Self::from_name) answer with the earlier one,
            /// which is the one way the list below can be internally wrong. So
            /// that is what `each_role_has_a_name_no_other_role_has` tests, and
            /// it is the only property of the list a test still carries.
            #[must_use]
            pub const fn name(self) -> &'static str {
                match self {
                    $(Self::$variant => $spelling,)*
                }
            }

            /// The role of that name, if the vocabulary has one.
            ///
            /// Derived from [`ALL`](Self::ALL) rather than written as a second
            /// match, which is what makes the negative answer structural: there
            /// is no name this function accepts that is not already a role, so
            /// a string cannot become an escape hatch by being passed through
            /// here.
            #[must_use]
            pub fn from_name(name: &str) -> Option<Self> {
                Self::ALL.into_iter().find(|role| role.name() == name)
            }

            /// The vocabulary version this role was introduced in.
            ///
            /// The second field of the role's line. RFC 0083's admission rule
            /// is `from_index(i)` **and** `since(role) <= agreed`, and this is
            /// the half that was missing: without it a `Vocabulary` implementor
            /// has nothing to filter by, so the version it was handed is a
            /// number it can only discard, and an older receiver is protected
            /// by its own list length — which is what it would have had with no
            /// handshake at all.
            ///
            /// Written on the line rather than derived from the position,
            /// because two roles appended in one release share a version and
            /// nothing about a position says so. The column is asserted
            /// non-decreasing at compile time below this macro, so a role
            /// appended out of order fails the build rather than a review: RFC
            /// 0083 part two's *the vocabulary's indices are append-only* is
            /// what that assertion is, said in the one place a build can read
            /// it.
            /// Unit: none — a vocabulary version ordinal.
            #[must_use]
            pub const fn since(self) -> u16 {
                match self {
                    $(Self::$variant => $since,)*
                }
            }

            /// Which part of the vocabulary this role belongs to.
            ///
            /// The third field of the role's line. A role belonging to no
            /// family cannot be written, because the line does not parse
            /// without one — the small version of the whole argument here: the
            /// question *what can a projection do with this* is asked where the
            /// role is declared rather than left for a test to ask later.
            #[must_use]
            pub const fn family(self) -> Family {
                match self {
                    $(Self::$variant => Family::$family,)*
                }
            }

            /// May a node of this role carry an intent?
            ///
            /// The dividing line is whether invoking it is unambiguous about
            /// *what* was invoked. A [`Role::Label`] with an intent is a
            /// control whose label is itself, which is exactly the ambiguity
            /// that made advisory annotation useless everywhere it was tried; a
            /// [`Role::Row`] with one is a file you can open, and there is
            /// nothing ambiguous about that.
            #[must_use]
            pub const fn is_operable(self) -> bool {
                match self {
                    $(Self::$variant => $operable,)*
                }
            }

            /// May a node of this role sit under a parent of that role?
            ///
            /// `None` is the root position. Only a [`Role::Surface`] may be a
            /// root: a tree with a floating control in it is a tree whose
            /// author has not said which surface it belongs to, and no
            /// projection can guess.
            ///
            /// The rule is the last field of the role's line, written as the
            /// pattern its parent must match. `Some(_any)` is *under anything
            /// at all, but never a root*. The rules that are not `Some(_any)`
            /// are the ones the vocabulary's meaning rests on — cells live in
            /// rows, rows in tables, clips on tracks, tracks in a canvas. Those
            /// are not conventions here.
            #[must_use]
            pub const fn accepts_parent(self, parent: Option<Self>) -> bool {
                match self {
                    $(Self::$variant => matches!(parent, $parent),)*
                }
            }
        }

        /// The `since` column never goes backwards, checked by the compiler.
        ///
        /// RFC 0083 part two says the vocabulary's indices are append-only, and
        /// this is the half of that a build can see: a role inserted in the
        /// middle of the list, or appended with a version below the one before
        /// it, is an index that has been re-meant. That failure is the one
        /// nobody notices — both peers say version 1, both are honest, and 14
        /// is `Toggle` on one and `Command` on the other — so it is refused
        /// here, where the answer arrives as a build error with the list in
        /// front of whoever caused it, rather than in a test that runs later
        /// and names a digest.
        ///
        /// It does not catch a *reorder within one version*, and nothing here
        /// could: two roles both `since: 1` may be swapped and this stays true.
        /// That is what `abi`'s frozen digest over version 1's names is for,
        /// and the two guards are complementary rather than redundant — this
        /// one covers the append, that one covers the shuffle.
        const _: () = {
            let since = [$($since),*];
            let mut i = 1;
            while i < since.len() {
                assert!(
                    since[i - 1] <= since[i],
                    "a role's `since` is below the role before it: the list is not append-only"
                );
                i += 1;
            }
            assert!(
                since.is_empty() || since[0] == 1,
                "the vocabulary's first role belongs to version 1, which is what version 1 is"
            );
        };
    };
}

vocabulary! {
    /// A region a projection can present on its own: a window, a screen, a pane.
    /// The only role that may be a root.
    Surface, "surface", 1, Structure, false, None | Some(Role::Surface),

    /// Children that belong together and are named together. The vocabulary's
    /// workhorse, and deliberately the answer to *toolbar*, *section*,
    /// *fieldset* and *card*, none of which tells a projection anything its
    /// children do not already.
    Group, "group", 1, Structure, false, Some(_any),

    /// An ordered sequence of [`Role::Item`]s that are alike.
    List, "list", 1, Structure, false, Some(_any),

    /// A hierarchy of [`Role::Item`]s that expands and collapses.
    Tree, "tree", 1, Structure, false, Some(_any),

    /// A member of a [`Role::List`], [`Role::Tree`] or [`Role::Choice`], or of
    /// another item where the hierarchy nests.
    Item, "item", 1, Structure, true, Some(Role::List | Role::Tree | Role::Choice | Role::Item),

    /// A grid whose rows carry the same cells in the same order. That equality
    /// is the whole of what makes a column a column here, and [`check`] enforces
    /// it rather than trusting it.
    Table, "table", 1, Structure, false, Some(_any),

    /// One row of a [`Role::Table`].
    Row, "row", 1, Structure, true, Some(Role::Table),

    /// One cell of a [`Role::Row`].
    Cell, "cell", 1, Structure, true, Some(Role::Row),

    /// A declared boundary between groups. Not decoration: it is the difference
    /// between two groups and one group with a gap in it, and only the author
    /// knows which was meant.
    Separator, "separator", 1, Structure, false, Some(_any),

    /// Text that names another node, which says so with
    /// [`Relation::LabelledBy`].
    Label, "label", 1, Content, false, Some(_any),

    /// Text that is itself the content: a paragraph, a file name, a message.
    Text, "text", 1, Content, false, Some(_any),

    /// A picture the projection may describe, substitute, or decline to render.
    /// It carries no intent: a picture you can click is a [`Role::Command`]
    /// whose content is media.
    Image, "image", 1, Content, false, Some(_any),

    /// A reading of how something stands — a count, a message, a condition. A
    /// separate role for *alert* was refused deliberately: urgency is
    /// [`StateSet::INVALID`] or a style token, and a role meaning *pay
    /// attention* is a role every author uses for everything.
    Status, "status", 1, Content, false, Some(_any),

    /// Invoking it does one thing, named by its intent.
    Command, "command", 1, Control, true, Some(_any),

    /// Two states, and invoking it moves between them.
    Toggle, "toggle", 1, Control, true, Some(_any),

    /// Free text a caller supplies.
    Entry, "entry", 1, Control, true, Some(_any),

    /// Exactly one of its [`Role::Item`] children is selected. A radio group, a
    /// dropdown and a segmented control are one role here because they are one
    /// fact and three appearances.
    Choice, "choice", 1, Control, true, Some(_any),

    /// A quantity a caller sets, within whatever its [`Reading`] says. A slider,
    /// a stepper and a spinner are presentations of this — and so is a progress
    /// bar, which is this role with no intent.
    Number, "number", 1, Control, true, Some(_any),

    /// A region whose pixels the application draws, and whose content it must
    /// still declare as children.
    ///
    /// This is the loophole section 13 says could eat the thesis from inside, so
    /// it is not left as a convention: a canvas with no declared children is a
    /// [`Defect::EmptyCanvas`] and the tree does not pass [`check`]. A canvas
    /// that declares nothing is strictly less expressive than the [`Role::Group`]
    /// it should have been.
    Canvas, "canvas", 1, Arrangement, true, Some(_any),

    /// A named lane of an arrangement, declared under a [`Role::Canvas`].
    Track, "track", 1, Arrangement, true, Some(Role::Canvas),

    /// A placed occupant of a [`Role::Track`]. *Where* it is placed — the time
    /// range section 13's question turns on — is not a field here: `canvas.rs`
    /// (`E3-D02`) holds the arrangement and keys it by [`NodeId`], because a
    /// vocabulary extended by widening its node is not closed.
    Clip, "clip", 1, Arrangement, true, Some(Role::Track),

    /// A named point in the arrangement's dimension: a playhead, a cue, a
    /// bookmark.
    Marker, "marker", 1, Arrangement, true, Some(Role::Canvas | Role::Track),
}

/// What an application declares. Section 11's struct, plus one field.
///
/// # The one field section 11 does not have
///
/// [`parent`](Self::parent). Section 11's struct carries no structure at all,
/// because there the tree's shape is stated by the transport — `DeclareNode`
/// says where a node goes — and the struct is only what a node *holds*. This
/// crate has no transport, and a vocabulary you cannot build a tree out of
/// cannot be shown to express three interfaces, which is this decision's exit.
/// So the parent is carried on the node, and a `&[Node]` is a tree. RFC 0077
/// records this as a deliberate departure and names what would make it wrong.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Node {
    /// Stable, author-assigned, and surviving redesign. See [`NodeId`].
    pub id: NodeId,
    /// The node this one sits under, or [`NodeId::UNNAMED`] for a root.
    pub parent: NodeId,
    /// What it is. Closed: see [`Role`].
    pub role: Role,
    /// What it holds.
    pub content: Content,
    /// How it stands.
    pub state: StateSet,
    /// Edges the tree's shape does not already say.
    pub relations: Relations,
    /// What operating it does — a capability, never a callback. `None` means the
    /// node is not operable, which is how read-only is spelled.
    pub intent: Option<CapRef>,
    /// What it needs in order to be useful. Never where it is.
    pub layout: Constraints,
    /// Which semantic tokens it wears. Never what they resolve to.
    pub style: TokenSet,
}

impl Node {
    /// A node with an identity, a place and a role, and nothing else said.
    #[must_use]
    pub const fn new(id: NodeId, parent: NodeId, role: Role) -> Self {
        Self {
            id,
            parent,
            role,
            content: Content::None,
            state: StateSet::NONE,
            relations: Relations::EMPTY,
            intent: None,
            layout: Constraints::LEAF,
            style: TokenSet::EMPTY,
        }
    }

    /// The same node, saying this.
    #[must_use]
    pub fn with_content(self, content: Content) -> Self {
        Self { content, ..self }
    }

    /// The same node, standing like this.
    #[must_use]
    pub fn with_state(self, state: StateSet) -> Self {
        Self { state, ..self }
    }

    /// The same node, doing this when operated.
    ///
    /// Nothing here refuses an intent on an inert role. That refusal belongs to
    /// [`check`], which sees the whole tree and can therefore say which node it
    /// is talking about.
    #[must_use]
    pub fn with_intent(self, intent: CapRef) -> Self {
        Self { intent: Some(intent), ..self }
    }

    /// The same node, needing this.
    #[must_use]
    pub fn with_layout(self, layout: Constraints) -> Self {
        Self { layout, ..self }
    }

    /// The same node, wearing these.
    #[must_use]
    pub fn with_style(self, style: TokenSet) -> Self {
        Self { style, ..self }
    }

    /// The same node, with one more edge.
    ///
    /// # Errors
    ///
    /// [`Refused::TooManyRelations`] past [`RELATIONS_MAX`].
    pub fn with_relation(self, relation: Relation) -> Result<Self, Refused> {
        match self.relations.push(relation) {
            Ok(relations) => Ok(Self { relations, ..self }),
            Err(refused) => Err(refused),
        }
    }
}

/// What is wrong with a declared tree.
///
/// Closed, like everything else here, and each variant names the node so that
/// *which one* does not need a second pass to answer. These are defects of the
/// declaration rather than of the projection: every one of them is something an
/// author can fix and no projection can work around.
///
/// **A variant here owes a test that produces it.** Four of them had none — a
/// review found `DuplicateId`, `UnknownParent`, `UnknownRelation` and `TooDeep`
/// carrying documented claims that nothing ran, and `TooDeep` carries two at
/// once, since it is also how [`check`] terminates on a cycle. All nine have
/// one now. This is an obligation on whoever adds the tenth rather than a
/// property anything mechanical checks: a defect nothing constructs is a
/// sentence, and this module has been wrong about enough of those.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Defect {
    /// A node whose own identity is [`NodeId::UNNAMED`], which names nothing.
    ///
    /// Named here by the parent it claimed, because its own identity is the
    /// value that identifies nothing — which is the whole of why this is a
    /// defect. Nothing can address the node; worse, [`find`] answers it for
    /// every edge that meant *nobody*, so a [`Relation::LabelledBy`] pointing
    /// at [`NodeId::UNNAMED`] would resolve and the tree would pass. Refusing
    /// the identity is what makes that relation a
    /// [`Defect::UnknownRelation`] instead.
    Unnamed {
        /// What it claimed to sit under. [`NodeId::UNNAMED`] here means it
        /// claimed to be a root as well, which is the whole of what it said.
        parent: NodeId,
    },
    /// Two nodes claim one identity, so nothing can address either.
    DuplicateId(NodeId),
    /// A node sits under something that was never declared.
    UnknownParent {
        /// The node with the dangling parent.
        node: NodeId,
        /// What it claimed to sit under.
        parent: NodeId,
    },
    /// A relation points at something that was never declared.
    UnknownRelation {
        /// The node that declared the edge.
        node: NodeId,
        /// What the edge pointed at.
        target: NodeId,
    },
    /// A node sits somewhere its role does not permit — a cell outside a row, a
    /// clip off a track, a control with no surface over it.
    MisplacedChild(NodeId),
    /// A table whose rows carry different numbers of cells, which means it has
    /// no columns, which means it is a list wearing a table's name.
    RaggedTable(NodeId),
    /// An intent on a role that cannot carry one. See [`Role::is_operable`].
    IntentOnInertRole(NodeId),
    /// A canvas that declared no content: the escape hatch, refused.
    EmptyCanvas(NodeId),
    /// A node deeper than [`DEPTH_MAX`], or one whose parent chain never reaches
    /// a root. The two are one check, because a cycle is a chain that does not
    /// end, and the tree is equally unprojectable either way.
    TooDeep(NodeId),
}

/// The node with this identity, if the tree declares one.
#[must_use]
pub fn find(nodes: &[Node], id: NodeId) -> Option<&Node> {
    nodes.iter().find(|node| node.id == id)
}

/// Every node declared directly under `parent`, in declaration order.
pub fn children(nodes: &[Node], parent: NodeId) -> impl Iterator<Item = &Node> {
    nodes.iter().filter(move |node| node.parent == parent)
}

/// Does anything sit under `parent`?
#[must_use]
pub fn has_children(nodes: &[Node], parent: NodeId) -> bool {
    children(nodes, parent).next().is_some()
}

/// How many of this tree's canvases declared nothing.
///
/// **Not the canvas-escape rate, and this comment used to say it was the rate's
/// single-tree term.** It is not: RFC 0077's condition counts *canvases*
/// against nodes, and this counts the narrower thing — a canvas that kept its
/// pixels and declared no content, which is the floor of
/// [`Defect::EmptyCanvas`] being evaded rather than the vocabulary being too
/// small. [`canvas_census`] is the rate's per-tree half.
///
/// A tree that passes [`check`] therefore scores zero here **by construction**,
/// because `check` refuses exactly what this counts — which means an assertion
/// that a checked tree scores zero asserts nothing, and three of this module's
/// assertions used to. What the count is for is a tree nobody checked: a corpus
/// gathered from applications, a projection handed a tree from across a ring, a
/// fixture. `the_escape_count_is_not_the_checker_wearing_a_different_name`
/// exercises it where it can be wrong — `check` names the first defect and
/// stops, and a report that stopped at the first escape would understate.
#[must_use]
pub fn canvas_escapes(nodes: &[Node]) -> usize {
    nodes.iter().filter(|node| node.role == Role::Canvas && !has_children(nodes, node.id)).count()
}

/// The two terms RFC 0077's reversal condition is a ratio of, for one tree: how
/// many nodes are a [`Role::Canvas`], and how many nodes there are.
///
/// `None` for a tree with no nodes, and the `Option` is the whole point of the
/// signature. A rate over an empty corpus is not zero, it is **absent**, and a
/// clean zero is the answer that would retire a reversal condition by accident —
/// the one direction in which this metric can do real damage, because it damages
/// it silently and in the reassuring direction. `E3-B06l`'s exit asks the
/// corpus-level reproduction for the same refusal; this is the per-tree half of
/// it, and the two halves are written the same way on purpose.
///
/// Two integers rather than a ratio, deliberately. A ratio here would be a fixed
/// point whose scale this module chose, and the scale belongs with the target —
/// `claims/0034-canvas-escape-rate.toml` carries both, spelled
/// `canvas_escape_rate_x1000`. It would also be a ratio of one tree, and the
/// rate is defined over a corpus: summing two integers per tree is the only
/// composition that gives the corpus its own denominator rather than an average
/// of averages.
#[must_use]
pub fn canvas_census(nodes: &[Node]) -> Option<(usize, usize)> {
    if nodes.is_empty() {
        return None;
    }
    Some((nodes.iter().filter(|node| node.role == Role::Canvas).count(), nodes.len()))
}

/// Is this a tree a projection can be given?
///
/// # Errors
///
/// The first [`Defect`] found, rather than all of them: a tree with two defects
/// is a tree whose author is still writing it, and a list of consequences is
/// less useful than the first cause.
///
/// # Why this is quadratic, and why that is the right answer here
///
/// Identity and relation targets are resolved by scanning, because this tree has
/// no hash set — RFC 0004, and the iteration order of one is seeded per process
/// — and a sorted index would be scratch memory this signature does not have. A
/// tree large enough for the cost to matter is a tree that fails [`DEPTH_MAX`],
/// or belongs to a projection, which validates incrementally at the delta that
/// changed something rather than over the whole tree. This function is for
/// authors and for tests, and saying so is cheaper than a benchmark nobody would
/// act on.
pub fn check(nodes: &[Node]) -> Result<(), Defect> {
    for (at, node) in nodes.iter().enumerate() {
        // First, because everything below this line reads identities. A node
        // holding `UNNAMED` is not a node with a bad name, it is a node that
        // makes `find` lie: an edge pointing at nothing would resolve to it.
        if !node.id.is_named() {
            return Err(Defect::Unnamed { parent: node.parent });
        }

        if nodes[..at].iter().any(|earlier| earlier.id == node.id) {
            return Err(Defect::DuplicateId(node.id));
        }

        let parent = if node.parent.is_named() {
            match find(nodes, node.parent) {
                Some(found) => Some(found.role),
                None => return Err(Defect::UnknownParent { node: node.id, parent: node.parent }),
            }
        } else {
            None
        };
        if !node.role.accepts_parent(parent) {
            return Err(Defect::MisplacedChild(node.id));
        }

        for relation in node.relations.iter() {
            if find(nodes, relation.target()).is_none() {
                return Err(Defect::UnknownRelation { node: node.id, target: relation.target() });
            }
        }

        if node.intent.is_some() && !node.role.is_operable() {
            return Err(Defect::IntentOnInertRole(node.id));
        }

        if node.role == Role::Canvas && !has_children(nodes, node.id) {
            return Err(Defect::EmptyCanvas(node.id));
        }

        if node.role == Role::Table {
            let mut width: Option<usize> = None;
            // Rows, not children. `accepts_parent` admits a separator under a
            // table — and a separator between two groups of rows is precisely
            // what that role is for, being the difference between two groups
            // and one group with a gap — but a separator carries no cells, so
            // walking every child measured it as a row of width zero and
            // refused the table. `Defect::RaggedTable` says *a table whose rows
            // carry different numbers of cells*; this is the walk that sentence
            // describes. The same reading applies to every other role a table
            // may hold: a caption is not a short row.
            for row in children(nodes, node.id).filter(|child| child.role == Role::Row) {
                let cells = children(nodes, row.id).filter(|cell| cell.role == Role::Cell).count();
                match width {
                    None => width = Some(cells),
                    Some(seen) if seen != cells => return Err(Defect::RaggedTable(node.id)),
                    Some(_) => {}
                }
            }
        }

        // Walk to the root, bounded. A chain longer than the bound is either too
        // deep or a cycle, and nothing downstream can tell the difference.
        let mut walked = 0;
        let mut walking = node.parent;
        while walking.is_named() {
            if walked >= DEPTH_MAX {
                return Err(Defect::TooDeep(node.id));
            }
            match find(nodes, walking) {
                Some(found) => walking = found.parent,
                None => return Err(Defect::UnknownParent { node: node.id, parent: walking }),
            }
            walked += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Words a role would be called if somebody gave up.
    ///
    /// The exit of `E3-D01` is *no node marked "other"*, and the cheapest way
    /// for that to stop being true is not a `Role::Other` — which would be
    /// argued about — but a role added late with a name that means the same
    /// thing and a doc comment that apologises for it. So the refusal is
    /// checked against the spellings rather than against the one variant, and
    /// it is checked over `Role::ALL` rather than over the nodes of any one
    /// tree, so that the assertion is about the vocabulary and not about three
    /// examples of it.
    const ESCAPE_WORDS: [&str; 10] = [
        "other",
        "custom",
        "generic",
        "misc",
        "unknown",
        "unspecified",
        "extension",
        "vendor",
        "opaque",
        "any",
    ];

    // `same_name` and `all_holds` were here — two `const fn`s supporting an
    // exhaustive match that asserted every variant into `Role::ALL` at compile
    // time. They are gone with it. The question they answered, *does `ALL` hold
    // this variant*, is no longer a question: `ALL` is emitted from the same
    // list as the enum, so there is no variant it can fail to hold. A helper
    // whose answer is fixed by construction reads as a check and is a decoration.

    fn text(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits within TEXT_MAX"))
    }

    fn styled(names: &[&str]) -> TokenSet {
        let mut set = TokenSet::EMPTY;
        for name in names {
            set = set
                .push(TokenName::new(name).expect("a token name, not a value"))
                .expect("within TOKENS_MAX");
        }
        set
    }

    fn related(node: Node, edges: &[Relation]) -> Node {
        let mut built = node;
        for edge in edges {
            built = built.with_relation(*edge).expect("within RELATIONS_MAX");
        }
        built
    }

    /// How many canvases a tree is supposed to have, asserted per fixture.
    ///
    /// **This used to be called `no_node_is_an_escape` and used to do three more
    /// things, none of which could fail.** It walked the tree asserting that
    /// each `node.role` was in `Role::ALL`, that no role name read as an escape
    /// word, and that `from_name` round-tripped — but `node.role` is a `Role`,
    /// so all three were questions about the *type* restricted to the roles
    /// three fixtures happen to use, and the type is what
    /// `the_vocabulary_has_no_escape_hatch` and
    /// `each_role_has_a_name_no_other_role_has` already cover over every role.
    ///
    /// What is left is the one thing that is about the tree: the count is a
    /// parameter rather than a zero because two of the three interfaces contain
    /// no canvas at all, and a fixture that loses its canvas or grows one says
    /// so here. The timeline losing its canvas is the timeline ceasing to be the
    /// hard case this module chose it for.
    fn canvases_number(nodes: &[Node], canvases: usize) {
        assert_eq!(
            nodes.iter().filter(|node| node.role == Role::Canvas).count(),
            canvases,
            "the tree's canvases moved, and the exit's hard case moved with them"
        );
    }

    // `all_is_every_variant_the_enum_has` was here, and deleting it is the
    // repair rather than a casualty of it.
    //
    // It matched a `Role` exhaustively, one `const` arm per variant, each arm
    // asserting that `Role::ALL` held the variant that arm named. The claim
    // written about it — in this file and in RFC 0077 — was that a
    // twenty-third role then had two endings and neither was green. There was a
    // third and it was green: `Role::Other => {}`. An exhaustive match demands
    // an *arm*, not an arm that says anything, so a `Role::Other` with an
    // index, a name, the other required arms and an empty arm here passed the
    // whole suite with `ALL` and `COUNT` untouched — including
    // `the_vocabulary_has_no_escape_hatch`, which carries this decision's one
    // named clause.
    //
    // The lesson is not that the test needed a sharper assertion. It is that a
    // test cannot guard a registry it has to be kept in step with by hand. So
    // the registry is gone: `vocabulary!` above emits the enum, `ALL`, `COUNT`,
    // `index`, `name`, `family`, `is_operable` and `accepts_parent` from one
    // list, and a role that is not in that list does not exist to be tested
    // for. What the tests below assert is what the list can still get wrong.

    #[test]
    fn each_role_has_a_name_no_other_role_has() {
        // The one way the vocabulary list can be internally wrong. Everything
        // else about it — that `ALL` holds every variant, that `index` agrees
        // with `ALL`'s order, that `COUNT` is `ALL`'s length — is emitted from
        // the list and cannot disagree with it. Two *spellings* can collide,
        // because the spelling is written out per line, and a collision is not
        // loud: `from_name` answers the earlier role and a projection keyed by
        // name silently merges two roles into one.
        //
        // *The edit that makes this go red:* give two lines in `vocabulary!`
        // one spelling — `Marker, "clip", ...` — and this fails on the round
        // trip before it fails on the uniqueness scan.
        for (at, role) in Role::ALL.iter().enumerate() {
            assert_eq!(
                Role::from_name(role.name()),
                Some(*role),
                "{} does not resolve to itself",
                role.name()
            );
            assert!(
                Role::ALL[..at].iter().all(|earlier| earlier.name() != role.name()),
                "{} shares a name with an earlier role",
                role.name()
            );
        }
    }

    #[test]
    fn the_vocabulary_has_no_escape_hatch() {
        // This decision's one named clause, and the loop over `Role::ALL` is
        // now sound for a structural reason rather than a tested one: `ALL` is
        // emitted from the vocabulary list, so it is every role there is. The
        // version of this test that failed review could not see a `Role::Other`
        // kept out of a hand-written array. There is no hand-written array.
        //
        // *The edit that makes this go red:* add a line to `vocabulary!` —
        // `Other, "other", 1, Structure, false, Some(_any),` — which is the
        // whole of what adding the escape hatch now costs, and this test names
        // it.
        // (`the_family_census_is_what_the_rfc_says` fails on the same line, for
        // the second reason: the census and `COUNT` both move.)
        for role in Role::ALL {
            for word in ESCAPE_WORDS {
                assert!(
                    !role.name().contains(word),
                    "{} is an escape hatch wearing a role's name",
                    role.name()
                );
            }
        }
    }

    #[test]
    fn a_name_that_is_not_a_role_is_refused() {
        // `from_name` is derived from `ALL`, so this asserts a property of the
        // vocabulary rather than of a second match somebody could widen: there
        // is no string that becomes a role by being passed through it.
        for word in ESCAPE_WORDS {
            assert_eq!(Role::from_name(word), None, "{word} was accepted as a role");
        }
        assert_eq!(Role::from_name(""), None);
        assert_eq!(Role::from_name("Surface"), None, "names are exact, not case-folded");
        assert_eq!(Role::from_name("timeline-widget"), None);
    }

    #[test]
    fn an_ordinal_becomes_a_role_here_or_it_does_not_become_one() {
        // RFC 0083's *not representable* rests on there being exactly one
        // function that turns an ordinal into a `Role`, and for three rounds
        // there was none — the RFC named `Role::from_index` as a fact and the
        // item did not exist, while `Role::ALL` was public and
        // `Role::ALL[ordinal as usize]` was a shorter route to the same answer
        // that skipped the admission and panicked instead of refusing. `ALL` is
        // `pub(crate)` now, so that route does not compile outside this crate;
        // this is the route that replaced it.
        //
        // The negative answer is the whole test. A `from_index` that clamped,
        // wrapped or fell back to a neutral role would be the wildcard arm
        // moved one function upstream of the projections that were closed to
        // prevent it, and it would be invisible: every ordinal would decode and
        // a settings panel would be announced wrongly forever.
        //
        // *The edits that make this go red:* making `from_index` total —
        // `ALL[index % COUNT]`, `ALL.get(index).copied().unwrap_or(Role::Group)`
        // — or deriving it from anything but `ALL`, which is how it comes to
        // accept an index that is not a role.
        for (at, role) in Role::all().enumerate() {
            assert_eq!(role.index(), at, "{} is not at its own index", role.name());
            assert_eq!(Role::from_index(at), Some(role), "{} does not round-trip", role.name());
        }
        assert_eq!(Role::all().count(), Role::COUNT);

        // One past the end, and the two values a wire ordinal widened to a
        // `usize` most easily becomes. None of them is a role.
        assert_eq!(Role::from_index(Role::COUNT), None);
        assert_eq!(Role::from_index(Role::COUNT + 1), None);
        assert_eq!(Role::from_index(usize::from(u16::MAX)), None);
        assert_eq!(Role::from_index(usize::MAX), None);
    }

    #[test]
    fn every_role_says_which_version_introduced_it_and_the_column_only_goes_up() {
        // The half of RFC 0083's admission rule that lives here.
        // `f_abi::semantic` asks a `Vocabulary` implementor whether the agreed
        // version names an ordinal, and what the implementor is supposed to
        // answer with is `ALL.iter().filter(|r| r.since() <= agreed)`. Until
        // this column existed there was nothing to filter by, so the agreed
        // version was a number every implementor discarded and an older
        // receiver was protected by its own list length — which is what it
        // would have had with no handshake at all.
        //
        // The non-decreasing property is asserted by the compiler beside the
        // macro, so the edit that breaks it is a build failure rather than this
        // test. It is asserted again here for the reason the tree asserts
        // `size_of::<Sqe>()` twice: a fact held in another item is a fact this
        // one is trusting rather than stating, and a reader of this test should
        // not have to find the `const` block to know the rule exists.
        let mut floor = 0;
        for role in Role::all() {
            assert!(
                role.since() >= floor,
                "{} was introduced before the role in front of it",
                role.name()
            );
            assert!(
                role.since() >= 1,
                "{} claims a version zero, which is no version",
                role.name()
            );
            floor = role.since();
        }

        // And version 1 is the whole list today, which is the fact RFC 0083's
        // frozen digest is a digest *of*. When this stops being true — the day
        // a role is appended under version 2 — the honest edit is to say so
        // here rather than to loosen it, because the number this asserts is
        // what `abi`'s `VOCABULARY_VERSION` has to agree with.
        assert_eq!(floor, 1, "the vocabulary has a version 2 and this test still says it does not");
    }

    #[test]
    fn the_family_census_is_what_the_rfc_says() {
        // Nine, four, five, four. A role cannot be added without a family —
        // the `vocabulary!` line does not parse without one — so what this
        // catches is the addition itself: any new line moves one of these four
        // numbers, and all four are quoted by RFC 0077. That is the point. The
        // diff that grows the vocabulary is the diff that has to argue for it.
        //
        // *The edit that makes this go red:* add any line to `vocabulary!`, or
        // move an existing role between families — `Status` from `Content` to
        // `Control`, say, which is the change somebody would make to give a
        // status an intent without saying so.
        let census =
            |family: Family| Role::ALL.iter().filter(|role| role.family() == family).count();
        assert_eq!(census(Family::Structure), 9);
        assert_eq!(census(Family::Content), 4);
        assert_eq!(census(Family::Control), 5);
        assert_eq!(census(Family::Arrangement), 4);
        assert_eq!(
            census(Family::Structure)
                + census(Family::Content)
                + census(Family::Control)
                + census(Family::Arrangement),
            Role::COUNT
        );
    }

    #[test]
    fn a_token_name_spelled_like_a_value_is_refused() {
        // The one hole in *style is never values*, closed at construction.
        assert_eq!(TokenName::new("ff0000"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new("12px"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new("50%"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new("#surface"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new("Surface"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new(""), Err(Refused::NotATokenName));
        // The hex alphabet, not the leading byte: every one of these begins
        // with a letter and would have passed a grammar that only checked that.
        assert_eq!(TokenName::new("abcdef"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new("a1b2c3"), Err(Refused::NotATokenName));
        assert_eq!(TokenName::new("beef"), Err(Refused::NotATokenName));
        assert!(TokenName::new("surface-2").is_ok());
        assert!(TokenName::new("emphasis").is_ok());
        assert!(TokenName::new("text.danger").is_ok());
    }

    #[test]
    fn text_too_long_is_refused_rather_than_truncated() {
        // Built from an array rather than from `str::repeat`, which lives in
        // `alloc` and this crate does not have one.
        let bytes = [b'n'; TEXT_MAX + 1];
        let long = core::str::from_utf8(&bytes).expect("ASCII");
        assert_eq!(Text::new(long), Err(Refused::TextTooLong));
        let fits = core::str::from_utf8(&bytes[..TEXT_MAX]).expect("ASCII");
        assert_eq!(Text::new(fits).expect("exactly the maximum").len(), TEXT_MAX);
    }

    #[test]
    fn a_fifth_relation_is_refused() {
        // `RELATIONS_MAX` and `TOKENS_MAX` each carry a paragraph of reasoning
        // and neither bound was ever reached by a test, which is how
        // `RELATIONS_MAX` came to be explained by a sentence about `check` that
        // was not true of `check`. A bound nothing hits is a bound nobody has
        // read.
        //
        // *The edit that makes this go red:* change `RELATIONS_MAX` to 5. The
        // fifth relation is then accepted and the refusal never arrives.
        let node = Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface);
        let edges = [
            Relation::LabelledBy(NodeId::new(2)),
            Relation::Controls(NodeId::new(3)),
            Relation::Owns(NodeId::new(4)),
            Relation::FlowsTo(NodeId::new(5)),
        ];
        let full = related(node, &edges);
        assert_eq!(full.relations.len(), RELATIONS_MAX);
        assert_eq!(
            full.with_relation(Relation::Owns(NodeId::new(6))),
            Err(Refused::TooManyRelations)
        );
    }

    #[test]
    fn a_fifth_token_is_refused() {
        // *The edit that makes this go red:* change `TOKENS_MAX` to 5.
        let mut set = TokenSet::EMPTY;
        for name in ["surface-1", "emphasis", "text.muted", "field.danger"] {
            set = set.push(TokenName::new(name).expect("a name")).expect("within the bound");
        }
        assert_eq!(set.len(), TOKENS_MAX);
        assert_eq!(set.push(TokenName::new("panel").expect("a name")), Err(Refused::TooManyTokens));
    }

    /// The first hard interface: a settings panel — **the shipped one**.
    ///
    /// It moved to [`crate::example`] and is imported rather than rebuilt
    /// here. `E3-B06h`'s exit is about *one unmodified application*, and a
    /// fixture only this file can reach is a fixture every projection copies;
    /// two copies of a declaration are two applications, and the second one
    /// stops being the one anybody declares the moment a node is added to the
    /// first. The assertions below are unchanged and now run against the value
    /// the projections are handed.
    use crate::example::settings_panel;

    #[test]
    fn a_settings_panel_is_expressible() {
        let panel = settings_panel();
        check(&panel).expect("a settings panel the vocabulary can hold");
        canvases_number(&panel, 0);

        // What a projection gets for free, and what the four rules bought.
        let volume = find(&panel, NodeId::new(16)).expect("the volume");
        let Content::Value(reading) = volume.content else { panic!("a volume is a quantity") };
        assert_eq!(reading.value.unit, Unit::Ratio);
        assert_eq!(reading.low.expect("a floor").scaled, 0);
        assert_eq!(reading.high.expect("a ceiling").scaled, 100);
        assert_eq!(reading.step.expect("an increment").scaled, 5);
        assert!(volume.intent.is_some(), "a settable quantity names the capability that sets it");
        assert!(
            volume.relations.iter().any(|edge| edge == Relation::LabelledBy(NodeId::new(15))),
            "a screen reader learns the name from the tree, not from the author"
        );

        let mute = find(&panel, NodeId::new(17)).expect("the mute toggle");
        assert!(mute.relations.iter().any(|edge| edge == Relation::Controls(NodeId::new(16))));

        // Style is names. Nothing in this tree can hold a value, and the one
        // place one could be smuggled in is refused at construction.
        let entry = find(&panel, NodeId::new(22)).expect("the name entry");
        assert!(entry.style.contains("field.danger"));
        assert!(entry.state.holds(StateSet::INVALID));
        for node in &panel {
            for token in node.style.iter() {
                assert!(
                    TokenName::new(token.as_str()).is_ok(),
                    "a token that would not be accepted today is in a tree"
                );
            }
        }
    }

    /// The second hard interface: a file browser.
    ///
    /// Chosen because it punishes a vocabulary with no honest notion of a
    /// column. A listing is a [`Role::Table`] whose rows carry the same cells in
    /// the same order, and that equality is the whole of what makes *the size
    /// column* something an agent can name — so [`check`] enforces it rather
    /// than leaving it to whoever wrote the rows.
    ///
    /// The tree is ordered so that a cell is last, which lets the ragged case
    /// below be this same tree one node shorter rather than a second fixture
    /// that could drift away from it.
    fn file_browser() -> [Node; 23] {
        const SURFACE: NodeId = NodeId::new(100);
        const BAR: NodeId = NodeId::new(110);
        const BACK: NodeId = NodeId::new(111);
        const FORWARD: NodeId = NodeId::new(112);
        const SEARCH_LABEL: NodeId = NodeId::new(113);
        const SEARCH: NodeId = NodeId::new(114);
        const PLACES: NodeId = NodeId::new(120);
        const HOME: NodeId = NodeId::new(121);
        const DOCUMENTS: NodeId = NodeId::new(122);
        const PICTURES: NodeId = NodeId::new(123);
        const PREVIEW: NodeId = NodeId::new(130);
        const THUMBNAIL: NodeId = NodeId::new(131);
        const COUNTED: NodeId = NodeId::new(140);
        const LISTING: NodeId = NodeId::new(150);
        const HEAD: NodeId = NodeId::new(151);
        const HEAD_NAME: NodeId = NodeId::new(152);
        const HEAD_SIZE: NodeId = NodeId::new(153);
        const REPORT: NodeId = NodeId::new(160);
        const REPORT_NAME: NodeId = NodeId::new(161);
        const REPORT_SIZE: NodeId = NodeId::new(162);
        const NOTES: NodeId = NodeId::new(170);
        const NOTES_NAME: NodeId = NodeId::new(171);
        const NOTES_SIZE: NodeId = NodeId::new(172);

        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_content(text("Files"))
                .with_layout(Constraints::flowing(Flow::Block)),
            Node::new(BAR, SURFACE, Role::Group)
                .with_content(text("Navigation"))
                .with_layout(Constraints::flowing(Flow::Inline)),
            Node::new(BACK, BAR, Role::Command)
                .with_content(text("Back"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0101)),
            Node::new(FORWARD, BAR, Role::Command)
                .with_content(text("Forward"))
                .with_intent(CapRef::new(0x0102)),
            Node::new(SEARCH_LABEL, BAR, Role::Label).with_content(text("Search")),
            related(
                Node::new(SEARCH, BAR, Role::Entry)
                    .with_state(StateSet::ENABLED)
                    .with_intent(CapRef::new(0x0103))
                    .with_layout(Constraints::flowing(Flow::Inline).growing(1)),
                &[Relation::LabelledBy(SEARCH_LABEL)],
            ),
            Node::new(PLACES, SURFACE, Role::Tree)
                .with_content(text("Places"))
                .with_layout(Constraints::flowing(Flow::Block).at_most(1600)),
            Node::new(HOME, PLACES, Role::Item)
                .with_content(text("Home"))
                .with_state(StateSet::ENABLED.with(StateSet::EXPANDED))
                .with_intent(CapRef::new(0x0104)),
            Node::new(DOCUMENTS, HOME, Role::Item)
                .with_content(text("Documents"))
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(CapRef::new(0x0105)),
            Node::new(PICTURES, HOME, Role::Item)
                .with_content(text("Pictures"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0106)),
            Node::new(PREVIEW, SURFACE, Role::Group).with_content(text("Preview")),
            Node::new(THUMBNAIL, PREVIEW, Role::Image)
                .with_content(Content::Media(CapRef::new(0x0107)))
                .with_layout(Constraints::LEAF.at_least(600)),
            Node::new(COUNTED, SURFACE, Role::Status)
                .with_content(Content::Value(Reading::plain(Quantity::whole(2, Unit::Count)))),
            related(
                Node::new(LISTING, SURFACE, Role::Table)
                    .with_content(text("Documents"))
                    .with_layout(Constraints::flowing(Flow::Block).growing(1)),
                &[Relation::LabelledBy(HEAD)],
            ),
            Node::new(HEAD, LISTING, Role::Row),
            related(
                Node::new(HEAD_NAME, HEAD, Role::Cell)
                    .with_content(text("Name"))
                    .with_state(StateSet::ENABLED)
                    .with_intent(CapRef::new(0x0108)),
                &[Relation::Controls(LISTING)],
            ),
            related(
                Node::new(HEAD_SIZE, HEAD, Role::Cell)
                    .with_content(text("Size"))
                    .with_state(StateSet::ENABLED)
                    .with_intent(CapRef::new(0x0109)),
                &[Relation::Controls(LISTING)],
            ),
            Node::new(REPORT, LISTING, Role::Row)
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(CapRef::new(0x010a)),
            Node::new(REPORT_NAME, REPORT, Role::Cell).with_content(text("quarterly-report.pdf")),
            Node::new(REPORT_SIZE, REPORT, Role::Cell)
                .with_content(Content::Value(Reading::plain(Quantity::whole(20418, Unit::Bytes)))),
            Node::new(NOTES, LISTING, Role::Row)
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x010b)),
            Node::new(NOTES_NAME, NOTES, Role::Cell).with_content(text("notes.md")),
            Node::new(NOTES_SIZE, NOTES, Role::Cell)
                .with_content(Content::Value(Reading::plain(Quantity::whole(1177, Unit::Bytes)))),
        ]
    }

    #[test]
    fn a_file_browser_is_expressible() {
        let browser = file_browser();
        check(&browser).expect("a file browser the vocabulary can hold");
        canvases_number(&browser, 0);

        // A column is a position in equal rows, and that is the only thing
        // making *the size column* nameable without a role for a column header.
        let listing = find(&browser, NodeId::new(150)).expect("the listing");
        let widths: [usize; 3] = [151, 160, 170].map(|id| {
            children(&browser, NodeId::new(id)).filter(|cell| cell.role == Role::Cell).count()
        });
        assert_eq!(widths, [2, 2, 2]);
        assert!(
            listing.relations.iter().any(|edge| edge == Relation::LabelledBy(NodeId::new(151))),
            "the table names the row its columns are named by"
        );

        // A size is a quantity in bytes and not a rendered string, so a
        // projection with four characters of room and one with forty both get it
        // right without the application knowing which it is talking to.
        let size = find(&browser, NodeId::new(162)).expect("the report's size");
        let Content::Value(reading) = size.content else { panic!("a size is a quantity") };
        assert_eq!(reading.value.unit, Unit::Bytes);
        assert_eq!(reading.value.scaled, 20418);

        // A row is operable, and so is a cell in the header row: opening a file
        // and sorting a column are both capability invocations, and neither is a
        // gesture a projection has to recognise in order to offer.
        let report = find(&browser, NodeId::new(160)).expect("the selected row");
        assert!(report.state.holds(StateSet::SELECTED));
        assert!(report.intent.is_some());
        assert!(find(&browser, NodeId::new(152)).expect("a header cell").intent.is_some());
    }

    #[test]
    fn a_ragged_table_is_a_defect() {
        // The same tree, one cell shorter. Without the equal-row rule this is a
        // listing with a missing size and no way for anything to know.
        let browser = file_browser();
        let short = &browser[..browser.len() - 1];
        assert_eq!(check(short), Err(Defect::RaggedTable(NodeId::new(150))));
    }

    /// The third hard interface: a timeline, as far as nodes go.
    ///
    /// This is the case section 13 says sank every predecessor, and it is
    /// deliberately only *partly* expressible here. What is nodes is here: the
    /// canvas whose pixels the application draws, the tracks, the clips on them,
    /// the playhead, the transport, and the selection — every one of them
    /// addressable by [`NodeId`] and operable through a capability.
    ///
    /// What is **not** here is where a clip sits, and the split is worth stating
    /// precisely because the last version of this comment quoted section 13's
    /// question while the fixture answered a smaller one. The question is *can
    /// an agent select the clip between 4.2 s and 6.8 s on track 3*. It has two
    /// halves and this decision owns one of them:
    ///
    /// - **Track 3, and the clip on it.** This tree's, and it is here: three
    ///   named tracks under the canvas, each with an occupant, every one of them
    ///   addressable by [`NodeId`] and operable through a capability. The
    ///   fixture had two tracks when that sentence was first written, which made
    ///   the quoted question unanswerable in a way nothing said out loud.
    /// - **Between 4.2 s and 6.8 s.** Not this decision's, and refused by name:
    ///   RFC 0077 refuses a time range on [`Node`], so the ranges, the selection
    ///   over them and the arithmetic that answers *which clip is at this
    ///   instant* are `canvas.rs`'s (`E3-D02`), attached to this tree by node
    ///   identity rather than by a field. That is the whole reason the
    ///   vocabulary can be closed and still grow a neighbour, and it is not this
    ///   module deciding its own scope: `E3-D02`'s exit in `TODO.md` names
    ///   *times* among the four things it owes.
    ///
    /// So what this fixture asserts is the seam: the clips have stable
    /// identities, they are distinct, and the tree says which track each one is
    /// on. `a_timeline_is_expressible_as_far_as_nodes_go` is named for the half
    /// it covers.
    fn timeline() -> [Node; 14] {
        const SURFACE: NodeId = NodeId::new(200);
        const TRANSPORT: NodeId = NodeId::new(210);
        const PLAY: NodeId = NodeId::new(211);
        const STOP: NodeId = NodeId::new(212);
        const ELAPSED: NodeId = NodeId::new(213);
        const ARRANGEMENT: NodeId = NodeId::new(220);
        const DIALOGUE: NodeId = NodeId::new(221);
        const TAKE_THREE: NodeId = NodeId::new(222);
        const TAKE_FOUR: NodeId = NodeId::new(223);
        const MUSIC: NodeId = NodeId::new(224);
        const BED: NodeId = NodeId::new(225);
        const PLAYHEAD: NodeId = NodeId::new(226);
        const EFFECTS: NodeId = NodeId::new(227);
        const STING: NodeId = NodeId::new(228);

        let elapsed_at = Quantity::scaled(4_200, 3, Unit::Seconds);

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
                .with_intent(CapRef::new(0x0201)),
            Node::new(STOP, TRANSPORT, Role::Command)
                .with_content(text("Stop"))
                .with_intent(CapRef::new(0x0202)),
            Node::new(ELAPSED, TRANSPORT, Role::Status)
                .with_content(Content::Value(Reading::plain(elapsed_at))),
            // The canvas. Its pixels are the application's and its content is
            // not: everything below it is declared, which is what stops this
            // being an opaque rectangle and the thesis being true only for
            // toolbars.
            Node::new(ARRANGEMENT, SURFACE, Role::Canvas)
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0203))
                .with_layout(Constraints::flowing(Flow::Own).growing(1)),
            Node::new(DIALOGUE, ARRANGEMENT, Role::Track)
                .with_content(text("Dialogue"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0204)),
            Node::new(TAKE_THREE, DIALOGUE, Role::Clip)
                .with_content(text("take-3"))
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(CapRef::new(0x0205)),
            Node::new(TAKE_FOUR, DIALOGUE, Role::Clip)
                .with_content(text("take-4"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0206)),
            Node::new(MUSIC, ARRANGEMENT, Role::Track)
                .with_content(text("Music"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0207)),
            Node::new(BED, MUSIC, Role::Clip)
                .with_content(text("bed"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0208)),
            // The third track, and it is here for one reason: section 13's
            // question is about *the clip on track 3*, and a fixture with two
            // tracks let this module quote that question while answering a
            // smaller one. The review that found that was right. What the tree
            // owes the question is the addressable half — a third named lane
            // with an occupant on it — and it now has one; the other half, the
            // 4.2 s to 6.8 s, is `canvas.rs`'s and is scoped out by name in
            // this fixture's documentation rather than left to be assumed.
            Node::new(EFFECTS, ARRANGEMENT, Role::Track)
                .with_content(text("Effects"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x020a)),
            Node::new(STING, EFFECTS, Role::Clip)
                .with_content(text("sting"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x020b)),
            Node::new(PLAYHEAD, ARRANGEMENT, Role::Marker)
                .with_content(text("Playhead"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0209)),
        ]
    }

    #[test]
    fn a_timeline_is_expressible_as_far_as_nodes_go() {
        let sequence = timeline();
        check(&sequence).expect("a timeline the vocabulary can hold");
        canvases_number(&sequence, 1);

        // The canvas declares content, so it is not an escape. Said plainly
        // because `check` has already refused the opposite one line up: what is
        // asserted here is not that the tree is clean, it is which node is the
        // canvas and what is under it, which is the part a fixture can silently
        // lose while staying green.
        let canvas = find(&sequence, NodeId::new(220)).expect("the arrangement");
        assert_eq!(canvas.role, Role::Canvas);
        assert!(has_children(&sequence, canvas.id));

        // Three, because section 13's question is about the clip on *track 3*
        // and a two-track fixture cannot be asked it. The third track is the
        // artefact half of that repair; the fixture's documentation is the half
        // that says what is still not here.
        let tracks: usize =
            children(&sequence, canvas.id).filter(|node| node.role == Role::Track).count();
        assert_eq!(tracks, 3);
        let third = children(&sequence, canvas.id)
            .filter(|node| node.role == Role::Track)
            .nth(2)
            .expect("track 3");
        assert_eq!(third.id, NodeId::new(227));
        let on_third: usize =
            children(&sequence, third.id).filter(|node| node.role == Role::Clip).count();
        assert_eq!(on_third, 1, "track 3 with nothing on it cannot be asked the question either");

        // The seam to `canvas.rs`: every clip is addressable, distinct, and on a
        // named track. The time range it occupies is that module's, and nothing
        // here pretends otherwise.
        let clips: [&Node; 4] =
            [222, 223, 225, 228].map(|id| find(&sequence, NodeId::new(id)).expect("a clip"));
        for clip in clips {
            assert_eq!(clip.role, Role::Clip);
            assert!(clip.intent.is_some(), "a clip is operated through a capability");
            let track = find(&sequence, clip.parent).expect("the track it is on");
            assert_eq!(track.role, Role::Track);
        }
        // Pairwise, not neighbour-by-neighbour: `canvas.rs` keys a time range by
        // identity, so two clips sharing one is the failure that would make the
        // seam silently wrong, and a chain of two comparisons could miss it.
        for (at, clip) in clips.iter().enumerate() {
            assert!(
                clips[..at].iter().all(|earlier| earlier.id != clip.id),
                "two clips share an identity"
            );
        }
        assert!(clips[0].state.holds(StateSet::SELECTED));
        assert!(!clips[1].state.holds(StateSet::SELECTED));

        // A screen reader can say *the elapsed time is 4.2 seconds* from the
        // tree alone, with no pixel and no floating-point type anywhere near it.
        let elapsed = find(&sequence, NodeId::new(213)).expect("the elapsed time");
        let Content::Value(reading) = elapsed.content else { panic!("a time is a quantity") };
        assert_eq!(reading.value.unit, Unit::Seconds);
        assert_eq!((reading.value.scaled, reading.value.scale_decimals), (4_200, 3));
    }

    #[test]
    fn a_canvas_that_declares_nothing_is_a_defect() {
        // The timeline with everything under the canvas removed: exactly the
        // opaque rectangle section 13 warns about, and the tree does not pass.
        // This is why *no node marked other* is structural here rather than a
        // convention — the escape hatch that would matter is not a role name, it
        // is a canvas nobody filled in.
        let sequence = timeline();
        let truncated = &sequence[..6];
        assert_eq!(truncated[5].role, Role::Canvas);
        assert_eq!(check(truncated), Err(Defect::EmptyCanvas(NodeId::new(220))));
        assert_eq!(canvas_escapes(truncated), 1);
        // `canvas_escapes(&sequence) == 0` was asserted here and is gone: the
        // full tree passed `check` two tests over, and `check` refuses exactly
        // what the count counts, so the zero was a restatement of the checker
        // rather than a second opinion about the tree. Where the count is made
        // to earn its keep is the next test.
    }

    #[test]
    fn the_escape_count_is_not_the_checker_wearing_a_different_name() {
        // `canvas_escapes` over a checked tree is zero by construction, so the
        // only case that can tell the two apart is a tree nobody checked — which
        // is also the only case the count exists for, since the rate RFC 0077
        // registers is taken over a corpus rather than over a tree somebody has
        // already refused.
        //
        // Two empty canvases. `check` names the first and stops, because a tree
        // with two defects is a tree whose author is still writing it. The count
        // is two, because a report that stopped at the first escape would
        // understate the rate it feeds.
        //
        // *The edit that makes this go red:* giving `canvas_escapes` an early
        // return at its first hit, or widening it to count every canvas rather
        // than every empty one.
        let two = [
            Node::new(NodeId::new(400), NodeId::UNNAMED, Role::Surface),
            Node::new(NodeId::new(401), NodeId::new(400), Role::Canvas),
            Node::new(NodeId::new(402), NodeId::new(400), Role::Canvas),
        ];
        assert_eq!(check(&two), Err(Defect::EmptyCanvas(NodeId::new(401))));
        assert_eq!(canvas_escapes(&two), 2);

        // And the count is about the canvas rather than about the tree's health:
        // a canvas that declared something is not an escape even in a tree that
        // is refused for an unrelated reason, and a tree refused before the
        // walk reached its empty canvas still reports that canvas.
        let mixed = [
            Node::new(NodeId::new(400), NodeId::UNNAMED, Role::Surface),
            Node::new(NodeId::new(401), NodeId::new(400), Role::Canvas),
            Node::new(NodeId::new(403), NodeId::new(401), Role::Track),
            Node::new(NodeId::new(404), NodeId::new(400), Role::Cell),
            Node::new(NodeId::new(402), NodeId::new(400), Role::Canvas),
        ];
        assert_eq!(check(&mixed), Err(Defect::MisplacedChild(NodeId::new(404))));
        assert_eq!(canvas_escapes(&mixed), 1);
    }

    #[test]
    fn the_rate_over_nothing_is_absent_rather_than_clean() {
        // `canvas_census` is the per-tree half of RFC 0077's reversal condition,
        // and the `Option` is the half of *that* which matters: a corpus with no
        // nodes in it has no rate, and reporting zero would report the
        // vocabulary as doing perfectly well on the day nobody had ported
        // anything. `E3-B06l`'s exit asks the corpus-level command for the same
        // refusal.
        //
        // *The edit that makes this go red:* returning `Some((0, 0))` for the
        // empty tree, which is the tidy-up that makes the signature simpler and
        // the metric dishonest.
        assert_eq!(canvas_census(&[]), None);
        assert_eq!(canvas_census(&settings_panel()), Some((0, 15)));
        assert_eq!(canvas_census(&file_browser()), Some((0, 23)));
        assert_eq!(canvas_census(&timeline()), Some((1, 14)));
    }

    #[test]
    fn a_separator_between_rows_does_not_make_a_table_ragged() {
        // The table rule counted the cells of every child of the table, and a
        // separator has none — so a table with a declared boundary between two
        // groups of rows was `RaggedTable`, naming the table for a defect that
        // was neither the table's nor the separator's. A separator under a table
        // is admitted by `accepts_parent` and is the exact case that role's own
        // documentation describes.
        //
        // *The edit that makes this go red:* dropping the `role == Role::Row`
        // filter from `check`'s table arm.
        const TABLE: NodeId = NodeId::new(500);
        const SURFACE: NodeId = NodeId::new(501);
        let tree = [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface),
            Node::new(TABLE, SURFACE, Role::Table),
            Node::new(NodeId::new(510), TABLE, Role::Row),
            Node::new(NodeId::new(511), NodeId::new(510), Role::Cell),
            Node::new(NodeId::new(520), TABLE, Role::Separator),
            Node::new(NodeId::new(530), TABLE, Role::Row),
            Node::new(NodeId::new(531), NodeId::new(530), Role::Cell),
        ];
        check(&tree).expect("a separator is a boundary between rows, not a row of width zero");

        // And the rule still bites, so the fix is a narrowing rather than a
        // switching-off: the same tree with the second row's only cell removed
        // is ragged, separator and all.
        assert_eq!(check(&tree[..tree.len() - 1]), Err(Defect::RaggedTable(TABLE)));
    }

    #[test]
    fn a_state_bit_this_version_does_not_define_is_visible() {
        // `is_known` could not return `false` and nothing called it: every other
        // constructor builds from this version's own constants, so no reachable
        // value could carry an unknown bit. `from_bits` is the decoder's half
        // the predicate's documentation already presupposed, and this is the
        // case it exists for.
        //
        // *The edit that makes this go red:* masking `from_bits` against
        // `KNOWN`, which is the obvious tidy-up and the one that turns the
        // predicate back into a tautology.
        assert!(StateSet::NONE.is_known());
        assert!(StateSet::KNOWN.is_known());
        assert!(StateSet::ENABLED.with(StateSet::INVALID).is_known());

        let later = StateSet::from_bits(StateSet::ENABLED.bits() | (1 << 5));
        assert!(!later.is_known(), "a bit this version does not define went unnoticed");
        assert!(later.holds(StateSet::ENABLED), "the bits it did understand are still there");
        assert_eq!(later.bits() & !StateSet::KNOWN.bits(), 1 << 5, "the evidence was kept");
    }

    #[test]
    fn an_intent_on_an_inert_role_is_a_defect() {
        // A label you can click is a control whose label is itself, which is the
        // ambiguity the vocabulary exists to refuse.
        let mut panel = settings_panel();
        panel[2] = panel[2].with_intent(CapRef::new(0x0058));
        assert_eq!(panel[2].role, Role::Label);
        assert_eq!(check(&panel), Err(Defect::IntentOnInertRole(NodeId::new(11))));
    }

    #[test]
    fn a_node_in_the_wrong_place_is_a_defect() {
        // A clip off a track, and a control with no surface over it. Both are
        // trees a projection would have to guess about.
        let mut sequence = timeline();
        sequence[7] = Node::new(NodeId::new(222), NodeId::new(220), Role::Clip);
        assert_eq!(check(&sequence), Err(Defect::MisplacedChild(NodeId::new(222))));

        let orphan = [Node::new(NodeId::new(300), NodeId::UNNAMED, Role::Command)];
        assert_eq!(check(&orphan), Err(Defect::MisplacedChild(NodeId::new(300))));
    }

    #[test]
    fn a_node_that_names_nothing_is_a_defect() {
        // The hole this closes was a sentence: `NodeId::new`'s documentation
        // said a tree using zero as an identity "fails `check` at its first
        // parent walk". It did not. A node whose *parent* is unnamed is a root,
        // so the walk ran zero times, and this tree passed clean — after which
        // `find(nodes, NodeId::UNNAMED)` answered it, and the relation to
        // nothing that `UNNAMED`'s own documentation says no relation may carry
        // resolved and passed too. Both halves are below.
        //
        // *The edit that makes this go red:* delete the `is_named` guard at the
        // top of `check`'s loop.
        let nameless = [Node::new(NodeId::UNNAMED, NodeId::UNNAMED, Role::Surface)];
        assert_eq!(check(&nameless), Err(Defect::Unnamed { parent: NodeId::UNNAMED }));

        // Not only at the root: a named surface with a nameless child under it.
        let under = [
            Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface),
            Node::new(NodeId::UNNAMED, NodeId::new(1), Role::Group),
        ];
        assert_eq!(check(&under), Err(Defect::Unnamed { parent: NodeId::new(1) }));

        // And the consequence that made the sentence worth repairing rather
        // than deleting: with no node holding zero, an edge to nothing is an
        // edge to a node that was never declared.
        let pointing = [
            Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface),
            related(
                Node::new(NodeId::new(2), NodeId::new(1), Role::Entry),
                &[Relation::LabelledBy(NodeId::UNNAMED)],
            ),
        ];
        assert_eq!(
            check(&pointing),
            Err(Defect::UnknownRelation { node: NodeId::new(2), target: NodeId::UNNAMED })
        );
    }

    #[test]
    fn two_nodes_with_one_identity_is_a_defect() {
        // The stated cost of author-assigned identity: `NodeId`'s documentation
        // says a duplicate becomes a defect rather than an impossibility and
        // that `check` reports it, "which is the trade taken deliberately".
        // Nothing checked that it does.
        //
        // *The edit that makes this go red:* delete the `nodes[..at]` scan in
        // `check`.
        let mut panel = settings_panel();
        panel[3].id = panel[2].id;
        assert_eq!(check(&panel), Err(Defect::DuplicateId(panel[2].id)));
    }

    #[test]
    fn a_parent_that_was_never_declared_is_a_defect() {
        // *The edit that makes this go red:* make `check` treat an unresolvable
        // parent as a root — `None => None` instead of the refusal — which is
        // exactly the shortcut that would make a partial tree look valid.
        let mut panel = settings_panel();
        panel[2].parent = NodeId::new(9_999);
        assert_eq!(
            check(&panel),
            Err(Defect::UnknownParent { node: panel[2].id, parent: NodeId::new(9_999) })
        );
    }

    #[test]
    fn a_relation_to_a_node_nobody_declared_is_a_defect() {
        // A tree may be built one node at a time and an edge may name a node
        // that has not been written yet; `check` is where that stops being
        // acceptable, because a projection resolving `LabelledBy` has nowhere
        // to go. This is the *dangling* case — the relation to nothing is in
        // `a_node_that_names_nothing_is_a_defect`.
        //
        // *The edit that makes this go red:* drop the relation loop from
        // `check`.
        let mut panel = settings_panel();
        panel[3] = related(panel[3], &[Relation::Owns(NodeId::new(4_242))]);
        assert_eq!(
            check(&panel),
            Err(Defect::UnknownRelation { node: panel[3].id, target: NodeId::new(4_242) })
        );
    }

    #[test]
    fn a_chain_deeper_than_the_bound_is_a_defect() {
        // `DEPTH_MAX` says sixteen is deeper than a reader holds in their head,
        // and that the bound is also how `check` finds a cycle without a
        // visited set. Neither half was ever run. Surfaces nest, so the chain
        // can be built from one role and the only thing under test is depth.
        //
        // *The edit that makes this go red:* raise `DEPTH_MAX`. At 64 this
        // chain of eighteen passes and the assertion below it never fires.
        let mut chain = [Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface); DEPTH_MAX + 2];
        for (at, node) in chain.iter_mut().enumerate().skip(1) {
            let id = at as u64 + 1;
            *node = Node::new(NodeId::new(id), NodeId::new(id - 1), Role::Surface);
        }
        // One node shallower is the deepest tree that is still a tree: the last
        // node has exactly `DEPTH_MAX` ancestors.
        check(&chain[..DEPTH_MAX + 1]).expect("exactly the bound");
        assert_eq!(check(&chain), Err(Defect::TooDeep(NodeId::new(DEPTH_MAX as u64 + 2))));
    }

    #[test]
    fn a_cycle_is_the_same_defect_as_too_deep() {
        // `Defect::TooDeep` says *a cycle is a chain that does not end*, and
        // this is the tree that sentence is about: two surfaces, each the
        // other's parent, no root, and a walk that would not terminate if the
        // bound were not there.
        //
        // *The edit that makes this go red:* remove the `walked >= DEPTH_MAX`
        // guard from `check`'s parent walk — this test then hangs rather than
        // failing, which is the honest demonstration of what the bound buys.
        let ouroboros = [
            Node::new(NodeId::new(1), NodeId::new(2), Role::Surface),
            Node::new(NodeId::new(2), NodeId::new(1), Role::Surface),
        ];
        assert_eq!(check(&ouroboros), Err(Defect::TooDeep(NodeId::new(1))));
    }

    /// The same settings panel, redesigned — and **written out rather than
    /// derived**, which is the whole reason it is a function and not three
    /// lines inside the test.
    ///
    /// The test below used to build its *after* tree by calling
    /// [`settings_panel`] a second time, reversing it, and reassigning layout,
    /// style and one parent. Nothing in that transform could change a role, an
    /// intent or a content — so the three assertions saying a redesign had not
    /// changed them were guarantees of the constructor rather than findings
    /// about a redesign. A survival test whose subject cannot die is not a
    /// survival test, and that is the finding that failed this work.
    ///
    /// This is a second authoring of the same fifteen identities: two columns
    /// rather than one, the status promoted out of the output group to the
    /// surface, the rule moved down into it, the reset promoted to a footer
    /// action, every constraint and every token different, and the declaration
    /// order rewritten with children before their parents. What it does not
    /// change is what each node *is*, does, says, how it stands and what it is
    /// joined to — and because fifteen nodes were retyped by hand, every one of
    /// those is now a thing the test can catch this file getting wrong.
    fn settings_panel_redesigned() -> [Node; 15] {
        const SURFACE: NodeId = NodeId::new(1);
        const RULE: NodeId = NodeId::new(2);
        const OUTPUT: NodeId = NodeId::new(10);
        const DEVICE_LABEL: NodeId = NodeId::new(11);
        const DEVICE: NodeId = NodeId::new(12);
        const SPEAKERS: NodeId = NodeId::new(13);
        const HEADPHONES: NodeId = NodeId::new(14);
        const VOLUME_LABEL: NodeId = NodeId::new(15);
        const VOLUME: NodeId = NodeId::new(16);
        const MUTE: NodeId = NodeId::new(17);
        const APPLIED: NodeId = NodeId::new(18);
        const INPUT: NodeId = NodeId::new(20);
        const NAME_LABEL: NodeId = NodeId::new(21);
        const NAME: NodeId = NodeId::new(22);
        const RESET: NodeId = NodeId::new(23);

        let percent = |value: i64| Quantity::whole(value, Unit::Ratio);

        [
            Node::new(RESET, SURFACE, Role::Command)
                .with_content(text("Reset to defaults"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0057))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(500))
                .with_style(styled(&["command", "emphasis"])),
            Node::new(INPUT, SURFACE, Role::Group)
                .with_content(text("Input"))
                .with_layout(Constraints::flowing(Flow::Wrap).growing(2))
                .with_style(styled(&["panel"])),
            related(
                Node::new(NAME, INPUT, Role::Entry)
                    .with_content(text("built-in"))
                    .with_state(StateSet::ENABLED.with(StateSet::INVALID))
                    .with_intent(CapRef::new(0x0056))
                    .with_layout(Constraints::flowing(Flow::Block).at_least(1000).growing(2))
                    .with_style(styled(&["field", "text.danger"])),
                &[Relation::LabelledBy(NAME_LABEL)],
            ),
            Node::new(NAME_LABEL, INPUT, Role::Label)
                .with_content(text("Microphone name"))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(300))
                .with_style(styled(&["text.muted"])),
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface)
                .with_content(text("Sound"))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(2400))
                .with_style(styled(&["surface-2"])),
            Node::new(APPLIED, SURFACE, Role::Status)
                .with_content(text("Applied"))
                .with_layout(Constraints::flowing(Flow::Inline).growing(1))
                .with_style(styled(&["status", "emphasis"])),
            related(
                Node::new(MUTE, OUTPUT, Role::Toggle)
                    .with_content(text("Mute"))
                    .with_state(StateSet::ENABLED)
                    .with_intent(CapRef::new(0x0055))
                    .with_layout(Constraints::flowing(Flow::Inline).at_least(400))
                    .with_style(styled(&["emphasis"])),
                &[Relation::Controls(VOLUME)],
            ),
            related(
                Node::new(VOLUME, OUTPUT, Role::Number)
                    .with_content(Content::Value(
                        Reading::bounded(percent(70), percent(0), percent(100)).stepped(percent(5)),
                    ))
                    .with_state(StateSet::ENABLED)
                    .with_intent(CapRef::new(0x0054))
                    .with_layout(Constraints::flowing(Flow::Block).growing(3))
                    .with_style(styled(&["slider"])),
                &[Relation::LabelledBy(VOLUME_LABEL)],
            ),
            Node::new(VOLUME_LABEL, OUTPUT, Role::Label)
                .with_content(text("Volume"))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(300))
                .with_style(styled(&["text.muted"])),
            Node::new(HEADPHONES, DEVICE, Role::Item)
                .with_content(text("Headphones"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0053))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(200))
                .with_style(styled(&["item"])),
            Node::new(SPEAKERS, DEVICE, Role::Item)
                .with_content(text("Speakers"))
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(CapRef::new(0x0052))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(200))
                .with_style(styled(&["item"])),
            related(
                Node::new(DEVICE, OUTPUT, Role::Choice)
                    .with_state(StateSet::ENABLED)
                    .with_intent(CapRef::new(0x0051))
                    .with_layout(Constraints::flowing(Flow::Inline).at_most(1800))
                    .with_style(styled(&["field"])),
                &[Relation::LabelledBy(DEVICE_LABEL)],
            ),
            Node::new(DEVICE_LABEL, OUTPUT, Role::Label)
                .with_content(text("Device"))
                .with_layout(Constraints::flowing(Flow::Inline).at_least(300))
                .with_style(styled(&["text.muted"])),
            Node::new(RULE, OUTPUT, Role::Separator)
                .with_layout(Constraints::flowing(Flow::Block).at_least(100))
                .with_style(styled(&["rule"])),
            Node::new(OUTPUT, SURFACE, Role::Group)
                .with_content(text("Output"))
                .with_layout(Constraints::flowing(Flow::Wrap).growing(1))
                .with_style(styled(&["panel"])),
        ]
    }

    #[test]
    fn identity_survives_a_redesign() {
        // `E3-P05`'s exit in miniature: the same application, visually
        // reorganised, and every assertion anybody wrote against it still names
        // the same thing. What makes it a test rather than a demonstration is
        // that the *after* tree was authored rather than transformed — see
        // `settings_panel_redesigned` for what the transform could not fail.
        //
        // *The edit that makes this go red:* any role, content, intent, state or
        // relation mistyped in that fixture — a `Role::Command` written as a
        // `Role::Toggle`, a capability transposed, a label reworded, a
        // `Relation::Controls` pointed at the wrong node. None of those was
        // visible to the version this replaced.
        let before = settings_panel();
        let after = settings_panel_redesigned();

        check(&before).expect("the tree before");
        check(&after).expect("the tree after");
        assert_eq!(before.len(), after.len());
        assert_ne!(
            before.map(|node| node.id),
            after.map(|node| node.id),
            "a redesign that kept its declaration order is not the thing being tested"
        );

        let mut reparented = 0;
        for node in &before {
            let same = find(&after, node.id).expect("a node the redesign dropped");
            assert_eq!(same.role, node.role, "a redesign changed what a node is");
            assert_eq!(same.intent, node.intent, "a redesign changed what a node does");
            assert_eq!(same.content, node.content, "a redesign changed what a node says");
            assert_eq!(same.state, node.state, "a redesign changed how a node stands");
            assert_eq!(same.relations, node.relations, "a redesign changed a node's edges");

            // And it is a redesign. Every node is laid out differently, or
            // wearing different tokens, or both: a node copied across unchanged
            // is a node this test would not be testing anything about.
            assert!(
                (same.layout, same.style) != (node.layout, node.style),
                "{} came through the redesign untouched",
                node.role.name()
            );
            if same.parent != node.parent {
                reparented += 1;
            }
        }
        assert_eq!(reparented, 3, "the status, the rule and the reset each changed parent");
    }

    #[test]
    fn the_token_bound_is_four_times_the_corpus() {
        // `TOKENS_MAX`'s documentation has now made a claim about this number
        // twice and been wrong twice — first that a case in the corpus pushed on
        // the bound, then that the corpus reached two. The observation is here
        // instead, where a fixture that changes says so.
        //
        // The corpus is the three interfaces the exit is about, which is the
        // same three `the_three_interfaces_reach_most_of_the_vocabulary` chains:
        // the redesign fixture is a re-authoring of the first and no test counts
        // it as a fourth interface. It is measured separately because it is
        // where the only two-token nodes in this file are, and the previous
        // sentence's "two" came from reading it.
        //
        // *The edit that makes this go red:* change `TOKENS_MAX`. The sentence
        // this guards is about the ratio between the bound and the corpus, so
        // moving either end has to move the prose.
        let panel = settings_panel();
        let browser = file_browser();
        let sequence = timeline();
        let corpus = panel.iter().chain(browser.iter()).chain(sequence.iter());

        let widest = corpus.clone().map(|node| node.style.len()).max().expect("nodes");
        assert_eq!(widest, 1, "the corpus's widest token set moved; TOKENS_MAX's doc says one");
        assert_eq!(corpus.filter(|node| !node.style.is_empty()).count(), 4, "four styled nodes");
        assert!(
            browser.iter().chain(sequence.iter()).all(|node| node.style.is_empty()),
            "two of the three interfaces style nothing at all, and the doc says so"
        );

        let redesigned = settings_panel_redesigned();
        let widest_anywhere = redesigned.iter().map(|node| node.style.len()).max().expect("nodes");
        assert_eq!(widest_anywhere, 2, "the only two-token nodes in this file are the redesign's");

        assert_eq!(TOKENS_MAX, 4 * widest, "the bound is four times what the corpus needed");
    }

    #[test]
    fn the_three_interfaces_reach_most_of_the_vocabulary() {
        // The size argument, asserted rather than asserted-in-prose. Between
        // them the three trees use twenty of the twenty-two roles. Two are
        // unreached — `List` and `Text` — and both are there because a fourth
        // interface reaches for them immediately: a menu is a list of operable
        // items, and a document viewer is paragraphs. A vocabulary where this
        // number were much lower would be a vocabulary sized for three examples,
        // which is how *too large* arrives quietly; one where it were twenty-two
        // would mean nothing was being held in reserve for the interfaces that
        // have not been tried yet, which is how *too small* does.
        let panel = settings_panel();
        let browser = file_browser();
        let sequence = timeline();

        let mut seen = [false; Role::COUNT];
        for node in panel.iter().chain(browser.iter()).chain(sequence.iter()) {
            seen[node.role.index()] = true;
        }
        let used = seen.iter().filter(|touched| **touched).count();
        assert_eq!(used, 20, "the corpus moved; the size argument moved with it");
        assert!(!seen[Role::List.index()]);
        assert!(!seen[Role::Text.index()]);

        // How many of the three reach each role. The module documentation used
        // to describe the selection criterion as *every role only one of the
        // three needed was refused*, which this census contradicts flatly:
        // thirteen of the twenty are reached by exactly one tree. That is what
        // three interfaces chosen for three *different* pressures produce —
        // mostly disjoint parts of the vocabulary over a common spine of four
        // roles — and it is not a defect; but the prose said the opposite of the
        // code two hundred lines above it, so the four numbers are asserted here
        // and the prose now points at them.
        //
        // *The edit that makes this go red:* moving a role between fixtures —
        // giving the browser a toggle, say — which changes the shape of the
        // argument the module documentation makes and should not be possible to
        // do silently.
        let trees = [&panel[..], &browser[..], &sequence[..]];
        let reach = |role: Role| trees.iter().filter(|t| t.iter().any(|n| n.role == role)).count();
        let reached_by = |times: usize| Role::ALL.iter().filter(|r| reach(**r) == times).count();
        assert_eq!(reached_by(0), 2, "List and Text, held in reserve");
        assert_eq!(reached_by(1), 13, "a role one interface needs is the common case here");
        assert_eq!(reached_by(2), 3);
        assert_eq!(reached_by(3), 4, "Surface, Group, Status, Command: what every tree has");
    }
}
