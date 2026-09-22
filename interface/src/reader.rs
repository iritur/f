// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The screen-reader projection: a whole declared tree, said, with nothing
//! rendered.
//!
//! `canvas.rs` linearises **one arrangement** — a canvas, its lanes, and what is
//! on them — and it is the narrowest thing that could answer section 13's
//! question. This module is the same walk over the **whole tree**: a surface,
//! its groups, its tables and its canvases in one reading, so that the sentence
//! *a consumer that renders nothing can say what this application is* is about
//! an application and not about its timeline.
//!
//! Three things had to be decided to widen it, and each is where this module
//! differs from its sibling rather than repeating it.
//!
//! # A role is said, and the saying is an exhaustive `match`
//!
//! [`phrasing`] answers *how does a reader say a node of this role* for all
//! twenty-two roles, with **no wildcard arm and no table**. That is the whole
//! mechanism this module is accepted on, and it is worth saying why the two are
//! not interchangeable.
//!
//! **A table is not simply worse, and the difference has to be stated exactly**,
//! because the loose version of this argument is false and `scene/src/kind.rs`
//! measured the true version from the other side. A **written-out**
//! `[&str; Role::COUNT]` of twenty-two entries is the wrong length the day the
//! vocabulary has twenty-three, and `error[E0308]` says so: that table is a
//! guard, and saying it is not would be this module flattering its own choice.
//!
//! It has two holes a `match` does not. It does not see a **reorder** — two
//! roles swapped inside one version keep the length, every phrase from there on
//! shifts by one, no build fails, no test that iterates the array fails, and a
//! listener is told that the toggle is a button. `abi`'s frozen digest over
//! version 1's names is what catches that, and it catches it in another crate,
//! on another day, naming a number rather than this file. And a table that is
//! **not** written out — `Role::ALL.map(…)`, or anything else sized from
//! `COUNT` — grows to twenty-three silently and announces the new role as its
//! neighbour, which is the hole `scene/src/kind.rs` names and has three
//! instances of in its own crate.
//!
//! A `match` with no wildcard has neither hole. An addition is
//! `error[E0004]`: *non-exhaustive patterns*, naming the variant nobody has
//! decided about, in the file that has to decide. A reorder is nothing at all,
//! because an arm names its role rather than counting to it. That is why the
//! exit of `E3-B06i` asks for a match twice and for a table never.
//!
//! **Measured rather than asserted**, because a reader auditing this module
//! will audit the number and a number in a paragraph headed *measured* has to
//! have been. A twenty-third role — `Gauge`, added to the `vocabulary!`
//! invocation in `interface/src/node.rs` and taken out again — stops this
//! crate's build at three sites, of which this file is one:
//! `error[E0004]: non-exhaustive patterns: 'Role::Gauge' not covered` at
//! `interface/src/reader.rs:196`, the match in [`phrasing`]. The other two are
//! the two exhaustive matches in `interface/src/token.rs`, whose lines are not
//! quoted here because that file is not this one's to keep accurate. All three
//! are matches; there is no fourth site, because no file in this crate holds a
//! per-role table of either kind.
//!
//! One site in this file and not two, deliberately: a second per-role decision
//! here would be a second place to forget, and everything a reading says about
//! a role comes out of the one [`Phrasing`] this returns.
//!
//! What no `match` can do is refuse a *future* wildcard: `_ => Phrasing { .. }`
//! compiles, and Rust offers no way to forbid one over an enum this file does
//! not own. So the arm count and the absence of a wildcard are read out of this
//! file's own source by
//! `the_projection_decides_every_role_and_carries_no_wildcard`, in the shape
//! `abi/src/semantic.rs` already reads the vocabulary out of `node.rs`.
//!
//! # This module says English words, and `canvas.rs` deliberately does not
//!
//! `canvas.rs` refuses to emit a sentence, on the grounds that a module emitting
//! English is one every other locale has to work around. That argument is
//! correct about a *linearisation*, which is what a sentence is assembled from,
//! and it does not survive contact with the thing being built here: a screen
//! reader that never says a word is not a screen reader. Somebody has to own the
//! nouns, and a projection is the layer that is allowed to.
//!
//! So this module owns them, and the boundary is drawn where it can be moved.
//! [`Phrasing::noun`] is a `&'static str` in one language; everything else a
//! consumer needs — which node, what role, how deep, how it stands, when it is,
//! what may be invoked on it — is data with no language in it. A second locale
//! is therefore this one function answering with a different noun for the same
//! three decisions, and not a second projection.
//!
//! *What would reverse this:* a second locale in the tree. At that point the
//! noun stops being a literal here and becomes a key a locale resolves, and the
//! thing this module must keep is the `match` — a key table indexed by ordinal
//! is the defect above, wearing an internationalisation argument.
//!
//! # A canvas that no arrangement is about is refused
//!
//! The reading of a clip is its identity, its role, its words and **when it is**,
//! and the last of those lives in an [`Arrangement`] rather than in the node —
//! `node.rs` says why, and that separation is what keeps the vocabulary closed.
//! A whole-tree reader therefore has to be handed the tree *and* the
//! arrangements over its canvases, and the interesting case is the one where it
//! is handed fewer arrangements than the tree has canvases.
//!
//! Saying the canvas anyway, with its clips [`Spoken::Untimed`], is the opaque
//! rectangle arriving through the reader: every clause of RFC 0078's floor was
//! met by somebody, and the projection quietly dropped the half that made
//! meeting them worth anything. So [`Unreadable::Unarranged`] refuses the whole
//! reading. The price is named rather than hidden: **a consumer that wants to
//! read the chrome of an application whose canvas it has not been handed cannot
//! have it.** That is the trade `Arrangement::admit` already makes, one layer
//! out, and RFC 0078's reversal condition covers it — the first consumer for
//! which a partial reading is worth more than a refusal.
//!
//! # What a reading does not carry, and why
//!
//! It does not carry *is this inside the canvas's selected stretch*.
//! `canvas.rs`'s [`Phrase`](crate::canvas::Phrase) does, and it is right to:
//! that value is handed out by a canvas, in answer to a question about that
//! canvas. A reading of a whole tree is not. A selected stretch moves without
//! any node changing, so a reading that copied it would hold a second copy of a
//! fact with a shorter life than the reading — and the consumer that wants it
//! has the canvas and `Participating::selected`, which is the one place it is
//! authoritative.
//!
//! [`Utterance::when`] is carried and is not the same kind of fact: where a clip
//! sits is a property of that clip, and no selection changes it.
//!
//! *What would reverse this:* a consumer that has a reading and cannot get at
//! the canvas — a reading sent somewhere, which is the day this crate has a wire
//! and the answer is the wire's rather than this struct's.

use crate::canvas::{Arrangement, Escape, Operable, SPOKEN_DECIMALS, Span, Spoken};
use crate::node::{Content, Defect, Node, NodeId, Role, StateSet, check, children};

/// How a reader says a node of one role.
///
/// Three decisions, and a twenty-third role has to make all three because there
/// is nowhere to put a role that has not. They are three rather than one
/// because *what it is called* is the only one a table could have carried:
/// whether a reader enters a node and whether it counts it among its siblings
/// change the **shape** of the reading, and a projection that took them from
/// somewhere other than the role would be deciding them per node, which is how
/// two lists come to be read differently in one application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Phrasing {
    /// What a reader calls it.
    ///
    /// Deliberately not [`Role::name`](crate::node::Role::name), which is the
    /// wire's spelling and belongs to the vocabulary. *surface* is what the
    /// vocabulary calls the thing an application declares; *window* is what a
    /// listener is told, and three of the twenty-two differ for exactly that
    /// reason.
    pub noun: &'static str,
    /// Whether a reader enters it and leaves it, rather than saying it in one
    /// line.
    ///
    /// A container a listener is inside is one they can be told they have left,
    /// and without the second line a nested tree collapses into a run of names
    /// at unrelated depths. It is a property of the role and not of whether the
    /// node happens to have children: an empty group announced and closed is a
    /// listener learning the group is empty, which is a fact about the
    /// application rather than a gap in the reading.
    pub entered: bool,
    /// Whether a reader says where it sits among the siblings that share its
    /// role.
    ///
    /// *Third of three* is the difference between a list a listener can
    /// navigate and one they can only hear. It is off for a [`Role::Clip`] on
    /// purpose: a clip's place is **when** it is, declaration order is not time
    /// order — `canvas.rs` says so and refuses to sort — so counting clips
    /// would hand a listener an ordinal that means nothing they can act on.
    pub counted: bool,
}

/// How a reader says a node of `role`.
///
/// # Why this is a function here and not a method on `Role`
///
/// Because the noun is the projection's and not the vocabulary's. A
/// `Role::noun` would put one language into the crate's closed list, where every
/// other projection would inherit it and no locale could replace it — and it
/// would put the thing that is allowed to change into the file whose whole worth
/// is that it does not. `interface/src/node.rs` answers what a role *is*; this
/// answers what this one reader *says*, and the day there is a second reader it
/// gets its own function rather than an argument to this one.
///
/// # The twenty-two decisions
///
/// The shapes do not follow the families, which is why each line is written out
/// rather than derived from [`Role::family`](crate::node::Role::family).
/// Structure is mostly entered — a listener is inside a group, a list, a table
/// — and is not entered at [`Role::Item`], [`Role::Row`] and [`Role::Cell`],
/// which are counted instead: *item 3 of 7* is what a listener navigates by,
/// and *entering item, leaving item* around it is noise that doubles the
/// reading. Content is flat and uncounted throughout. Control is flat except
/// [`Role::Choice`], which holds items and is therefore a place a listener is
/// inside. Arrangement is entered at the canvas and the lane, and flat at what
/// is placed on them.
#[must_use]
pub const fn phrasing(role: Role) -> Phrasing {
    match role {
        Role::Surface => Phrasing { noun: "window", entered: true, counted: false },
        Role::Group => Phrasing { noun: "group", entered: true, counted: false },
        Role::List => Phrasing { noun: "list", entered: true, counted: false },
        Role::Tree => Phrasing { noun: "tree", entered: true, counted: false },
        Role::Item => Phrasing { noun: "item", entered: false, counted: true },
        Role::Table => Phrasing { noun: "table", entered: true, counted: false },
        Role::Row => Phrasing { noun: "row", entered: false, counted: true },
        Role::Cell => Phrasing { noun: "cell", entered: false, counted: true },
        Role::Separator => Phrasing { noun: "separator", entered: false, counted: false },
        Role::Label => Phrasing { noun: "label", entered: false, counted: false },
        Role::Text => Phrasing { noun: "text", entered: false, counted: false },
        Role::Image => Phrasing { noun: "image", entered: false, counted: false },
        Role::Status => Phrasing { noun: "status", entered: false, counted: false },
        Role::Command => Phrasing { noun: "button", entered: false, counted: false },
        Role::Toggle => Phrasing { noun: "toggle", entered: false, counted: false },
        Role::Entry => Phrasing { noun: "text field", entered: false, counted: false },
        Role::Choice => Phrasing { noun: "choice", entered: true, counted: false },
        Role::Number => Phrasing { noun: "number", entered: false, counted: false },
        Role::Canvas => Phrasing { noun: "canvas", entered: true, counted: false },
        Role::Track => Phrasing { noun: "track", entered: true, counted: true },
        Role::Clip => Phrasing { noun: "clip", entered: false, counted: false },
        Role::Marker => Phrasing { noun: "marker", entered: false, counted: false },
    }
}

/// Where a node sits among the siblings that share its role.
///
/// **Not a position**, which in this crate would be a coordinate and is the one
/// thing the vocabulary refuses to express. It is an ordinal over a set the
/// declaration already fixes: the children of one parent, filtered to one role,
/// in declaration order. *Track 3 of 3* is a thing a listener and an agent can
/// both say, and it survives every redesign that does not change the order the
/// author declared.
///
/// Siblings of the **same role**, and not all siblings, because otherwise a
/// marker declared between two lanes would make the third lane the fourth
/// child, and *track 3* — which is section 13's own phrase — would name the
/// wrong lane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Among {
    /// Which one it is, counting from one, because a listener counts from one.
    /// Unit: none — an ordinal among siblings of one role.
    pub nth: u16,
    /// How many there are.
    /// Unit: none — a count of siblings of one role.
    pub of: u16,
}

/// Which part of a node's reading one line is.
///
/// Two lines for a node a reader enters and one for everything else, which is
/// what [`Phrasing::entered`] decides. Both lines carry the same identity and
/// the same everything else, so a consumer that lost its place recovers it from
/// either — and a consumer that wants only the content can drop every
/// [`Leaving`](Self::Leaving) without losing a fact.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    /// The whole of what is said about a node the reader does not enter.
    Whole,
    /// The reader is entering a node that holds others.
    Entering,
    /// The reader has finished with it.
    Leaving,
}

/// One line of a reading.
///
/// It carries what `canvas.rs`'s [`Phrase`](crate::canvas::Phrase) carries and
/// three things more — the [`Phrasing`], the [`Part`], and where the node sits
/// among its like siblings — because those three are what a whole tree needs
/// and one canvas does not. A canvas's reading is a canvas, its lanes and its
/// occupants, and a consumer can infer the shape from three roles; a surface's
/// reading is every role there is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Utterance {
    /// Which node this is about. Stable, so an agent can address what it heard.
    pub node: NodeId,
    /// What it is.
    pub role: Role,
    /// How a reader says a node of that role.
    pub phrasing: Phrasing,
    /// Which part of this node's reading this line is.
    pub part: Part,
    /// What it holds, whole and unconverted, for
    /// [`crate::canvas::Phrase`]'s reason.
    pub content: Content,
    /// How deep under its root it sits. A root is zero.
    pub depth: u8,
    /// Where it sits among the siblings that share its role, when the role is
    /// one a reader counts.
    pub among: Option<Among>,
    /// How it stands, as the node declares it.
    pub state: StateSet,
    /// When it is, for a node some canvas of this tree places.
    pub when: Spoken,
    /// What may be invoked on it. Carried so that a consumer which heard the
    /// tree can act on what it heard without walking it again.
    pub operable: Operable,
}

impl Utterance {
    /// A line nobody said: what a caller fills a buffer with before handing it
    /// to [`Reader::read`].
    ///
    /// [`Role::Surface`] for [`crate::canvas::Phrase::UNSAID`]'s reason — the
    /// vocabulary has no *no role* and this crate will not invent one for a
    /// filler. The slots past what `read` returns are not lines at all.
    pub const UNSAID: Self = Self {
        node: NodeId::UNNAMED,
        role: Role::Surface,
        phrasing: phrasing(Role::Surface),
        part: Part::Whole,
        content: Content::None,
        depth: 0,
        among: None,
        state: StateSet::NONE,
        when: Spoken::Untimed,
        operable: Operable::Inert,
    };
}

/// Why a tree cannot be read.
///
/// The two floors below this one are carried whole rather than restated:
/// [`Vocabulary`](Self::Vocabulary) is `node.rs`'s refusal and
/// [`Canvas`](Self::Canvas) is `canvas.rs`'s. A reader that paraphrased either
/// would be a third opinion about a tree, and the two that exist are the ones
/// with the arguments behind them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// The tree does not pass the vocabulary's own check.
    Vocabulary(Defect),
    /// A canvas of this tree is an escape, or a time it places cannot be said.
    Canvas(Escape),
    /// A [`Role::Canvas`] in the tree that no arrangement is about. See the
    /// module's *A canvas that no arrangement is about is refused*.
    Unarranged(NodeId),
    /// Two arrangements about one canvas. Two answers to *when is everything on
    /// this* is no answer, and which one won would depend on argument order.
    ArrangedTwice(NodeId),
    /// A reading was offered less room than the tree needs. Refused rather than
    /// truncated, for [`Escape::NoRoom`]'s reason: a description that stops
    /// halfway says the application ends where the buffer did.
    NoRoom,
}

/// A tree that can be read, and the only thing here that produces a reading.
///
/// `canvas.rs`'s `Participating` shape, applied to a whole tree: the only way to
/// obtain one is [`Reader::of`], so a reading is evidence that the vocabulary's
/// check passed, that every canvas met RFC 0078's floor, and that no canvas was
/// left without a dimension to be placed in. There is no way to ask this module
/// to say a tree that has not.
#[derive(Clone, Copy, Debug)]
pub struct Reader<'a> {
    /// The declared tree, checked.
    nodes: &'a [Node],
    /// The arrangements over its canvases: each admitted, one per canvas.
    canvases: &'a [Arrangement],
}

impl<'a> Reader<'a> {
    /// Meet a tree and the arrangements over its canvases, and be admitted or
    /// refused.
    ///
    /// # Errors
    ///
    /// [`Unreadable::Vocabulary`] when the tree is not one;
    /// [`Unreadable::Canvas`] when an arrangement does not meet RFC 0078's
    /// floor against it; [`Unreadable::ArrangedTwice`] when two arrangements
    /// name one canvas; and [`Unreadable::Unarranged`] for a canvas no
    /// arrangement is about.
    ///
    /// The first refusal found, for [`check`]'s reason: a declaration with two
    /// faults is one somebody is still writing.
    pub fn of(nodes: &'a [Node], canvases: &'a [Arrangement]) -> Result<Self, Unreadable> {
        if let Err(defect) = check(nodes) {
            return Err(Unreadable::Vocabulary(defect));
        }
        for (at, arrangement) in canvases.iter().enumerate() {
            if let Err(escape) = arrangement.admit(nodes) {
                return Err(Unreadable::Canvas(escape));
            }
            let canvas = arrangement.canvas();
            if canvases[..at].iter().any(|earlier| earlier.canvas() == canvas) {
                return Err(Unreadable::ArrangedTwice(canvas));
            }
        }
        for node in nodes.iter().filter(|node| node.role == Role::Canvas) {
            if !canvases.iter().any(|arrangement| arrangement.canvas() == node.id) {
                return Err(Unreadable::Unarranged(node.id));
            }
        }
        Ok(Self { nodes, canvases })
    }

    /// The tree being read.
    #[must_use]
    pub const fn nodes(&self) -> &'a [Node] {
        self.nodes
    }

    /// How many lines a full reading takes.
    ///
    /// Call it to size the buffer [`read`](Self::read) is given. It is a
    /// function of the roles alone — one line, and a second for a role a reader
    /// enters — which is worth stating because it is what makes a reading's
    /// length predictable from a declaration nobody has walked. It is a walk
    /// rather than a stored count for `Participating::phrases`' reason: the tree
    /// is the authority on what is in it, and this module keeps no second copy.
    #[must_use]
    pub fn lines(&self) -> usize {
        self.nodes.iter().map(|node| 1 + usize::from(phrasing(node.role).entered)).sum()
    }

    /// Say the whole tree, in declaration order, into `into`; answer how many
    /// lines that took.
    ///
    /// # What reading order is
    ///
    /// Depth first, in the order the author declared, starting at every root —
    /// `canvas.rs`'s order, widened to the tree and for its reason. Nothing here
    /// sorts anything, and in particular nothing puts the clips of three lanes
    /// into time order: a timeline read chronologically interleaves its lanes
    /// and leaves a listener with three lanes' worth of clips and no structure
    /// to hang them on. Every line carries its own [`Spoken`] time, so a
    /// consumer that wants the other order can have it, with room this crate
    /// does not have to put the result in.
    ///
    /// Every node is said exactly once, which rests on [`check`] having refused
    /// a parent chain that does not reach a root: the roots reach everything,
    /// and nothing is reached twice.
    ///
    /// # Errors
    ///
    /// [`Unreadable::NoRoom`] when `into` is shorter than
    /// [`lines`](Self::lines), and [`Unreadable::Canvas`] carrying
    /// [`Escape::Unsayable`] for a time that will not fit [`SPOKEN_DECIMALS`].
    pub fn read(&self, into: &mut [Utterance]) -> Result<usize, Unreadable> {
        let mut at = 0;
        for root in children(self.nodes, NodeId::UNNAMED) {
            self.say(root, 0, into, &mut at)?;
        }
        Ok(at)
    }

    /// One node and everything under it, said.
    fn say(
        &self,
        node: &Node,
        depth: u8,
        into: &mut [Utterance],
        at: &mut usize,
    ) -> Result<(), Unreadable> {
        let phrasing = phrasing(node.role);
        let line = Utterance {
            node: node.id,
            role: node.role,
            phrasing,
            part: if phrasing.entered { Part::Entering } else { Part::Whole },
            content: node.content,
            depth,
            among: if phrasing.counted { Some(self.among(node)) } else { None },
            state: node.state,
            when: self.when(node.id)?,
            operable: Operable::of(node),
        };
        push(into, at, line)?;
        for child in children(self.nodes, node.id) {
            self.say(child, depth + 1, into, at)?;
        }
        if phrasing.entered {
            push(into, at, Utterance { part: Part::Leaving, ..line })?;
        }
        Ok(())
    }

    /// Where `node` sits among the siblings that share its role.
    fn among(&self, node: &Node) -> Among {
        let mut nth = 0;
        let mut of = 0;
        for sibling in children(self.nodes, node.parent).filter(|s| s.role == node.role) {
            of += 1;
            if sibling.id == node.id {
                nth = of;
            }
        }
        Among { nth, of }
    }

    /// When `node` is, in the unit a description speaks.
    ///
    /// At most one arrangement places it, and that is a property of admission
    /// rather than an assumption: `Arrangement::admit` refuses a placement of a
    /// node that is not a clip or a marker **of its own canvas**, and the
    /// vocabulary lets a clip sit on exactly one lane and a lane on exactly one
    /// canvas. So the first arrangement that places it is the only one that
    /// does.
    fn when(&self, node: NodeId) -> Result<Spoken, Unreadable> {
        let placed = self
            .canvases
            .iter()
            .find_map(|arrangement| Some((arrangement, arrangement.placement_of(node)?)));
        let Some((arrangement, placement)) = placed else { return Ok(Spoken::Untimed) };
        let base = arrangement.base();
        let said = match placement.span {
            Span::At(instant) => base.seconds_from_ticks(instant, SPOKEN_DECIMALS).map(Spoken::At),
            Span::Between(extent) => {
                let from = base.seconds_from_ticks(extent.start(), SPOKEN_DECIMALS);
                let to = base.seconds_from_ticks(extent.end(), SPOKEN_DECIMALS);
                match (from, to) {
                    (Some(from), Some(to)) => Some(Spoken::Between(from, to)),
                    _ => None,
                }
            }
        };
        said.ok_or(Unreadable::Canvas(Escape::Unsayable(node)))
    }
}

/// One line into the buffer, or the refusal that there is no room for it.
fn push(into: &mut [Utterance], at: &mut usize, line: Utterance) -> Result<(), Unreadable> {
    if *at >= into.len() {
        return Err(Unreadable::NoRoom);
    }
    into[*at] = line;
    *at += 1;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{Extent, Placement, Selection, Ticks, TimeBase};
    use crate::node::{CapRef, Constraints, Flow, Quantity, Text, Unit, find, has_children};

    const SURFACE: NodeId = NodeId::new(400);
    const TRANSPORT: NodeId = NodeId::new(410);
    const PLAY: NodeId = NodeId::new(411);
    const SEQUENCE: NodeId = NodeId::new(420);
    const DIALOGUE: NodeId = NodeId::new(421);
    const AMBIENCE: NodeId = NodeId::new(422);
    const MUSIC: NodeId = NodeId::new(423);
    const BED: NodeId = NodeId::new(424);
    const FOLEY: NodeId = NodeId::new(425);
    const STEPS: NodeId = NodeId::new(426);
    const DOOR: NodeId = NodeId::new(427);
    const RAIN: NodeId = NodeId::new(428);
    const PLAYHEAD: NodeId = NodeId::new(429);

    /// The canvas's own rendering: the pixels the application draws, named and
    /// never resolved anywhere in this module or its tests.
    const RENDERING: CapRef = CapRef::new(0x0400);
    /// What operating the door slam does.
    const DOOR_INTENT: CapRef = CapRef::new(0x0409);
    /// What operating the rain would do, if the application were letting it.
    const RAIN_INTENT: CapRef = CapRef::new(0x040a);

    fn text(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits within TEXT_MAX"))
    }

    /// A time in tenths of a second, as ticks of the canvas's base.
    ///
    /// The scale is in the name because here the writer knows what is being
    /// counted: these are the numbers section 13's sentence is written in, and
    /// `42` is 4.2 s. There is no floating-point value in this file and RFC
    /// 0004 leaves no other option.
    fn seconds_x10(value: i64) -> Ticks {
        TimeBase::FLICKS.ticks_from_seconds(value, 1).expect("a time this base can name")
    }

    /// A stretch, in tenths of a second.
    fn span_x10(from: i64, to: i64) -> Extent {
        Extent::new(seconds_x10(from), seconds_x10(to)).expect("a span that advances")
    }

    /// RFC 0078's timeline, with the chrome a whole-tree reading has and a
    /// canvas's own linearisation does not: a surface, a transport with a
    /// button on it, and then the canvas.
    ///
    /// Three lanes, because the question is about *track 3* and a fixture with
    /// two would let the answer come back by having nowhere else to go. `Music`
    /// carries a clip over exactly the answer's stretch for the same reason.
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
                .with_intent(CapRef::new(0x0401)),
            Node::new(SEQUENCE, SURFACE, Role::Canvas)
                .with_content(Content::Media(RENDERING))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0402))
                .with_layout(Constraints::flowing(Flow::Own).growing(1)),
            Node::new(DIALOGUE, SEQUENCE, Role::Track).with_content(text("Dialogue")),
            Node::new(AMBIENCE, DIALOGUE, Role::Clip).with_content(text("ambience")),
            Node::new(MUSIC, SEQUENCE, Role::Track).with_content(text("Music")),
            Node::new(BED, MUSIC, Role::Clip).with_content(Content::Media(CapRef::new(0x04b0))),
            Node::new(FOLEY, SEQUENCE, Role::Track).with_content(text("Foley")),
            Node::new(STEPS, FOLEY, Role::Clip).with_content(text("steps")),
            Node::new(DOOR, FOLEY, Role::Clip)
                .with_content(text("door-slam"))
                .with_state(StateSet::ENABLED.with(StateSet::SELECTED))
                .with_intent(DOOR_INTENT),
            Node::new(RAIN, FOLEY, Role::Clip)
                .with_content(text("rain"))
                .with_state(StateSet::BUSY)
                .with_intent(RAIN_INTENT),
            Node::new(PLAYHEAD, SEQUENCE, Role::Marker).with_content(text("Playhead")),
        ]
    }

    /// The same timeline, as an arrangement: RFC 0078's four clauses, met.
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

    /// The line a reading gives about one node, whichever part it is.
    fn line_for(said: &[Utterance], node: NodeId) -> Utterance {
        *said.iter().find(|line| line.node == node).expect("the reading names it")
    }

    /// Where a node's first line sits in a reading.
    fn at_of(said: &[Utterance], node: NodeId) -> usize {
        said.iter().position(|line| line.node == node).expect("the reading names it")
    }

    // ---- The vocabulary is decided here, and a twenty-third role is a build
    // error ------------------------------------------------------------------

    /// This file, read when it is compiled.
    ///
    /// `abi/src/semantic.rs` reads `node.rs` this way and says why: a property
    /// of a *source text* is not observable from a value, and the property this
    /// module is accepted on is one. An exhaustive `match` is the compiler's to
    /// enforce; that there is still no wildcard in it tomorrow is not, because
    /// Rust has no way to forbid one.
    const SOURCE: &str = include_str!("reader.rs");

    /// The body of [`phrasing`]'s match, as text.
    ///
    /// The first `match role {` in the file is `phrasing`'s, and everything up
    /// to the first line that closes a block at four spaces is its arms.
    fn phrasing_arms() -> &'static str {
        let after = SOURCE.split_once("match role {").expect("the projection matches on Role").1;
        after.split_once("\n    }\n}").expect("the match closes the function").0
    }

    /// **The exit's first half.** Every one of the twenty-two roles has a
    /// phrase, and a twenty-third would be a compile error.
    ///
    /// The compile error is the compiler's: [`phrasing`] is a `match` over a
    /// closed enum with no wildcard, so `error[E0004]` is what a twenty-third
    /// variant produces at `interface/src/reader.rs:196`. What this test holds
    /// is the two things the compiler cannot: that the arms are still one per
    /// role rather than a wildcard somebody added in a hurry, and that no arm
    /// has been collapsed into a catch-all.
    #[test]
    fn the_projection_decides_every_role_and_carries_no_wildcard() {
        let arms = phrasing_arms();
        assert_eq!(
            arms.matches("Role::").count(),
            Role::COUNT,
            "one arm per role, and the count is read from the vocabulary rather than written here"
        );
        assert!(!arms.contains("_ =>"), "a wildcard arm is where a role nobody decided about goes");
        assert!(arms.contains("Role::Surface =>"), "the first role of the vocabulary");
        assert!(arms.contains("Role::Marker =>"), "and the last");
    }

    /// Every role is sayable, and no two roles are said the same way.
    ///
    /// Two nouns alike would make a reading ambiguous in exactly the way a
    /// wildcard arm does — a listener told *group* for two different roles
    /// cannot tell them apart — so this is the same property the test above
    /// holds, checked on the other side of the match.
    #[test]
    fn every_role_has_a_phrase_and_no_two_share_one() {
        for role in Role::all() {
            let said = phrasing(role);
            assert!(!said.noun.is_empty(), "{} has no noun", role.name());
            let alike = Role::all().filter(|other| phrasing(*other).noun == said.noun).count();
            assert_eq!(alike, 1, "{} shares its noun with another role", role.name());
        }
    }

    /// The spoken noun is not the wire's spelling, and the three that differ are
    /// counted here so the claim above cannot drift into being false.
    ///
    /// If this ever reads zero, [`phrasing`] has become
    /// [`Role::name`](crate::node::Role::name) wearing a match, and the module's
    /// argument for owning its own words has quietly stopped applying.
    #[test]
    fn the_noun_a_listener_hears_is_not_the_name_the_wire_carries() {
        assert_eq!(phrasing(Role::Surface).noun, "window");
        assert_eq!(phrasing(Role::Command).noun, "button");
        assert_eq!(phrasing(Role::Entry).noun, "text field");
        let differ = Role::all().filter(|role| phrasing(*role).noun != role.name()).count();
        assert_eq!(
            differ, 3,
            "three of the twenty-two are said differently from how they are spelled"
        );
    }

    // ---- The whole tree ------------------------------------------------------

    /// A tree that reaches every role in the vocabulary, so that a reading of it
    /// exercises all twenty-two arms rather than the eight a timeline has.
    fn whole_vocabulary() -> [Node; 24] {
        [
            Node::new(NodeId::new(500), NodeId::UNNAMED, Role::Surface).with_content(text("Panel")),
            Node::new(NodeId::new(510), NodeId::new(500), Role::Group).with_content(text("Output")),
            Node::new(NodeId::new(511), NodeId::new(510), Role::Label).with_content(text("Volume")),
            Node::new(NodeId::new(512), NodeId::new(510), Role::Text).with_content(text("A note")),
            Node::new(NodeId::new(513), NodeId::new(510), Role::Image)
                .with_content(Content::Media(CapRef::new(0x0500))),
            Node::new(NodeId::new(514), NodeId::new(510), Role::Status).with_content(text("Ready")),
            Node::new(NodeId::new(515), NodeId::new(510), Role::Separator),
            Node::new(NodeId::new(516), NodeId::new(510), Role::Command)
                .with_content(text("Apply"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0501)),
            Node::new(NodeId::new(517), NodeId::new(510), Role::Toggle)
                .with_content(text("Mute"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0502)),
            Node::new(NodeId::new(518), NodeId::new(510), Role::Entry)
                .with_content(text("name"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0503)),
            Node::new(NodeId::new(519), NodeId::new(510), Role::Number)
                .with_content(Content::Value(crate::node::Reading::plain(Quantity::whole(
                    3,
                    Unit::Count,
                ))))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0504)),
            Node::new(NodeId::new(520), NodeId::new(500), Role::List).with_content(text("Takes")),
            Node::new(NodeId::new(521), NodeId::new(520), Role::Item).with_content(text("take 1")),
            Node::new(NodeId::new(530), NodeId::new(500), Role::Tree).with_content(text("Folders")),
            Node::new(NodeId::new(531), NodeId::new(530), Role::Item).with_content(text("Home")),
            Node::new(NodeId::new(540), NodeId::new(500), Role::Table).with_content(text("Files")),
            Node::new(NodeId::new(541), NodeId::new(540), Role::Row),
            Node::new(NodeId::new(542), NodeId::new(541), Role::Cell).with_content(text("report")),
            Node::new(NodeId::new(550), NodeId::new(500), Role::Choice)
                .with_content(text("Quality"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0505)),
            Node::new(NodeId::new(551), NodeId::new(550), Role::Item).with_content(text("high")),
            Node::new(NodeId::new(560), NodeId::new(500), Role::Canvas)
                .with_content(Content::Media(CapRef::new(0x0506))),
            Node::new(NodeId::new(561), NodeId::new(560), Role::Track).with_content(text("One")),
            Node::new(NodeId::new(562), NodeId::new(561), Role::Clip).with_content(text("a clip")),
            Node::new(NodeId::new(563), NodeId::new(560), Role::Marker).with_content(text("cue")),
        ]
    }

    /// That tree's canvas, arranged.
    fn whole_vocabulary_arrangement() -> Arrangement {
        Arrangement::declaring(NodeId::new(560), TimeBase::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(NodeId::new(562), span_x10(0, 10)))
            .expect("room")
            .with_placement(Placement::at(NodeId::new(563), seconds_x10(5)))
            .expect("room")
    }

    /// **The exit's first half, end to end.** Every role in the vocabulary is
    /// reached by one declaration and every one of them is said.
    ///
    /// The test above holds the match; this holds that the match is on the path
    /// a reading actually takes. A projection whose per-role decision were dead
    /// code would pass the first and fail this one.
    #[test]
    fn a_reading_says_every_role_the_vocabulary_has() {
        let tree = whole_vocabulary();
        let canvases = [whole_vocabulary_arrangement()];
        let reader = Reader::of(&tree, &canvases).expect("a tree and its canvas");
        let mut said = [Utterance::UNSAID; 40];
        let count = reader.read(&mut said).expect("room enough");
        assert_eq!(count, reader.lines());
        let said = &said[..count];

        for role in Role::all() {
            let heard = said
                .iter()
                .find(|line| line.role == role)
                .unwrap_or_else(|| panic!("{} is never said", role.name()));
            assert_eq!(heard.phrasing, phrasing(role), "{} is said as itself", role.name());
        }
    }

    /// A reading says every node exactly once, and [`Reader::lines`] says in
    /// advance how long it will be.
    ///
    /// The second half is what makes the first checkable without a set to
    /// remember identities in: a reading whose length matches a count derived
    /// from the roles, with every node appearing, cannot have skipped one and
    /// said another twice.
    #[test]
    fn a_reading_says_every_node_once_and_its_length_is_known_in_advance() {
        let tree = timeline();
        let canvases = [arrangement()];
        let reader = Reader::of(&tree, &canvases).expect("the timeline and its canvas");

        // Thirteen nodes; six of them are roles a reader enters — the surface,
        // the transport group, the canvas and three lanes — so nineteen lines.
        assert_eq!(reader.lines(), 19);

        let mut said = [Utterance::UNSAID; 19];
        let count = reader.read(&mut said).expect("room enough");
        assert_eq!(count, 19);

        for node in &tree {
            let opens =
                said.iter().filter(|line| line.node == node.id && line.part != Part::Leaving);
            assert_eq!(opens.count(), 1, "{} is said once", node.id.value());
        }
    }

    /// Entering and leaving bracket what is inside, and depth is the tree's.
    ///
    /// This is what [`Phrasing::entered`] buys, and the assertion is the one
    /// that fails if it stops being read: a lane whose `entered` went false
    /// leaves its clips outside any bracket, and a listener has no way to know
    /// which lane they are on.
    #[test]
    fn a_container_is_entered_and_left_and_what_is_inside_sits_between() {
        let tree = timeline();
        let canvases = [arrangement()];
        let reader = Reader::of(&tree, &canvases).expect("the timeline and its canvas");
        let mut said = [Utterance::UNSAID; 19];
        let count = reader.read(&mut said).expect("room enough");
        let said = &said[..count];

        let opens = at_of(said, FOLEY);
        let closes = said
            .iter()
            .rposition(|line| line.node == FOLEY)
            .expect("the lane is left as well as entered");
        assert_eq!(said[opens].part, Part::Entering);
        assert_eq!(said[closes].part, Part::Leaving);
        for clip in [STEPS, DOOR, RAIN] {
            let at = at_of(said, clip);
            assert!(opens < at && at < closes, "the clip is read inside its lane");
            assert_eq!(said[at].part, Part::Whole, "a clip is said in one line");
            assert_eq!(said[at].depth, said[opens].depth + 1);
        }
    }

    /// A lane is counted among its lanes and a clip is not counted at all.
    ///
    /// Both halves matter. *Track 3 of 3* is what the exit's sentence is
    /// addressed to; *clip, at 4.2 s to 6.8 s* is what a clip gets instead,
    /// because declaration order is not time order and an ordinal over clips
    /// would be a number a listener could not act on.
    #[test]
    fn a_lane_is_counted_and_a_clip_is_placed() {
        let tree = timeline();
        let canvases = [arrangement()];
        let reader = Reader::of(&tree, &canvases).expect("the timeline and its canvas");
        let mut said = [Utterance::UNSAID; 19];
        let count = reader.read(&mut said).expect("room enough");
        let said = &said[..count];

        assert_eq!(line_for(said, DIALOGUE).among, Some(Among { nth: 1, of: 3 }));
        assert_eq!(line_for(said, MUSIC).among, Some(Among { nth: 2, of: 3 }));
        assert_eq!(line_for(said, FOLEY).among, Some(Among { nth: 3, of: 3 }));
        assert_eq!(line_for(said, DOOR).among, None, "a clip's place is when it is");

        // The marker is declared after the three lanes and is not a lane, so it
        // does not move the count. A projection that counted all siblings would
        // have made `Foley` the third of four, which is the mistake this
        // fixture exists to catch.
        assert_eq!(line_for(said, PLAYHEAD).among, None);
        assert_eq!(children(&tree, SEQUENCE).count(), 4);
    }

    // ---- The exit -------------------------------------------------------------

    /// **The exit's second half.** RFC 0078's timeline, described without a
    /// pixel — including the clip between 4.2 s and 6.8 s on track 3.
    ///
    /// Nothing here constructs, resolves or asserts a pixel. The canvas declares
    /// a rendering, because it is a canvas and draws its own; the test never
    /// resolves the handle, and every fact below comes out of a reading of the
    /// declaration. And nothing here is a floating-point value: *4.2 s* is
    /// `4_200` at a scale of three, which is what `Unit::Seconds` means.
    #[test]
    fn a_reader_describes_the_clip_between_4_2_s_and_6_8_s_on_track_3_without_a_pixel() {
        let tree = timeline();
        let canvases = [arrangement()];
        let reader = Reader::of(&tree, &canvases).expect("the timeline and its canvas");
        let mut said = [Utterance::UNSAID; 19];
        let count = reader.read(&mut said).expect("room enough to say it");
        let said = &said[..count];

        // *Track 3.* The reader has no pixels, so the lane is the third one the
        // canvas declares — and the reading says which one that is, rather than
        // the test counting for it.
        let lane = said
            .iter()
            .find(|line| line.role == Role::Track && line.among == Some(Among { nth: 3, of: 3 }))
            .expect("a third lane");
        assert_eq!(lane.phrasing.noun, "track");
        assert_eq!(lane.node, FOLEY);

        // *The clip between 4.2 s and 6.8 s.* One line of the reading, inside
        // that lane's brackets, and the time is in seconds because a listener
        // does not know what a flick is.
        let opens = at_of(said, lane.node);
        let closes = said.iter().rposition(|line| line.node == lane.node).expect("left as well");
        let clip = said[opens..=closes]
            .iter()
            .find(|line| {
                line.when
                    == Spoken::Between(
                        Quantity::scaled(4_200, 3, Unit::Seconds),
                        Quantity::scaled(6_800, 3, Unit::Seconds),
                    )
            })
            .expect("a clip between 4.2 s and 6.8 s on that lane");

        // *And a screen reader describes it.* Everything a description needs,
        // from the declaration: what it is called, what it says, how it stands,
        // and what may be invoked on it.
        assert_eq!(clip.node, DOOR);
        assert_eq!(clip.role, Role::Clip);
        assert_eq!(clip.phrasing.noun, "clip");
        assert_eq!(clip.part, Part::Whole);
        assert_eq!(clip.content, text("door-slam"));
        assert!(clip.state.holds(StateSet::SELECTED));
        assert_eq!(clip.operable, Operable::Invocable(DOOR_INTENT));

        // The lane was load-bearing. Another lane carries a clip over exactly
        // that stretch, and it is a different node with a different name.
        let elsewhere = line_for(said, BED);
        assert_eq!(elsewhere.when, clip.when, "the same stretch");
        assert_ne!(elsewhere.node, clip.node, "and not the same clip");

        // *Without a pixel.* The rendering is declared and is carried across as
        // a capability: named, never resolved, and nothing above would answer
        // differently if it were absent — which is the half of section 13's
        // sentence a fixture declaring no rendering could not show.
        assert_eq!(line_for(said, SEQUENCE).content, Content::Media(RENDERING));

        // And the chrome a canvas's own linearisation cannot reach is in the
        // same reading: the transport, and the button on it.
        assert_eq!(line_for(said, TRANSPORT).phrasing.noun, "group");
        assert_eq!(line_for(said, PLAY).phrasing.noun, "button");
        assert_eq!(line_for(said, SURFACE).phrasing.noun, "window");
    }

    // ---- What a reading refuses ----------------------------------------------

    /// A canvas no arrangement is about is refused, rather than read with its
    /// clips untimed.
    ///
    /// This is the floor of RFC 0078 arriving in the projection. The tree is
    /// the same one the exit reads; the only thing missing is the dimension,
    /// and a reading that carried on would be the opaque rectangle with a
    /// caption under it.
    #[test]
    fn a_canvas_no_arrangement_is_about_is_refused() {
        let tree = timeline();
        assert_eq!(Reader::of(&tree, &[]).unwrap_err(), Unreadable::Unarranged(SEQUENCE));
        // And it is the canvas that is missing rather than the tree that is
        // wrong: with the arrangement the same tree reads.
        let canvases = [arrangement()];
        assert!(Reader::of(&tree, &canvases).is_ok());
    }

    /// Two arrangements about one canvas are refused, because which of them won
    /// would depend on the order they were passed in.
    #[test]
    fn two_arrangements_about_one_canvas_are_refused() {
        let tree = timeline();
        let canvases = [arrangement(), arrangement()];
        assert_eq!(Reader::of(&tree, &canvases).unwrap_err(), Unreadable::ArrangedTwice(SEQUENCE));
    }

    /// A canvas that did not meet RFC 0078's floor is refused here too, in
    /// `canvas.rs`'s own words.
    #[test]
    fn a_canvas_that_did_not_meet_the_floor_is_refused_in_its_own_words() {
        let tree = timeline();
        let bare = [Arrangement::declaring(SEQUENCE, TimeBase::FLICKS, Selection::NOTHING)];
        assert_eq!(
            Reader::of(&tree, &bare).unwrap_err(),
            Unreadable::Canvas(Escape::NothingPlaced(SEQUENCE))
        );
    }

    /// A tree the vocabulary refuses is refused here, in `node.rs`'s own words.
    #[test]
    fn a_tree_that_is_not_one_is_refused_in_the_vocabularys_words() {
        let orphan = [Node::new(PLAY, SURFACE, Role::Command)];
        assert_eq!(
            Reader::of(&orphan, &[]).unwrap_err(),
            Unreadable::Vocabulary(Defect::UnknownParent { node: PLAY, parent: SURFACE })
        );
    }

    /// A reading with less room than it needs is refused rather than truncated.
    #[test]
    fn a_reading_with_no_room_is_refused_rather_than_stopped_halfway() {
        let tree = timeline();
        let canvases = [arrangement()];
        let reader = Reader::of(&tree, &canvases).expect("the timeline and its canvas");
        let mut cramped = [Utterance::UNSAID; 4];
        assert_eq!(reader.read(&mut cramped), Err(Unreadable::NoRoom));
        assert!(reader.lines() > cramped.len(), "and the caller can tell how much room is enough");
    }

    /// A time the base can hold and a description cannot express names the node
    /// it is about, rather than being silently dropped.
    ///
    /// Unreachable at [`TimeBase::FLICKS`]; reachable at a coarse base the
    /// application declares, which is the point — clause 2 hands the base to the
    /// application, so the bound is theirs to exceed.
    #[test]
    fn a_time_no_description_can_hold_is_named() {
        let tree = [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface),
            Node::new(SEQUENCE, SURFACE, Role::Canvas).with_content(Content::Media(RENDERING)),
            Node::new(PLAYHEAD, SEQUENCE, Role::Marker).with_content(text("Playhead")),
        ];
        let base = TimeBase::new(1).expect("a base, and the coarsest one there is");
        let canvases = [Arrangement::declaring(SEQUENCE, base, Selection::NOTHING)
            .with_placement(Placement::at(PLAYHEAD, Ticks::new(i64::MAX)))
            .expect("one placement")];
        let reader = Reader::of(&tree, &canvases).expect("the floor is met: something is placed");
        let mut said = [Utterance::UNSAID; 8];
        assert_eq!(reader.read(&mut said), Err(Unreadable::Canvas(Escape::Unsayable(PLAYHEAD))));
    }

    /// A tree with no canvas at all reads, and reads with no arrangements.
    ///
    /// Worth its own test because the refusal above is a loop over canvases, and
    /// a loop that refused when there were none would make this module unusable
    /// for every application that is not a timeline — which is most of them.
    #[test]
    fn a_tree_with_no_canvas_reads_with_no_arrangements() {
        let tree = [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface).with_content(text("Panel")),
            Node::new(TRANSPORT, SURFACE, Role::Group).with_content(text("Transport")),
            Node::new(PLAY, TRANSPORT, Role::Command)
                .with_content(text("Play"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0401)),
        ];
        let reader = Reader::of(&tree, &[]).expect("a tree that declares no canvas");
        let mut said = [Utterance::UNSAID; 8];
        let count = reader.read(&mut said).expect("room enough");
        assert_eq!(count, reader.lines());
        assert_eq!(said[..count].iter().filter(|line| line.when != Spoken::Untimed).count(), 0);
        assert!(has_children(&tree, SURFACE));
        assert!(find(reader.nodes(), PLAY).is_some());
    }
}
