// SPDX-License-Identifier: Apache-2.0 OR MIT
//! The agent projection: the tree read to be acted on, and one selection per
//! act.
//!
//! [`reader`](crate::reader) is the same tree said. This is the same tree
//! *used*: an agent locates a node by what the application declared about it —
//! a lane, a stretch of the dimension the application owns — and comes away
//! with a [`Selected`], which is the one value an invocation accepts. The two
//! projections take the same two arguments and reach the same nodes, and
//! `the_two_projections_agree_about_one_clip` holds them to it. That is the
//! point of `E3-B06j`'s exit naming the same clip `E3-B06i` describes rather
//! than a fresh fixture: an agent and a screen reader over one tree is section
//! 11's thesis doing work, and two projections that each read their own tree
//! would be the thing this system is an argument against.
//!
//! # Why an agent does not read the reading
//!
//! The cheap agent takes the screen reader's output and acts on it, which is
//! what accessibility-driven automation actually does, and it is the failure
//! this module exists not to repeat. A reading is prose: it has chosen a
//! language, dropped what a listener does not need, and flattened a tree into
//! lines. An agent built on one is parsing English to recover structure the
//! tree had all along, and it breaks when the wording changes — which RFC 0104
//! explicitly permits a projection to do.
//!
//! So this module takes `&[Node]` and `&[Arrangement]`, the same two arguments
//! [`Reader::of`] takes, and reads them itself.
//!
//! # Why it holds a [`Reader`] anyway
//!
//! Because *is this a tree at all* is one question with one answer.
//! [`Reader::of`] decides it — the vocabulary's [`check`](crate::node::check),
//! RFC 0078's floor at every canvas, one arrangement per canvas and no canvas
//! without one — and none of those are facts about saying words. They are facts
//! about the tree. A second admission here would be a second opinion, and the
//! day the two disagreed a tree would be readable and not actable, or the other
//! way round, with nobody able to say which was right.
//!
//! So [`Acting::of`] runs [`Reader::of`] and keeps what it returns, and
//! [`Unactable::Tree`] carries the refusal whole rather than paraphrasing it.
//! *What would reverse this:* an agent that must act on a tree a reader refuses
//! — a partial tree, a tree mid-edit — at which point the admission is owed its
//! own name and both projections take it as an argument.
//!
//! # What an agent may do with each role, and why a `match` and not a table
//!
//! [`affordance`] answers it for all twenty-two roles with no wildcard arm and
//! no table, for the reasons `reader.rs` sets out at length and which are not
//! repeated here: a written-out table is the wrong length on a twenty-third
//! role and says nothing about a reorder; a derived one grows silently and
//! announces the new role as its neighbour; a `match` with no wildcard is
//! `error[E0004]` naming the variant nobody decided about, in the file that has
//! to decide.
//!
//! What is worth saying here is what makes this a *second* decision rather than
//! the first one restated. [`Invocation::Never`] falls on exactly the ten roles
//! [`Role::is_operable`] refuses an intent, and
//! `an_agent_never_invokes_what_the_vocabulary_refuses_an_intent` asserts the
//! agreement rather than deriving it, because the two disagreeing in either
//! direction would be a defect: an agent invoking what cannot carry an intent
//! is invoking nothing, and one declining what can is an agent that cannot
//! press a button. The projection's own decision is the three-way split across
//! the other twelve — whether doing it twice does it twice, undoes it, or
//! changes nothing the second time — and the vocabulary holds no such field.
//! That is the fact an agent needs and a listener does not: a screen reader
//! that says *button* twice has said a word twice, and an agent that invokes a
//! clip twice has cut it twice.
//!
//! *What would reverse this:* the vocabulary growing an idempotence column. It
//! would belong there rather than here the day a second agent exists, and this
//! function would become the reader's [`phrasing`](crate::reader::phrasing) —
//! one projection's opinion about a fact the tree already holds.
//!
//! **Measured rather than asserted**, on `reader.rs`'s terms exactly, because
//! this module is accepted on the same mechanism and a second claim about it
//! that nobody ran would be worth less than no claim. A twenty-third role —
//! `Gauge`, added to the `vocabulary!` invocation in `interface/src/node.rs`
//! and taken out again — stops this crate's build at four sites, of which this
//! file is one: `error[E0004]: non-exhaustive patterns: 'Role::Gauge' not
//! covered` at `interface/src/agent.rs:195`, the match in [`affordance`]. It
//! was three sites before this module existed, and `reader.rs` carries the
//! number because that is the file whose whole argument is about it.
//!
//! One site in this file and not two: an agent's second per-role question —
//! whether to descend — is a field of the one [`Affordance`] this returns
//! rather than a second function, so there is one place to forget rather than
//! two, and forgetting it is a compile error either way.
//!
//! # One selection, one act, and no way to invoke a node nobody selected
//!
//! [`Selected`] has no public constructor. The only routes to one are
//! [`Acting::between`] and [`Acting::select`], both of which run against a tree
//! this module was admitted against, so a value of that type is evidence that
//! the node exists, that its role was decided about, and that what the
//! application says may be done with it was read from the application's own
//! declaration rather than assumed.
//!
//! This is `Participating::operable`'s rule one scope out, and it is the same
//! rule: the answer *what may be invoked on this* is only ever given about a
//! node something was admitted against. `f_semantic::agent` is what turns a
//! [`Selected`] into a capability call, and it cannot make one.

use crate::canvas::{Arrangement, Escape, Extent, Operable, Sole};
use crate::node::{Node, NodeId, Role, children, find};
use crate::reader::{Reader, Unreadable};

/// What invoking a node of one role does, and therefore what invoking it twice
/// does.
///
/// **The question a screen reader never asks.** An agent retries, and an agent
/// that does not know whether a retry repeats the act, undoes it, or costs
/// nothing either gives up after one refusal or cuts the same clip four times.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Invocation {
    /// An agent does not invoke a node of this role at all.
    ///
    /// Not *this node happens to carry no intent*, which is
    /// [`Operable::Inert`] and is a fact about one node. This is a fact about
    /// the role: the vocabulary refuses an intent here, so an agent sending one
    /// would be inventing an act the application never offered.
    Never,
    /// Invoking it does one thing, and again does it again.
    ///
    /// The dangerous one, and it is first among the three for that reason: an
    /// agent that retries one of these after a refusal it misread has done the
    /// act twice.
    Repeats,
    /// Invoking it moves between two standings, and again is back where it
    /// started.
    ///
    /// A retry is therefore neither safe nor destructive — it is wrong in a way
    /// the agent can see, because the node's own
    /// [`StateSet`](crate::node::StateSet) says which standing it is in.
    Reverses,
    /// Invoking it settles on a value, and again with the same value changes
    /// nothing.
    ///
    /// The only one a retry is free on. An agent may drive one of these to a
    /// known standing without reading first, which is what makes *set the
    /// quality to high* a thing an agent can do idempotently and *press apply*
    /// not.
    Settles,
}

/// What an agent may do with a node of one role.
///
/// Two decisions and not three, which is the difference from
/// [`Phrasing`](crate::reader::Phrasing) rather than an economy: a reader has
/// to decide what to call a thing, and an agent does not care what anything is
/// called.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Affordance {
    /// What invoking it does.
    pub invocation: Invocation,
    /// Whether an agent looking for something to act on goes under it.
    ///
    /// **A decision the vocabulary deliberately does not make.**
    /// [`Role::accepts_parent`] lets a [`Role::Group`] sit under almost
    /// anything, so a [`Role::Label`] with a button under it is a legal tree;
    /// this says an agent does not go looking there. The cost is named rather
    /// than hidden: an application that hangs its controls off a label has them
    /// invisible to every agent. That is the right cost, because a projection
    /// descending everywhere would find controls in places no reader announces,
    /// which is a tree that reads one way and acts another.
    pub under: bool,
}

/// What an agent may do with a node of `role`.
///
/// # The twenty-two decisions
///
/// The ten [`Invocation::Never`]s are the ten roles [`Role::is_operable`]
/// refuses an intent, and the module comment says why they must agree and why
/// the agreement is asserted rather than derived. The twelve that remain are
/// this function's own, and they do not follow the families:
///
/// [`Role::Command`], [`Role::Row`], [`Role::Cell`], [`Role::Canvas`] and
/// [`Role::Clip`] repeat — pressing, opening and cutting are acts that happen
/// as many times as they are asked for. [`Role::Toggle`] and [`Role::Track`]
/// reverse: a toggle is two standings by definition, and muting a lane twice is
/// a lane that is not muted, which is the one place a lane and a canvas differ
/// and the reason they are not one decision. [`Role::Item`], [`Role::Choice`],
/// [`Role::Entry`], [`Role::Number`] and [`Role::Marker`] settle — choosing the
/// item already chosen, typing the text already there and dragging the playhead
/// to where it already is are all nothing happening, and an agent may do them
/// without reading first.
///
/// [`Affordance::under`] is true for the nine structure roles an agent looks
/// through and for [`Role::Choice`], which holds its items. It is false for
/// everything a projection would only ever read — the four content roles and
/// the separator — and for [`Role::Clip`] and [`Role::Marker`], which are where
/// the looking stops.
#[must_use]
pub const fn affordance(role: Role) -> Affordance {
    match role {
        Role::Surface => Affordance { invocation: Invocation::Never, under: true },
        Role::Group => Affordance { invocation: Invocation::Never, under: true },
        Role::List => Affordance { invocation: Invocation::Never, under: true },
        Role::Tree => Affordance { invocation: Invocation::Never, under: true },
        Role::Item => Affordance { invocation: Invocation::Settles, under: true },
        Role::Table => Affordance { invocation: Invocation::Never, under: true },
        Role::Row => Affordance { invocation: Invocation::Repeats, under: true },
        Role::Cell => Affordance { invocation: Invocation::Repeats, under: true },
        Role::Separator => Affordance { invocation: Invocation::Never, under: false },
        Role::Label => Affordance { invocation: Invocation::Never, under: false },
        Role::Text => Affordance { invocation: Invocation::Never, under: false },
        Role::Image => Affordance { invocation: Invocation::Never, under: false },
        Role::Status => Affordance { invocation: Invocation::Never, under: false },
        Role::Command => Affordance { invocation: Invocation::Repeats, under: false },
        Role::Toggle => Affordance { invocation: Invocation::Reverses, under: false },
        Role::Entry => Affordance { invocation: Invocation::Settles, under: false },
        Role::Choice => Affordance { invocation: Invocation::Settles, under: true },
        Role::Number => Affordance { invocation: Invocation::Settles, under: false },
        Role::Canvas => Affordance { invocation: Invocation::Repeats, under: true },
        Role::Track => Affordance { invocation: Invocation::Reverses, under: true },
        Role::Clip => Affordance { invocation: Invocation::Repeats, under: false },
        Role::Marker => Affordance { invocation: Invocation::Settles, under: false },
    }
}

/// Why an agent cannot act on this tree, or cannot find what it asked for.
///
/// The refusals split into two kinds and the split is worth reading. The first
/// variant is about the *tree*, is decided by [`Reader::of`], and is carried
/// whole for the module comment's reason. Everything below it is about one
/// question an agent asked — a lane that is not there, a stretch nothing spans,
/// a stretch two things span — and none of those is a defect in anything. An
/// agent told *nothing is there* has been answered.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unactable {
    /// The tree is not one a projection may be admitted against.
    Tree(Unreadable),
    /// A node this tree does not declare.
    NoSuchNode(NodeId),
    /// A node that is not a [`Role::Canvas`] of this tree, asked for a lane.
    NoSuchCanvas(NodeId),
    /// A canvas has fewer lanes than the one asked for. Both numbers are
    /// carried, because *track 4 of 3* is something an agent can correct and
    /// *there is no track 4* is not.
    NoSuchLane {
        /// The canvas asked.
        canvas: NodeId,
        /// Which lane was asked for, counting from one.
        /// Unit: none — an ordinal among the lanes of one canvas.
        nth: u16,
        /// How many it has.
        /// Unit: none — a count of lanes.
        of: u16,
    },
    /// A canvas of this tree that no arrangement is about. Reachable only for a
    /// lane of a canvas [`Reader::of`] admitted, so it is here for completeness
    /// rather than as a case a caller meets.
    Unarranged(NodeId),
    /// Nothing on that lane spans the whole of that stretch.
    Nothing(NodeId),
    /// Two things do, and this module will not guess which was meant —
    /// [`Sole`]'s argument, passed through rather than collapsed.
    Several(NodeId),
    /// A question the canvas refused. Carried whole, for [`Unactable::Tree`]'s
    /// reason.
    Canvas(Escape),
}

/// A tree an agent may act on: admitted once, exactly as a reading is.
///
/// The only route to a [`Selected`], and therefore the only route to an
/// invocation.
#[derive(Clone, Copy, Debug)]
pub struct Acting<'a> {
    /// What [`Reader::of`] admitted. Held rather than re-derived; the module
    /// comment says why an agent keeps a reader's admission and not its words.
    admitted: Reader<'a>,
}

impl<'a> Acting<'a> {
    /// Meet a tree and the arrangements over its canvases, and be admitted or
    /// refused.
    ///
    /// # Errors
    ///
    /// [`Unactable::Tree`], carrying whatever [`Reader::of`] refused.
    pub fn of(nodes: &'a [Node], canvases: &'a [Arrangement]) -> Result<Self, Unactable> {
        match Reader::of(nodes, canvases) {
            Ok(admitted) => Ok(Self { admitted }),
            Err(refused) => Err(Unactable::Tree(refused)),
        }
    }

    /// The tree being acted on.
    #[must_use]
    pub const fn nodes(&self) -> &'a [Node] {
        self.admitted.nodes()
    }

    /// The same tree, as something to say.
    ///
    /// Published so that a consumer holding one projection can reach the other
    /// without a second admission — which is the whole of what *the two
    /// projections read one tree* buys, and it would be odd to argue for it and
    /// then make it impossible.
    #[must_use]
    pub const fn reading(&self) -> Reader<'a> {
        self.admitted
    }

    /// The `nth` lane of `canvas`, counting from one.
    ///
    /// **This is *track 3*.** Lanes in declaration order, filtered to
    /// [`Role::Track`], which is [`Among`](crate::reader::Among)'s rule — the
    /// same ordinal a listener is given, so an agent acting on what somebody
    /// heard acts on the lane they heard about. Counting all children instead
    /// would let a marker declared between two lanes shift every lane after it,
    /// and `reader.rs` says why that names the wrong one.
    ///
    /// # Errors
    ///
    /// [`Unactable::NoSuchNode`] for an identity this tree does not declare,
    /// [`Unactable::NoSuchCanvas`] for a node that is not a canvas, and
    /// [`Unactable::NoSuchLane`] carrying how many lanes there are.
    pub fn lane(&self, canvas: NodeId, nth: u16) -> Result<NodeId, Unactable> {
        let declared = find(self.nodes(), canvas).ok_or(Unactable::NoSuchNode(canvas))?;
        if declared.role != Role::Canvas {
            return Err(Unactable::NoSuchCanvas(canvas));
        }
        let mut of = 0;
        let mut found = None;
        for lane in children(self.nodes(), canvas).filter(|node| node.role == Role::Track) {
            of += 1;
            if of == nth {
                found = Some(lane.id);
            }
        }
        found.ok_or(Unactable::NoSuchLane { canvas, nth, of })
    }

    /// The one occupant of `lane` that spans the whole of `span`, selected.
    ///
    /// **Section 13's question, asked by something that will then act on the
    /// answer.** The stretch and the lane are the application's own coordinates
    /// — its time base, its declaration order — so nothing here is a pixel, an
    /// index into an array, or a position on a screen.
    ///
    /// # Errors
    ///
    /// [`Unactable::Nothing`] and [`Unactable::Several`] for [`Sole`]'s two
    /// non-answers, [`Unactable::Canvas`] for a node that is not a lane of a
    /// canvas of this tree, and [`Unactable::Unarranged`] for a lane whose
    /// canvas has no arrangement — which admission has already ruled out.
    pub fn between(&self, lane: NodeId, span: Extent) -> Result<Selected, Unactable> {
        let declared = find(self.nodes(), lane).ok_or(Unactable::NoSuchNode(lane))?;
        if declared.role != Role::Track {
            return Err(Unactable::Canvas(Escape::NotALane(lane)));
        }
        let canvas = declared.parent;
        let arrangement = self
            .admitted
            .canvases()
            .iter()
            .find(|arrangement| arrangement.canvas() == canvas)
            .ok_or(Unactable::Unarranged(canvas))?;
        let participating = arrangement.admit(self.nodes()).map_err(Unactable::Canvas)?;
        match participating.occupant_between(lane, span).map_err(Unactable::Canvas)? {
            Sole::One(occupant) => self.select(occupant),
            Sole::Nothing => Err(Unactable::Nothing(lane)),
            Sole::Several => Err(Unactable::Several(lane)),
        }
    }

    /// The node with this identity, selected.
    ///
    /// The identity route, for an agent that already has one — from a reading
    /// it was given, from a selection it made a minute ago, from a script. It
    /// is not a weaker door than [`between`](Self::between): both end here, and
    /// what makes a [`Selected`] worth holding is that this tree declares the
    /// node and that what may be done with it was read from the declaration.
    ///
    /// # Errors
    ///
    /// [`Unactable::NoSuchNode`] for an identity this tree does not declare.
    pub fn select(&self, node: NodeId) -> Result<Selected, Unactable> {
        let found = find(self.nodes(), node).ok_or(Unactable::NoSuchNode(node))?;
        Ok(Selected {
            node: found.id,
            role: found.role,
            affordance: affordance(found.role),
            operable: Operable::of(found),
        })
    }
}

/// A node an agent has found, and what may be done with it.
///
/// **No public constructor, and that is the whole of its worth.** It is minted
/// only by [`Acting`], against a tree [`Reader::of`] admitted, so a holder of
/// one has not guessed an identity, invented an affordance, or decided for
/// itself that an application is offering something. `f_semantic::agent` takes
/// a reference to one and will not invoke anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selected {
    node: NodeId,
    role: Role,
    affordance: Affordance,
    operable: Operable,
}

impl Selected {
    /// Which node.
    #[must_use]
    pub const fn node(self) -> NodeId {
        self.node
    }

    /// What it is.
    #[must_use]
    pub const fn role(self) -> Role {
        self.role
    }

    /// What an agent may do with a node of that role.
    #[must_use]
    pub const fn affordance(self) -> Affordance {
        self.affordance
    }

    /// What this application says may be done with *this* node, now.
    ///
    /// The other half, and the two are asked separately for the reason
    /// `canvas.rs` gives about [`Operable`] itself: a role that can be invoked
    /// and a node the application is currently declining to run are different
    /// facts, and an agent that conflated them would retry forever or give up
    /// at once.
    #[must_use]
    pub const fn operable(self) -> Operable {
        self.operable
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::canvas::{Placement, Selection, Spoken, Ticks, TimeBase};
    use crate::node::{CapRef, Content, StateSet, Text};
    use crate::reader::{Among, Utterance};

    // RFC 0078's timeline, at the identities `reader.rs` gives it. **The same
    // fixture, written a second time on purpose**: `E3-B06j`'s exit names the
    // clip `E3-B06i` describes, so the two projections have to be shown over
    // one tree, and a test module cannot import another test module's
    // fixtures. What must not drift is the declaration — the identities, the
    // lanes, the times — and `the_two_projections_agree_about_one_clip` is what
    // would notice if it did, because it asks the reader about the same tree it
    // hands the agent.
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

    /// The canvas's own rendering, never resolved anywhere in this module.
    const RENDERING: CapRef = CapRef::new(0x0400);
    /// What operating the door slam does.
    const DOOR_INTENT: CapRef = CapRef::new(0x0409);
    /// What operating the rain would do, if the application were letting it.
    const RAIN_INTENT: CapRef = CapRef::new(0x040a);
    /// What operating the music bed does.
    const BED_INTENT: CapRef = CapRef::new(0x040b);

    fn text(value: &str) -> Content {
        Content::Text(Text::new(value).expect("fits within TEXT_MAX"))
    }

    /// A time in tenths of a second, as ticks of the canvas's base. The scale is
    /// in the name for RFC 0004's reason: `42` is 4.2 s, and there is no
    /// floating-point value in this tree to write it any other way.
    fn seconds_x10(value: i64) -> Ticks {
        TimeBase::FLICKS.ticks_from_seconds(value, 1).expect("a time this base can name")
    }

    /// A stretch, in tenths of a second.
    fn span_x10(from: i64, to: i64) -> Extent {
        Extent::new(seconds_x10(from), seconds_x10(to)).expect("a span that advances")
    }

    fn timeline() -> [Node; 13] {
        [
            Node::new(SURFACE, NodeId::UNNAMED, Role::Surface).with_content(text("Sequence")),
            Node::new(TRANSPORT, SURFACE, Role::Group).with_content(text("Transport")),
            Node::new(PLAY, TRANSPORT, Role::Command)
                .with_content(text("Play"))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0401)),
            Node::new(SEQUENCE, SURFACE, Role::Canvas)
                .with_content(Content::Media(RENDERING))
                .with_state(StateSet::ENABLED)
                .with_intent(CapRef::new(0x0402)),
            Node::new(DIALOGUE, SEQUENCE, Role::Track).with_content(text("Dialogue")),
            Node::new(AMBIENCE, DIALOGUE, Role::Clip).with_content(text("ambience")),
            Node::new(MUSIC, SEQUENCE, Role::Track).with_content(text("Music")),
            Node::new(BED, MUSIC, Role::Clip)
                .with_content(text("bed"))
                .with_state(StateSet::ENABLED)
                .with_intent(BED_INTENT),
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

    // ---- The vocabulary is decided here, and a twenty-third role is a build
    // error -------------------------------------------------------------------

    /// This file, read when it is compiled.
    ///
    /// `reader.rs` reads itself this way and says why: a property of a *source
    /// text* is not observable from a value, and an exhaustive `match` staying
    /// free of a wildcard tomorrow is one — Rust offers no way to forbid one.
    const SOURCE: &str = include_str!("agent.rs");

    /// The body of [`affordance`]'s match, as text.
    fn affordance_arms() -> &'static str {
        let after = SOURCE.split_once("match role {").expect("the projection matches on Role").1;
        after.split_once("\n    }\n}").expect("the match closes the function").0
    }

    /// **The exit's third clause.** Every one of the twenty-two roles has an
    /// affordance, and a twenty-third would be a compile error.
    ///
    /// The compile error is the compiler's — `error[E0004]`, in this file,
    /// naming the variant nobody decided about. What this holds is the two
    /// things the compiler cannot: that the arms are still one per role, and
    /// that none has been collapsed into a catch-all.
    #[test]
    fn what_an_agent_may_do_is_decided_per_role_and_carries_no_wildcard() {
        let arms = affordance_arms();
        assert_eq!(
            arms.matches("Role::").count(),
            Role::COUNT,
            "one arm per role, and the count is read from the vocabulary rather than written here"
        );
        assert!(!arms.contains("_ =>"), "a wildcard arm is where a role nobody decided about goes");
        assert!(arms.contains("Role::Surface =>"), "the first role of the vocabulary");
        assert!(arms.contains("Role::Marker =>"), "and the last");
    }

    /// The one column this projection is not free to invent.
    ///
    /// An agent that invoked a role the vocabulary refuses an intent would be
    /// invoking nothing; one that declined a role the vocabulary permits one
    /// could not press a button. So the two must agree exactly, and this is
    /// where the agreement is checked rather than assumed — the module comment
    /// says why it is asserted rather than derived.
    #[test]
    fn an_agent_never_invokes_what_the_vocabulary_refuses_an_intent() {
        for role in Role::all() {
            assert_eq!(
                affordance(role).invocation == Invocation::Never,
                !role.is_operable(),
                "{} is decided one way by the vocabulary and the other by the projection",
                role.name(),
            );
        }
        // And the split across the twelve that remain is this module's own, so
        // it has to be a split: all three of the other kinds are reached, or the
        // enum has become a boolean wearing four names.
        for kind in [Invocation::Repeats, Invocation::Reverses, Invocation::Settles] {
            assert!(
                Role::all().any(|role| affordance(role).invocation == kind),
                "{kind:?} is a decision no role makes",
            );
        }
    }

    /// Descending is a decision and not a family: the vocabulary would let a
    /// control hang under a label, and an agent does not go looking there.
    #[test]
    fn an_agent_looks_under_structure_and_not_under_content() {
        assert!(affordance(Role::Group).under);
        assert!(affordance(Role::Choice).under, "a choice holds the items it is choosing between");
        assert!(!affordance(Role::Label).under);
        assert!(!affordance(Role::Clip).under, "a clip is where the looking stops");
    }

    // ---- One tree, two projections ------------------------------------------

    /// **The exit's first clause, the selecting half.** The agent finds the clip
    /// between 4.2 s and 6.8 s on track 3, by the application's own coordinates,
    /// and the reading of the same tree calls the same lane *track 3 of 3* and
    /// says the same thing about the same clip.
    ///
    /// The fixture is built so the answer cannot arrive by having nowhere else
    /// to go: `Music` carries a clip over exactly that stretch, so a projection
    /// that ignored the lane would answer `bed`.
    #[test]
    fn the_two_projections_agree_about_one_clip() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");

        let lane = acting.lane(SEQUENCE, 3).expect("the canvas has three lanes");
        assert_eq!(lane, FOLEY);
        let selected = acting.between(lane, span_x10(42, 68)).expect("one clip spans it");
        assert_eq!(selected.node(), DOOR);
        assert_eq!(selected.role(), Role::Clip);
        assert_eq!(selected.operable(), Operable::Invocable(DOOR_INTENT));
        assert_eq!(selected.affordance().invocation, Invocation::Repeats);

        // The decoy: the same stretch, one lane over, a different clip.
        let other = acting.lane(SEQUENCE, 2).expect("and a second");
        let bed = acting.between(other, span_x10(42, 68)).expect("one clip spans it there too");
        assert_eq!(bed.node(), BED, "the stretch alone does not identify a clip");

        // And the reader, over the very same two slices.
        let reading = acting.reading();
        let mut said = [Utterance::UNSAID; 32];
        let lines = reading.read(&mut said).expect("the tree is sayable");
        let said = &said[..lines];
        let line =
            |node| *said.iter().find(|line| line.node == node).expect("the reading names it");
        assert_eq!(
            line(FOLEY).among,
            Some(Among { nth: 3, of: 3 }),
            "the agent's third lane is the lane a listener is told is the third",
        );
        assert_eq!(line(DOOR).operable, selected.operable(), "one tree, one answer");
        assert!(matches!(line(DOOR).when, Spoken::Between(_, _)));
        assert_eq!(
            line(DOOR).when,
            line(BED).when,
            "the two clips are said to be at the same time, which is why the lane had to decide",
        );
    }

    /// A lane is counted among lanes, not among children.
    ///
    /// The timeline above declares its marker last, so it cannot tell the two
    /// rules apart. This one declares a cue between the first lane and the
    /// second: counting children would answer the second lane for *track 3*.
    #[test]
    fn a_lane_is_counted_among_lanes_and_not_among_children() {
        const SURFACE_B: NodeId = NodeId::new(599);
        const CANVAS: NodeId = NodeId::new(600);
        const LANE_A: NodeId = NodeId::new(601);
        const CLIP_A: NodeId = NodeId::new(602);
        const CUE: NodeId = NodeId::new(603);
        const LANE_B: NodeId = NodeId::new(604);
        const CLIP_B: NodeId = NodeId::new(605);
        const LANE_C: NodeId = NodeId::new(606);
        const CLIP_C: NodeId = NodeId::new(607);

        let nodes = [
            Node::new(SURFACE_B, NodeId::UNNAMED, Role::Surface).with_content(text("S")),
            Node::new(CANVAS, SURFACE_B, Role::Canvas)
                .with_content(Content::Media(CapRef::new(0x0600))),
            Node::new(LANE_A, CANVAS, Role::Track).with_content(text("A")),
            Node::new(CLIP_A, LANE_A, Role::Clip).with_content(text("a")),
            Node::new(CUE, CANVAS, Role::Marker).with_content(text("cue")),
            Node::new(LANE_B, CANVAS, Role::Track).with_content(text("B")),
            Node::new(CLIP_B, LANE_B, Role::Clip).with_content(text("b")),
            Node::new(LANE_C, CANVAS, Role::Track).with_content(text("C")),
            Node::new(CLIP_C, LANE_C, Role::Clip).with_content(text("c")),
        ];
        let canvases = [Arrangement::declaring(CANVAS, TimeBase::FLICKS, Selection::NOTHING)
            .with_placement(Placement::over(CLIP_A, span_x10(0, 10)))
            .expect("room")
            .with_placement(Placement::at(CUE, seconds_x10(5)))
            .expect("room")
            .with_placement(Placement::over(CLIP_B, span_x10(0, 10)))
            .expect("room")
            .with_placement(Placement::over(CLIP_C, span_x10(0, 10)))
            .expect("room")];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");

        assert_eq!(acting.lane(CANVAS, 3), Ok(LANE_C));
        assert_eq!(
            acting.lane(CANVAS, 2),
            Ok(LANE_B),
            "a marker between two lanes did not make the second lane the third child",
        );
        assert_eq!(
            acting.lane(CANVAS, 4),
            Err(Unactable::NoSuchLane { canvas: CANVAS, nth: 4, of: 3 }),
        );
    }

    /// The refusals a question about a lane earns, and none of them is a defect
    /// in the tree.
    #[test]
    fn a_question_with_no_single_answer_is_refused_rather_than_guessed_at() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");

        // Nothing spans the whole of it: the foley lane is empty from 10.0 s.
        assert_eq!(acting.between(FOLEY, span_x10(100, 110)), Err(Unactable::Nothing(FOLEY)));
        // And nothing covers the whole of 0.0 s to 10.0 s either, although three
        // clips between them do — which is `Sole`'s question and not `overlaps`.
        assert_eq!(acting.between(FOLEY, span_x10(0, 100)), Err(Unactable::Nothing(FOLEY)));
        // A lane that is not one.
        assert_eq!(
            acting.between(DOOR, span_x10(42, 68)),
            Err(Unactable::Canvas(Escape::NotALane(DOOR))),
        );
        // A node the tree does not declare, and a node that is not a canvas.
        let stranger = NodeId::new(9_999);
        assert_eq!(acting.select(stranger), Err(Unactable::NoSuchNode(stranger)));
        assert_eq!(acting.lane(SURFACE, 1), Err(Unactable::NoSuchCanvas(SURFACE)));
    }

    /// A tree a reader refuses is a tree an agent refuses, in the reader's own
    /// words.
    #[test]
    fn an_agent_holds_no_second_opinion_about_whether_a_tree_is_one() {
        let nodes = timeline();
        // The canvas, with no arrangement about it: `reader.rs` refuses the
        // whole reading rather than saying the clips are untimed, and an agent
        // inherits that rather than deciding for itself.
        let refused = Acting::of(&nodes, &[]).expect_err("a canvas nothing arranges");
        assert_eq!(refused, Unactable::Tree(Unreadable::Unarranged(SEQUENCE)));
        assert!(Reader::of(&nodes, &[]).is_err(), "and the reader refuses the identical pair");
    }

    /// What the application is currently declaring, read back at the agent
    /// unchanged.
    #[test]
    fn the_selection_carries_what_the_application_says_about_this_node_now() {
        let nodes = timeline();
        let canvases = [arrangement()];
        let acting = Acting::of(&nodes, &canvases).expect("the tree is one");

        assert_eq!(
            acting.select(RAIN).expect("declared").operable(),
            Operable::Disabled(RAIN_INTENT),
            "not now, which is not not ever",
        );
        assert_eq!(
            acting.select(STEPS).expect("declared").operable(),
            Operable::Inert,
            "a clip on a lane that is offering nothing",
        );
        let lane = acting.select(DIALOGUE).expect("declared");
        assert_eq!(lane.affordance().invocation, Invocation::Reverses, "a lane mutes and unmutes");
        let play = acting.select(PLAY).expect("declared");
        assert_eq!(play.affordance().invocation, Invocation::Repeats);
        assert_eq!(play.operable(), Operable::Invocable(CapRef::new(0x0401)));
    }
}
