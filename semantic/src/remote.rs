// SPDX-License-Identifier: Apache-2.0 OR MIT
//! Remote: the declaration crosses the link, and the far side presents it at its
//! own density and its own refresh rate.
//!
//! # The claim, and what would falsify it
//!
//! `docs/design/ring-scene-boot.html` part III lists a remote client among the
//! projections and says what it is in one line: *the tree itself, projected on
//! the far side at **its** density and refresh rate. Not an encoded pixel
//! stream.* This module is that sentence as code, and it is falsifiable in three
//! directions, which is why the exit names three things:
//!
//! - **One unmodified application.** [`f_interface::example::settings_panel`] is
//!   the declaration, it is the same value the display path is handed, and
//!   nothing here asks it for a field. A remote that needed the application to
//!   help would have moved the work rather than removed it.
//! - **Two densities and two refresh rates.** A density is not a scale applied
//!   at the end: it is what the far side's *layout* is solved against, because
//!   the em a theme resolves is the text size times the density, and every
//!   floor in `f_interface::token` is in ems. A refresh rate reaches the
//!   [`Pacer`] and nothing else — it decides how often what is held is shown,
//!   and it changes no geometry at all.
//! - **The bytes are counted against what pixels would have cost.** [`Crossed`]
//!   is a tally of the entries this module actually encoded, and
//!   [`Panel::frame_bytes`] is the honest arithmetic on the other side. The
//!   ratio is published whether or not it flatters; *what it assumes* is below,
//!   because a ratio without its assumptions is a slogan.
//!
//! # What crosses, and what does not
//!
//! The link is `f_abi::semantic`: a handshake, a `DeclareNode` per node, a
//! `SetState` per node that stands somehow, and a `Commit`. That format carries
//! **identity, place, order, role, intent and state**, and it carries nothing
//! else. In particular it does not carry:
//!
//! - **Constraints.** No opcode holds a `Flow` or a minimum, and `abi`'s own
//!   `Closed` says so: *a node's flow belongs to a layout entry this format does
//!   not yet have*. So the far side's arrangement is the far side's, derived
//!   from the role by [`presentable`] and from the theme by
//!   `f_interface::token::Resolved::span`. This is the largest thing this module
//!   decides and RFC 0121 is where it is argued.
//! - **Style.** No opcode carries a `TokenSet`, so the entry that declares
//!   itself `field.danger` is painted in the far side's ink for an entry. The
//!   information is not lost — `StateSet::INVALID` does cross — but nothing here
//!   reads a state into a paint yet, and saying so is better than implying the
//!   colour survived.
//! - **Content.** [`crate::tree::Tree`] refuses `SetContent` with
//!   `Rejected::NotStored`, so no text crosses and the far side presents
//!   structure. That is `E3-B06f`'s position unchanged: a content body reaches a
//!   scene as geometry, and nothing in this tree flattens a string into one.
//!
//! # The four honesty notes on the byte comparison
//!
//! 1. The counted side is **real encoded bytes**: every entry is put through
//!    `f_abi::semantic::Delta::encode`, and the same `(Sqe, payload)` pair that
//!    is counted is the pair the far side decodes. Nothing is estimated.
//! 2. An entry costs [`LINK_ENTRY_BYTES`] — a whole submission entry and a whole
//!    payload slot — and not the width of the record inside it. A ring moves
//!    fixed-size entries, so counting the useful prefix would be counting a cost
//!    nobody pays.
//! 3. The pixel side is **one uncompressed frame**: width times height times
//!    [`BYTES_PER_PIXEL`], which is the *upper* bound of what pixels cost and is
//!    what an unencoded framebuffer copy actually moves. A real remote-desktop
//!    protocol compresses, and this tree has measured none, so the ratio below
//!    is against a straw man in exactly one direction and the honest reading is
//!    *the declaration is three orders of magnitude smaller than the raw
//!    framebuffer and an unmeasured amount smaller than a compressed one.*
//! 4. The comparison is per frame of a **static** declaration, which is where a
//!    declaration wins by the most. A tree that changed every node every frame
//!    would send its whole self at the refresh rate, and the useful number would
//!    be the one this module makes easy to get instead: [`Crossed`] over as many
//!    frames as actually crossed.
//!
//! # No clock, no randomness, no float
//!
//! [`Pacer::offer`] takes the arrival stamp as an argument and reads no clock —
//! RFC 0004, and the reason is not ceremony: a pacing decision that read a clock
//! could not be replayed, and a remote that cannot be replayed cannot be
//! debugged from a capture. The day this runs for real the stamp is
//! `f_env::Env`'s and the caller is the compositor. Every ratio here is integer
//! division with the scale in the name.

use f_abi::Sqe;
use f_abi::scene::Delta as SceneDelta;
use f_abi::semantic::{
    Commit, DeclareNode, Delta, Entry, Fault, Handshake, NO_INTENT, NO_NODE, PAYLOAD_BYTES,
    Received, Refusal, Session, SetState,
};
use f_interface::node::{CapRef, Constraints, Flow, Node, NodeId, Role, StateSet};
use f_interface::solve::{Demand, Layout, Scope, Work};
use f_interface::token::{Resolved, Theme};

use crate::emit::{self, Emitted, Surface};
use crate::tree::{NODES_MAX, Rejected, Staged, Tree};
use crate::vocabulary::Interface;

/// What one entry costs on the link.
///
/// A whole submission entry and a whole payload slot, because that is what a
/// ring moves: `f_abi::layout` sizes a channel's submission area at sixty-four
/// bytes per entry and the payload stride is `PAYLOAD_BYTES` whatever the record
/// inside it is. Counting `Entry::width` instead would be counting the useful
/// prefix of a cost nobody avoids paying.
/// Unit: bytes.
pub const LINK_ENTRY_BYTES: u64 = (core::mem::size_of::<Sqe>() + PAYLOAD_BYTES) as u64;

/// What one device pixel costs, uncompressed.
///
/// Four: eight bits of each of three channels and eight of alpha, which is the
/// format `f_abi::scene`'s paints are in and what a framebuffer copy actually
/// moves. It is the *upper* bound on the pixel side of every comparison here —
/// see the module's honesty notes — and a smaller number would be a compression
/// ratio this tree has not measured.
/// Unit: bytes per pixel.
pub const BYTES_PER_PIXEL: u64 = 4;

/// The scale a `x1000` value carries.
/// Unit: none — the scale of a thousandths value.
const THOUSAND: i64 = 1_000;

/// The scale a `pt_x10` value carries.
/// Unit: none — the scale of a tenths-of-a-point value.
const TEN: i64 = 10;

/// The far side's display: what it is made of, and what it is scaled at.
///
/// Two numbers about density and not one, because they are two facts and a
/// remote that conflated them would be a remote that could not tell a sharp
/// screen from a large interface:
///
/// - [`Panel::density_x1000`] is the **theme's** scale, and it reaches the
///   layout: the resolved em is the text size times this, every floor in
///   `f_interface::token` is in ems, and `Demand::of` is where they become the
///   numbers a solve works in. Doubling it makes the interface bigger.
/// - [`Panel::px_per_pt_x1000`] is the **panel's** own resolution, and it
///   reaches nothing but the transform: `f_semantic::emit` converts a solved
///   point to a device pixel with it. Doubling it makes the same interface
///   sharper and no larger.
///
/// *What would reverse the pair:* a display type arriving from `E3-B06d`
/// carrying both, at which point this struct is that type and the two fields
/// stop being this module's to name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Panel {
    /// The display scale the far side resolves its theme at.
    /// Unit: none — a scale, in thousandths. One thousand is unscaled.
    pub density_x1000: i32,
    /// Device pixels per point.
    /// Unit: device pixels per point, scaled by 1 000. 1 333 is a 96-pixel inch
    /// against a 72-point one.
    pub px_per_pt_x1000: u32,
    /// How wide the frame is.
    /// Unit: device pixels.
    pub width_px: u32,
    /// How tall the frame is.
    /// Unit: device pixels.
    pub height_px: u32,
}

impl Panel {
    /// A 96-pixel-inch laptop at unscaled density: 1 280 by 800.
    ///
    /// The ordinary case, and the one whose arithmetic a reader can check by
    /// hand: 1 280 pixels at 1 333 thousandths of a pixel per point is 960
    /// points, which is 1 280 over 96 inches times 72.
    pub const DESK: Self =
        Self { density_x1000: 1_000, px_per_pt_x1000: 1_333, width_px: 1_280, height_px: 800 };

    /// A 288-pixel-inch handset at double density: 1 080 by 2 340.
    ///
    /// The second density, and it is deliberately not *the same panel twice as
    /// sharp*: it is four times the pixels per point **and** twice the theme
    /// scale, on a third of the width. A projection that had quietly folded the
    /// two densities into one number would place this panel's nodes at four
    /// times the offsets instead of at its own, and the test that separates them
    /// is `each_density_reaches_a_different_stage`.
    pub const HANDSET: Self =
        Self { density_x1000: 2_000, px_per_pt_x1000: 4_000, width_px: 1_080, height_px: 2_340 };

    /// This panel's theme: the caller's, with this panel's density in it.
    ///
    /// The **one** field a panel overrides, and the reason it is one is RFC
    /// 0104: a projection may choose how to present, and it may not choose what
    /// the theme means. A remote that also moved the text size would be a remote
    /// deciding how large the author's words are.
    #[must_use]
    pub fn theme(&self, base: Theme) -> Theme {
        Theme { density_x1000: self.density_x1000, ..base }
    }

    /// What `f_semantic::emit` needs to know about this panel.
    #[must_use]
    pub const fn surface(&self) -> Surface {
        Surface { px_per_pt_x1000: self.px_per_pt_x1000 }
    }

    /// The extent a root is offered along its own flow, on this panel.
    ///
    /// The width in points: pixels times ten, over pixels per point. A panel
    /// claiming no pixels per point is offered nothing rather than dividing by
    /// zero — a display with no resolution is not a display, and a projection is
    /// not the layer that should decide what to do about one.
    /// Unit: pt_x10.
    #[must_use]
    pub const fn viewport_pt_x10(&self) -> i32 {
        if self.px_per_pt_x1000 == 0 {
            return 0;
        }
        let px = self.width_px as i64;
        let scaled = px.saturating_mul(TEN).saturating_mul(THOUSAND);
        let points = scaled / self.px_per_pt_x1000 as i64;
        if points > i32::MAX as i64 { i32::MAX } else { points as i32 }
    }

    /// What one uncompressed frame of this panel costs.
    ///
    /// Width times height times [`BYTES_PER_PIXEL`]. Stated as arithmetic rather
    /// than measured, because there is nothing to measure: it is what the pixels
    /// *are*.
    /// Unit: bytes.
    #[must_use]
    pub const fn frame_bytes(&self) -> u64 {
        (self.width_px as u64).saturating_mul(self.height_px as u64).saturating_mul(BYTES_PER_PIXEL)
    }

    /// What `frames` uncompressed frames of this panel cost.
    /// Unit: bytes.
    #[must_use]
    pub const fn pixel_bytes(&self, frames: u64) -> u64 {
        self.frame_bytes().saturating_mul(frames)
    }
}

/// How often the far side shows what it holds.
///
/// A refresh rate and not a period, because a rate is what a panel advertises
/// and a period is what arithmetic wants; keeping the rate means the conversion
/// happens once, here, rather than in every caller that read a datasheet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cadence {
    /// Refreshes per second.
    /// Unit: hertz. Zero is a panel that shows the first frame and never
    /// refreshes, which [`Cadence::period_us`] answers rather than refusing:
    /// an e-paper panel is a real display and not an error.
    pub refresh_hz: u16,
}

impl Cadence {
    /// Thirty hertz.
    pub const HZ_30: Self = Self { refresh_hz: 30 };

    /// Sixty hertz.
    pub const HZ_60: Self = Self { refresh_hz: 60 };

    /// Microseconds in a second.
    /// Unit: microseconds.
    const SECOND_US: u32 = 1_000_000;

    /// How long one refresh lasts.
    ///
    /// Truncating, and the truncation is in the direction that shows *more*
    /// frames rather than fewer: at 60 hertz the period is 16 666 microseconds
    /// rather than 16 666.67, so a source producing exactly 60 frames a second
    /// is never held back by a rounding this module did.
    /// Unit: microseconds.
    #[must_use]
    pub const fn period_us(self) -> u32 {
        if self.refresh_hz == 0 {
            return u32::MAX;
        }
        Self::SECOND_US / self.refresh_hz as u32
    }
}

/// What a [`Pacer`] decided about one arrival.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paced {
    /// Present it: a refresh has come round since the last one shown.
    Show,
    /// Fold it into the next one shown.
    ///
    /// **Not dropped.** The frame has already been applied to the far side's
    /// tree by the time this is asked, so folding costs nothing and loses
    /// nothing: what the next presentation shows is the tree, which holds this
    /// update and every one folded before it. A remote that dropped the *entry*
    /// instead would be a remote whose tree lags its sender.
    Fold,
}

/// The refresh rate, as a decision about when to present.
///
/// It holds no clock and takes every stamp as an argument — the module's *no
/// clock* — so two runs with the same stamps present the same frames.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pacer {
    /// How often this panel refreshes.
    cadence: Cadence,
    /// When the last presented frame was shown, or `None` before the first.
    /// Unit: microseconds, on the caller's own epoch.
    shown_us: Option<u64>,
    /// How many frames were presented.
    /// Unit: frames.
    shown: u32,
    /// How many were folded into a later one.
    /// Unit: frames.
    folded: u32,
}

impl Pacer {
    /// A panel that has shown nothing yet.
    #[must_use]
    pub const fn at(cadence: Cadence) -> Self {
        Self { cadence, shown_us: None, shown: 0, folded: 0 }
    }

    /// Should the frame that arrived at `arrival_us` be presented?
    ///
    /// The first arrival always is: a panel that held a frame back because it had
    /// never shown one would never show one.
    pub fn offer(&mut self, arrival_us: u64) -> Paced {
        let due = match self.shown_us {
            None => true,
            Some(last) => arrival_us.saturating_sub(last) >= u64::from(self.cadence.period_us()),
        };
        if due {
            self.shown_us = Some(arrival_us);
            self.shown = self.shown.saturating_add(1);
            Paced::Show
        } else {
            self.folded = self.folded.saturating_add(1);
            Paced::Fold
        }
    }

    /// How many frames this panel presented.
    /// Unit: frames.
    #[must_use]
    pub const fn shown(&self) -> u32 {
        self.shown
    }

    /// How many arrived between refreshes and were folded into a later one.
    /// Unit: frames.
    #[must_use]
    pub const fn folded(&self) -> u32 {
        self.folded
    }
}

/// What the link actually carried.
///
/// Counted at the encoder, one statement per entry, for `emit::Emitted`'s
/// reason: a tally computed afterwards from a length is a tally that can be
/// right about a stream nobody sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossed {
    /// Entries encoded and handed to the link.
    /// Unit: entries.
    pub entries: u32,
    /// What they cost, at [`LINK_ENTRY_BYTES`] each.
    /// Unit: bytes.
    pub bytes: u64,
}

impl Crossed {
    /// Nothing crossed.
    pub const NOTHING: Self = Self { entries: 0, bytes: 0 };

    /// One more entry.
    const fn and_one(self) -> Self {
        Self {
            entries: self.entries.saturating_add(1),
            bytes: self.bytes.saturating_add(LINK_ENTRY_BYTES),
        }
    }
}

/// How many times smaller the declaration was than the pixels it replaces.
///
/// Truncating integer division, which under-reports rather than over-reports —
/// the direction a number in an argument's favour should round. Zero link bytes
/// answers zero rather than dividing: nothing crossed, so there is no ratio,
/// and a saturating answer would read as *infinitely better*.
/// Unit: none — a ratio of bytes to bytes.
#[must_use]
pub const fn times_smaller(pixel_bytes: u64, link_bytes: u64) -> u64 {
    if link_bytes == 0 {
        return 0;
    }
    pixel_bytes / link_bytes
}

/// Why a declaration could not be put on the link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unsendable {
    /// A node that names nothing. `f_abi::semantic::DeclareNode` refuses
    /// `NO_NODE` and `node::check` refuses the same identity one layer up; this
    /// is where a sender learns it before the far side does. It carries no
    /// identifier, because a node that names nothing has none to carry: what
    /// names it is its position in the tree the caller passed.
    Unnamed,
    /// The vocabulary this build speaks does not name a role this tree carries,
    /// or a state bit it stands in. Unreachable between two identical builds and
    /// not unreachable between two builds — which is RFC 0083's whole subject.
    NotNamed(Refusal),
    /// A frame nobody named. `Commit::frame_token` refuses zero, because a frame
    /// with no name is a frame a refusal cannot be reported against.
    UnnamedFrame,
    /// This build agrees with nobody, including itself.
    NoAgreement(Refusal),
}

/// Why a role cannot be presented on the far side.
///
/// Two, and both are facts about a **machine boundary** rather than opinions
/// about a design — which is the test a variant here has to pass. A role that is
/// merely hard to draw is not in this enum; it is drawn badly, and that is the
/// projection's problem to improve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unpresentable {
    /// A [`Role::Canvas`]'s pixels are drawn by the application, on the
    /// application's machine, by code no entry in this format carries. RFC 0078
    /// calls the canvas the loophole that could eat the thesis from inside; at a
    /// machine boundary it is not a loophole, it is a wall. *What would reverse
    /// this:* the day a canvas's own content crosses — as geometry, or as the
    /// encoded pixel stream this projection exists to not be — at which point
    /// the remote presents one and this variant is the argument for how.
    Pixels,
    /// A [`Role::Image`]'s content is `Content::Media(CapRef)`, and a capability
    /// reference is *in the declaring peer's own space*: `abi/src/cap.rs` says a
    /// handle is a slot and a generation in one table, and the far side holds a
    /// different table. A remote that presented an image would be resolving an
    /// authority nobody granted it, against whatever its own slot happened to
    /// hold. *What would reverse this:* a transfer that moves the object a
    /// `CapRef` names across the boundary, which is `ring`'s to define and not
    /// this module's to assume.
    Media,
}

impl Unpresentable {
    /// One line, for a report with no room for a match.
    #[must_use]
    pub const fn message(self) -> &'static str {
        match self {
            Self::Pixels => "a canvas is drawn by the application, on the application's machine",
            Self::Media => "a media reference is a capability in the declaring peer's own table",
        }
    }
}

/// Why a received tree did not reach the far side's graph.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A role this remote cannot present, named with the node that carries it.
    ///
    /// **The exit's third clause.** It is a refusal and not a skip: a projection
    /// that omitted the node would present a tree the sender does not have, and
    /// the sender would never learn which node went missing. The frame is
    /// refused whole, on `emit`'s terms — a frame that never closes rather than a
    /// graph that half agrees with its author.
    Unpresentable {
        /// Which node.
        /// Unit: none — a node identifier.
        node: u64,
        /// What it is.
        role: Role,
        /// Why not.
        why: Unpresentable,
    },
    /// An ordinal this build admitted and cannot turn into a role.
    ///
    /// Unreachable while `Role::from_index` and the admission read the same
    /// list, which they do — `f_semantic::vocabulary` calls the one and `abi`
    /// mints the other. It is here because the alternative is an `expect`, and
    /// an unreachable panic in a projection is a panic in somebody's compositor.
    /// Unit: none — the ordinal, as it crossed.
    NotARole(u16),
    /// A node whose parent this walk has not seen yet.
    ///
    /// The tree's own slots are in declaration order — `Tree::apply` refuses a
    /// declaration whose parent it does not hold — but a removal frees a slot
    /// that a later declaration fills, so the order is a property of a history
    /// rather than of the type. Checked rather than assumed for that reason.
    /// Unit: none — a node identifier.
    OutOfOrder(u64),
    /// The tree holds more nodes than this restatement can carry.
    ///
    /// Unreachable while both bounds are [`NODES_MAX`], and kept because the two
    /// are separate constants that could stop agreeing.
    Full,
    /// The layout refused the declaration this restatement built.
    Undeclarable(f_interface::solve::Refused),
    /// The solve pass was larger than the scope allowed.
    Cascade(f_interface::solve::Cascade),
    /// The projection refused, which is `emit`'s to explain.
    Unprojectable(emit::Refused),
}

/// Why a peer's entry did not land.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lost {
    /// The session refused it: the entry, and where in the frame it was.
    Session(Fault),
    /// The tree refused the frame the commit closed.
    Tree(Rejected),
}

/// What the far side does with a role, and the whole of what it decides.
///
/// **Twenty-two arms and no wildcard**, so a twenty-third role is `error[E0004]`
/// here and whoever adds one is asked what a remote does with it. The exit says
/// the exhaustiveness is *what stops a role the remote cannot present from
/// crossing as nothing*, and that is the shape of the answer rather than the
/// answer: the two roles that cannot cross are refusals with names, not arms
/// that quietly return the identity.
///
/// # Why a flow is what this answers
///
/// Because the link carries none. `f_abi::semantic`'s `Closed` says it in the
/// format's own words — *a node's flow belongs to a layout entry this format
/// does not yet have* — so a receiver either refuses to lay anything out or
/// decides. RFC 0121 argues the decision; what is decided is narrow, and it is
/// one question per role: **when this role has children, how are they arranged?**
///
/// - [`Flow::Block`] for the things that stack: a surface, a group, a list, a
///   tree, a table, and a [`Role::Choice`], whose alternatives are a column
///   wherever they are not a column by fashion.
/// - [`Flow::Inline`] for the things that run along: a [`Role::Row`] and its
///   [`Role::Cell`]s, a [`Role::Item`] — whose contents are *a line of an item*
///   — and a [`Role::Track`], which is a lane and runs the way time does.
/// - [`Flow::Own`] for everything that is a mark. It is not *no arrangement*, it
///   is `canvas.rs`'s escape read from this side: whatever is under a mark is
///   the mark's business, and a remote that arranged a control's interior would
///   be inventing an interior.
///
/// Every one of those is a default the author did not write, which is the whole
/// of what is lost by the format carrying no layout, and it is stated here in
/// one place so that a reader can disagree with it in one place.
///
/// # Errors
///
/// [`Unpresentable`], which is two facts about a machine boundary.
#[must_use = "a role that cannot be presented is a refusal, not a skip"]
pub const fn presentable(role: Role) -> Result<Flow, Unpresentable> {
    match role {
        Role::Surface => Ok(Flow::Block),
        Role::Group => Ok(Flow::Block),
        Role::List => Ok(Flow::Block),
        Role::Tree => Ok(Flow::Block),
        Role::Item => Ok(Flow::Inline),
        Role::Table => Ok(Flow::Block),
        Role::Row => Ok(Flow::Inline),
        Role::Cell => Ok(Flow::Inline),
        Role::Separator => Ok(Flow::Own),
        Role::Label => Ok(Flow::Own),
        Role::Text => Ok(Flow::Own),
        Role::Image => Err(Unpresentable::Media),
        Role::Status => Ok(Flow::Own),
        Role::Command => Ok(Flow::Own),
        Role::Toggle => Ok(Flow::Own),
        Role::Entry => Ok(Flow::Own),
        Role::Choice => Ok(Flow::Block),
        Role::Number => Ok(Flow::Own),
        Role::Canvas => Err(Unpresentable::Pixels),
        Role::Track => Ok(Flow::Inline),
        Role::Clip => Ok(Flow::Own),
        Role::Marker => Ok(Flow::Own),
    }
}

/// The declaration, as the entries that carry it.
///
/// A handshake, a `DeclareNode` per node in declaration order, a `SetState` for
/// every node that stands somehow, and a `Commit`. The sink is handed the
/// encoded pair rather than the [`Delta`], because what crosses a link is bytes
/// and a sink that took the value would let a caller count a cost it never paid.
///
/// # What is not sent, and why each absence is a decision
///
/// No `SetContent` and no `SetRelations`: [`crate::tree::Tree`] refuses both
/// with `Rejected::NotStored`, so sending them would be sending entries the far
/// side is required to refuse — a frame that fails by design is worse than a
/// field that is missing by argument. No `Remove`, because this is a first
/// frame; a reconciler's later frames are `E3-B06m`'s.
///
/// # Errors
///
/// [`Unsendable`]. The sink may already hold entries when one is returned, and
/// that is deliberate for `emit`'s reason: a frame is atomic at its `Commit`, so
/// a refusal before the commit is a frame that never closes.
pub fn send<S>(tree: &[Node], frame_token: u64, sink: &mut S) -> Result<Crossed, Unsendable>
where
    S: FnMut(&Sqe, &[u8; PAYLOAD_BYTES]),
{
    if frame_token == 0 {
        return Err(Unsendable::UnnamedFrame);
    }
    let agreed = Handshake::HERE.negotiate().map_err(Unsendable::NoAgreement)?;
    let mut crossed = Crossed::NOTHING;
    let mut put = |body: Entry, crossed: &mut Crossed| {
        let (entry, payload) = around(body).encode();
        sink(&entry, &payload);
        *crossed = crossed.and_one();
    };

    put(Entry::DeclareVocabulary(Handshake::HERE), &mut crossed);
    for node in tree {
        if !node.id.is_named() {
            return Err(Unsendable::Unnamed);
        }
        // Through the admission rather than around it. `Role::index` is one half
        // of RFC 0083's rule and `admit_role` is where the other half — the
        // agreed version — is applied; a sender that wrote the ordinal straight
        // into the record would be the second decoder that RFC names, facing the
        // other way.
        let role = agreed
            .admit_role(node.role.index() as u16, &Interface)
            .map_err(Unsendable::NotNamed)?;
        put(
            Entry::DeclareNode(DeclareNode {
                node: node.id.value(),
                parent: node.parent.value(),
                before: NO_NODE,
                role,
                intent: node.intent.map_or(NO_INTENT, CapRef::bits),
            }),
            &mut crossed,
        );
    }
    for node in tree {
        if node.state == StateSet::NONE {
            continue;
        }
        let state =
            agreed.admit_state(node.state.bits(), &Interface).map_err(Unsendable::NotNamed)?;
        put(Entry::SetState(SetState { node: node.id.value(), state }), &mut crossed);
    }
    put(Entry::Commit(Commit { frame_token }), &mut crossed);
    Ok(crossed)
}

/// The envelope a declaration crosses in.
///
/// Every field zero: `user_data` because correlation belongs to whoever submits
/// on a real ring and a projection that invented one would be inventing an
/// identity the author did not ask for, `class` because the scheduling class is
/// the submitter's, `payload_offset` because where a payload sits in an arena is
/// settled by whoever writes it there, and `flags` because `NO_CQE` on the
/// handshake is refused and on the rest would suppress a completion this module
/// does not know the caller can do without. `emit::around` is the same decision
/// one layer down, and the two are written out separately because the two
/// formats are separate.
const fn around(body: Entry) -> Delta {
    Delta { user_data: 0, class: 0, payload_offset: 0, flags: 0, body }
}

/// The far side: a session, a frame under assembly, and the tree it lands in.
///
/// It is the receiver and nothing else. It holds no [`Panel`], no [`Pacer`] and
/// no [`Layout`], because those are the *presentation's* and one remote may
/// present the same tree on two panels — which is the exit's first clause, and a
/// receiver that owned a panel could not satisfy it.
#[derive(Clone, Copy, Debug)]
pub struct Remote {
    /// What has been agreed on this channel, and what has been refused.
    session: Session,
    /// The frame being assembled.
    staged: Staged,
    /// What has been applied.
    tree: Tree,
    /// How many frames have landed.
    /// Unit: frames.
    frames: u64,
}

impl Remote {
    /// A far side with nothing agreed and nothing held.
    #[must_use]
    pub const fn opening(epoch: u32) -> Self {
        Self {
            session: Session::opening(epoch),
            staged: Staged::EMPTY,
            tree: Tree::EMPTY,
            frames: 0,
        }
    }

    /// The tree as it stands, for a projection to read.
    #[must_use]
    pub const fn tree(&self) -> &Tree {
        &self.tree
    }

    /// How many frames have been applied.
    /// Unit: frames.
    #[must_use]
    pub const fn frames(&self) -> u64 {
        self.frames
    }

    /// Take one entry off the link.
    ///
    /// `Ok(None)` for an entry that joins the frame being assembled, and
    /// `Ok(Some(..))` for the commit that closes it — at which point the whole
    /// frame has been applied to the tree, or none of it has.
    ///
    /// # Errors
    ///
    /// [`Lost`]: the session's refusal with the position of the entry that
    /// earned it, or the tree's refusal of the frame as a whole.
    pub fn receive(
        &mut self,
        entry: &Sqe,
        payload: &[u8; PAYLOAD_BYTES],
    ) -> Result<Option<crate::tree::Applied>, Lost> {
        match self.session.accept(entry, payload, &Interface) {
            Ok(Received::Staged(delta)) => {
                self.staged.offer(delta).map_err(Lost::Tree)?;
                Ok(None)
            }
            Ok(Received::Frame { key, .. }) => {
                let applied = self.tree.apply(&self.staged, key).map_err(Lost::Tree)?;
                self.staged.clear();
                self.frames = self.frames.saturating_add(1);
                Ok(Some(applied))
            }
            Err(fault) => Err(Lost::Session(fault)),
        }
    }
}

/// The declaration as the far side restates it.
///
/// **A restatement and not a copy**, and the difference is the whole of what a
/// remote is: what arrived is identity, place, order, role, intent and state,
/// and what a projection consumes is `f_interface::node::Node`. The fields in
/// between — the constraints — are this side's, from [`presentable`] and from
/// the theme. So this value is the far side's *own* declaration of the author's
/// structure, and calling it a copy would hide exactly the thing RFC 0121 is
/// about.
///
/// It is [`NODES_MAX`] nodes of stack and no allocation, which is a few
/// kilobytes: a `Node` carries its text inline — `f_interface::node::Text` is
/// bounded and owned so that the system can outlive the writer — and that cost
/// is paid here even though no text crossed. *What would reverse this:* a
/// receiver bound larger than a frame's worth of stack, at which point the tree
/// is walked into the layout node by node and there is no array.
#[derive(Clone, Copy, Debug)]
pub struct Restated {
    /// The restated nodes, in the order the far side's tree holds them.
    nodes: [Node; NODES_MAX],
    /// How many of them mean anything.
    /// Unit: nodes.
    len: usize,
}

impl Restated {
    /// Nothing restated.
    pub const EMPTY: Self = Self {
        nodes: [Node::new(NodeId::UNNAMED, NodeId::UNNAMED, Role::Surface); NODES_MAX],
        len: 0,
    };

    /// The nodes, in declaration order.
    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes[..self.len]
    }

    /// How many there are.
    /// Unit: nodes.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.len
    }

    /// Is there nothing to present?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

/// What the far side holds, as a declaration it can present.
///
/// The one place an ordinal becomes a [`Role`] on this route, and it is
/// `Role::from_index` — RFC 0083 names a remote projection at a machine boundary
/// as one of the three routes that must pass through that function, and this is
/// that route. An array indexed by the ordinal would need no new code and would
/// be the second decoder that RFC refuses.
///
/// # Errors
///
/// [`Refused::Unpresentable`] for a role this remote cannot present, naming the
/// node and the reason; [`Refused::OutOfOrder`] for a node whose parent this
/// walk has not seen; [`Refused::Full`] past [`NODES_MAX`].
pub fn restate(tree: &Tree) -> Result<Restated, Refused> {
    let mut restated = Restated::EMPTY;
    for held in tree.each() {
        let Some(role) = Role::from_index(held.role().get() as usize) else {
            return Err(Refused::NotARole(held.role().get()));
        };
        let flow = presentable(role).map_err(|why| Refused::Unpresentable {
            node: held.id(),
            role,
            why,
        })?;
        if held.parent() != NO_NODE
            && !restated.nodes[..restated.len].iter().any(|seen| seen.id.value() == held.parent())
        {
            return Err(Refused::OutOfOrder(held.id()));
        }
        if restated.len >= NODES_MAX {
            return Err(Refused::Full);
        }
        let mut node = Node::new(NodeId::new(held.id()), NodeId::new(held.parent()), role)
            .with_layout(Constraints::flowing(flow))
            .with_state(
                held.state().map_or(StateSet::NONE, |bits| StateSet::from_bits(bits.get())),
            );
        if held.intent() != NO_INTENT {
            node = node.with_intent(CapRef::new(held.intent()));
        }
        restated.nodes[restated.len] = node;
        restated.len += 1;
    }
    Ok(restated)
}

/// What one presentation did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shown {
    /// What the solve cost.
    pub work: Work,
    /// What the projection emitted.
    pub emitted: Emitted,
}

/// Present a restated declaration on one panel.
///
/// Declare, solve, project: the same three stages the local path runs, in the
/// same order, through the same two modules. **There is no remote-only stage**,
/// and that is the property worth having rather than a tidiness: a remote with
/// its own solver would be a second layout engine, and the first thing two
/// layout engines do is disagree.
///
/// The `layout` is the caller's and is kept between frames, because that is
/// where incrementality lives — `Layout::solve` re-solves the subtree of what
/// changed, and a projection that built a fresh layout per frame would have
/// thrown that away before the first frame it could have used it on.
///
/// # Errors
///
/// [`Refused`], which is this module's, the layout's and the projection's
/// refusals in one enum — because the caller that has to act on any of them is
/// the same caller.
pub fn present<S>(
    restated: &Restated,
    panel: Panel,
    resolved: &Resolved,
    layout: &mut Layout,
    scope: Scope,
    sink: &mut S,
) -> Result<Shown, Refused>
where
    S: FnMut(SceneDelta),
{
    for node in restated.nodes() {
        layout
            .declare(node.id, node.parent, Demand::of(resolved, node))
            .map_err(Refused::Undeclarable)?;
    }
    let work = layout.solve(panel.viewport_pt_x10(), scope).map_err(Refused::Cascade)?;
    let emitted = emit::project(restated.nodes(), layout, resolved, panel.surface(), sink)
        .map_err(Refused::Unprojectable)?;
    Ok(Shown { work, emitted })
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::cell::Cell;

    use f_abi::scene::Entry as SceneEntry;
    use f_abi::semantic::Remove;
    use f_interface::example::{SETTINGS_NODES, settings_panel};
    use f_interface::token::{Metric, resolve};

    /// The channel epoch every fixture opens on.
    ///
    /// Not zero, so that a session that ignored the epoch it was given would be
    /// indistinguishable from one that defaulted.
    const EPOCH: u32 = 3;

    /// The frame the declaration arrives in.
    ///
    /// Distinct in every byte that is not zero, for `emit`'s reason: a header
    /// that wrote the token at the wrong offset cannot produce the same value.
    const FRAME: u64 = 0x0BAD_F00D_0000_0011;

    /// How much of a frame one pass may re-solve, here.
    ///
    /// Unbounded, because what this module demonstrates is not incrementality —
    /// `E3-B06e` and `E3-B06f` hold that — and a scope that refused would make
    /// every fixture below a test of `Cascade`.
    const WHOLE: Scope = Scope::UNBOUNDED;

    /// How many entries the panel comes to.
    ///
    /// One handshake, fifteen declarations, seven states — the choice, its two
    /// items, the number, the toggle, the entry and the command — and one
    /// commit. Written as the sum rather than as twenty-four so that a node
    /// added to the application moves the term it belongs to.
    /// Unit: entries.
    const PANEL_ENTRIES: u32 = 1 + SETTINGS_NODES as u32 + 7 + 1;

    /// What the panel costs on the link.
    /// Unit: bytes.
    const PANEL_LINK_BYTES: u64 = PANEL_ENTRIES as u64 * LINK_ENTRY_BYTES;

    /// Send a declaration to a far side, and hand back what crossed.
    ///
    /// **The link is a function call**, and the entries counted are the entries
    /// received: there is no buffer in between for one side to be counted twice
    /// or filtered. A refusal at either end panics, because every fixture here
    /// sends a tree this build accepts and a refusal would be a fixture that had
    /// stopped testing what it says.
    fn link(tree: &[Node], remote: &mut Remote) -> Crossed {
        send(tree, FRAME, &mut |entry, payload| {
            remote.receive(entry, payload).expect("the far side accepts what this build sends");
        })
        .expect("the fixture is sendable")
    }

    /// A far side holding the settings panel.
    fn received() -> Remote {
        let mut remote = Remote::opening(EPOCH);
        let crossed = link(&settings_panel(), &mut remote);
        assert_eq!(crossed.entries, PANEL_ENTRIES);
        remote
    }

    /// A buffer for one frame of transforms.
    const STREAM_MAX: usize = 64;

    /// A frame's translations, held so that two panels can be compared.
    struct Placed {
        /// Node and translation, in the order they were emitted.
        /// Unit: (none, px_x65536, px_x65536).
        at: [(u32, i64, i64); STREAM_MAX],
        /// How many of them mean anything.
        /// Unit: transforms.
        len: usize,
    }

    impl Placed {
        const EMPTY: Self = Self { at: [(0, 0, 0); STREAM_MAX], len: 0 };

        /// Keep every `SetTransform` and ignore the rest.
        fn watch(&mut self, delta: SceneDelta) {
            if let SceneEntry::SetTransform(put) = delta.body {
                assert!(self.len < STREAM_MAX, "the fixture wants a larger buffer");
                self.at[self.len] = (put.node, put.tx_x65536, put.ty_x65536);
                self.len += 1;
            }
        }

        fn each(&self) -> &[(u32, i64, i64)] {
            &self.at[..self.len]
        }

        /// Where one node was put.
        fn of(&self, node: u32) -> (i64, i64) {
            let found = self.each().iter().find(|(id, _, _)| *id == node);
            let (_, tx, ty) = found.expect("the node was projected");
            (*tx, *ty)
        }
    }

    /// Present what a far side holds, on one panel.
    ///
    /// **The layout is the caller and not this function**, and the reason is a
    /// finding rather than a style: `f_interface::solve::Layout` is three
    /// quarters of a megabyte — ten thousand slots and a sixteen-thousand-bucket
    /// index, sized so that nothing above it allocates — and a debug build gives
    /// each `Layout::new()` a temporary as well as a destination. Two of them in
    /// one test frame is past a test thread stack, which this module found by
    /// overflowing one. So a test holds exactly one, which is also what a
    /// compositor does: the layout is where incrementality lives and a fresh one
    /// per frame would throw that away.
    fn present_on(remote: &Remote, panel: Panel, layout: &mut Layout) -> (Shown, Placed) {
        let (resolved, report) = resolve(&panel.theme(Theme::DEFAULT));
        assert!(report.is_clean(), "a panel density the theme layer argues with: {report:?}");
        let restated = restate(remote.tree()).expect("the tree holds nothing this remote refuses");
        let mut placed = Placed::EMPTY;
        let shown = present(&restated, panel, &resolved, layout, WHOLE, &mut |delta| {
            placed.watch(delta);
        })
        .expect("the restated tree presents");
        (shown, placed)
    }

    /// Where every node of the panel is solved to, on [`Panel::DESK`].
    ///
    /// **Derived by hand from the theme rather than read off a run**, which is
    /// the only way these numbers test anything. At density one thousand the
    /// resolved em is 105 tenths of a point, a rule is six hundredths of that —
    /// six — and an operable floor is the larger of eighteen points and the em
    /// with two rules round it, which is 180 against 117. So every node carrying
    /// an intent is 180 and the separator is 6; nothing else declares anything,
    /// because **no constraint crossed the link**.
    ///
    /// A container is the sum of its children: the choice is two items at 180,
    /// the output group is 0 + 360 + 0 + 180 + 180 + 0, the input group is 0 +
    /// 180 + 180. An offset is a cursor along the parent flow, so the output
    /// group starts at 0, the rule follows it at 720 and the input group at 726.
    /// Unit: (none, pt_x10).
    const DESK_OFFSETS_PT_X10: [(u64, i32); SETTINGS_NODES] = [
        (1, 0),
        (10, 0),
        (11, 0),
        (12, 0),
        (13, 0),
        (14, 180),
        (15, 360),
        (16, 360),
        (17, 540),
        (18, 720),
        (2, 720),
        (20, 726),
        (21, 726),
        (22, 726),
        (23, 906),
    ];

    /// The same panel on [`Panel::HANDSET`], and **every number that is not zero
    /// is different**.
    ///
    /// At density two thousand the em is 210, a rule is 12, and the operable
    /// floor is 210 with two rules round it — 234, which is now above eighteen
    /// points rather than below it. The same arithmetic as above with those
    /// three numbers, which is what *the layout is solved at the far side
    /// density* means when it is not a slogan.
    /// Unit: (none, pt_x10).
    const HANDSET_OFFSETS_PT_X10: [(u64, i32); SETTINGS_NODES] = [
        (1, 0),
        (10, 0),
        (11, 0),
        (12, 0),
        (13, 0),
        (14, 234),
        (15, 468),
        (16, 468),
        (17, 702),
        (18, 936),
        (2, 936),
        (20, 948),
        (21, 948),
        (22, 948),
        (23, 1_182),
    ];

    /// **The exit first clause.** One declaration, sent once, presented twice.
    ///
    /// The application is `f_interface::example::settings_panel` and it is the
    /// value this test hands to [`send`] — not a copy of it, not a variant with
    /// a field added, and not a second authoring. What reaches the far side is
    /// what that function returns, and the far side is handed no other argument
    /// about it.
    #[test]
    fn one_declaration_crosses_once_and_is_presented_on_two_panels() {
        let declared = settings_panel();
        let mut remote = Remote::opening(EPOCH);
        let crossed = link(&declared, &mut remote);

        // The far side holds the declaration: every identity, every parent,
        // every role, every intent and every state. Asserted against the
        // application own array, which is the one witness here that has not been
        // through this module.
        assert_eq!(remote.frames(), 1);
        assert_eq!(remote.tree().live(), SETTINGS_NODES);
        for node in &declared {
            let held = remote.tree().node(node.id.value()).expect("the node crossed");
            assert_eq!(held.parent(), node.parent.value(), "node {} moved", node.id.value());
            assert_eq!(
                Role::from_index(held.role().get() as usize),
                Some(node.role),
                "node {} changed what it is",
                node.id.value()
            );
            assert_eq!(held.intent(), node.intent.map_or(NO_INTENT, CapRef::bits));
            assert_eq!(
                held.state().map_or(StateSet::NONE, |bits| StateSet::from_bits(bits.get())),
                node.state,
                "node {} stands differently",
                node.id.value()
            );
        }

        let mut layout = Layout::new();
        let (desk, _desk_at) = present_on(&remote, Panel::DESK, &mut layout);
        let (handset, _handset_at) = present_on(&remote, Panel::HANDSET, &mut layout);

        // The same fifteen nodes and the same eleven marks: the structure is the
        // application, and neither panel changed it. `Emitted` is `emit`s own
        // count, so this is also the statement that a remote frame is an
        // ordinary frame.
        assert_eq!(desk.emitted, handset.emitted);
        assert_eq!(desk.emitted.nodes, SETTINGS_NODES as u32);
        assert_eq!(desk.emitted.creates, SETTINGS_NODES as u32);
        assert_eq!(desk.emitted.transforms, SETTINGS_NODES as u32);
        assert_eq!(desk.work.nodes, SETTINGS_NODES as u32);

        // And the declaration is untouched by all of it.
        assert_eq!(declared, settings_panel(), "the application was modified");
        assert_eq!(crossed.bytes, PANEL_LINK_BYTES);
    }

    /// **The exit second clause, density half.** Each density reaches a
    /// different stage, and this fails if either stops mattering.
    ///
    /// The theme density reaches the **solve**: the offsets above are derived
    /// from the em, and the two tables are not one another times anything. The
    /// panel pixels per point reach the **transform** and nothing else: a third
    /// panel, identical to the desk but twice as sharp, solves to exactly the
    /// same points and emits exactly twice the translation.
    ///
    /// *The edits that make this go red:* `Panel::theme` returning the base
    /// theme unchanged, at which point the two offset tables would have to agree
    /// and they differ at ten of fifteen nodes — the other five are zero on both
    /// panels, because a first child sits at its parent's origin at any density;
    /// or `Panel::surface` answering
    /// a constant, at which point the sharp panel transforms stop doubling.
    #[test]
    fn each_density_reaches_a_different_stage() {
        let remote = received();
        // One layout for the whole test, for `present_on`s reason.
        let mut layout = Layout::new();

        // Half one: the theme density, in the solve.
        let (desk_resolved, _) = resolve(&Panel::DESK.theme(Theme::DEFAULT));
        let (handset_resolved, _) = resolve(&Panel::HANDSET.theme(Theme::DEFAULT));
        assert_eq!(desk_resolved.em_pt_x10(), 105);
        assert_eq!(handset_resolved.em_pt_x10(), 210);
        assert_eq!(desk_resolved.metric(Metric::Density), 1_000);
        assert_eq!(handset_resolved.metric(Metric::Density), 2_000);

        for (panel, expected) in
            [(Panel::DESK, DESK_OFFSETS_PT_X10), (Panel::HANDSET, HANDSET_OFFSETS_PT_X10)]
        {
            let (resolved, _) = resolve(&panel.theme(Theme::DEFAULT));
            let restated = restate(remote.tree()).expect("nothing is refused");
            let mut nowhere = |_delta: SceneDelta| {};
            present(&restated, panel, &resolved, &mut layout, WHOLE, &mut nowhere)
                .expect("presents");
            for (node, offset_pt_x10) in expected {
                let (solved, _extent) =
                    layout.placed_pt_x10(NodeId::new(node)).expect("the node was solved");
                assert_eq!(
                    solved, offset_pt_x10,
                    "node {node} solved to {solved} and not {offset_pt_x10} at density {}",
                    panel.density_x1000
                );
            }
        }

        // Half two: the panel own pixels, in the transform. Same theme, twice
        // the pixels per point, and the difference is exactly a doubling — which
        // is what makes half one a *layout* rather than a scale.
        let sharp = Panel { px_per_pt_x1000: Panel::DESK.px_per_pt_x1000 * 2, ..Panel::DESK };
        let (_, flat_at) = present_on(&remote, Panel::DESK, &mut layout);
        let (sharper, sharp_at) = present_on(&remote, sharp, &mut layout);

        // **The strongest single assertion in this test.** The sharp panel
        // re-declared the same fifteen demands, so the solver found nothing to
        // do — which it could only find if the panel pixels per point had
        // reached none of them. A `px_per_pt_x1000` that leaked into the theme
        // or into `Demand::of` would put work here.
        assert_eq!(sharper.work, Work::NOTHING, "the panel own pixels reached the solve");

        assert_eq!(flat_at.len, sharp_at.len);
        for ((node, tx, ty), (sharp_node, sharp_tx, sharp_ty)) in
            flat_at.each().iter().zip(sharp_at.each())
        {
            // Doubled **to within the truncation**, and the one unit is a
            // finding rather than a slack: `emit::device_x65536` truncates after
            // multiplying, so twice the scale can keep a sixty-five-thousandth
            // of a pixel that the single scale dropped — node 14 is exactly that
            // case, at 3 144 941 against 3 144 940. A density that had reached
            // the geometry instead would be out by whole points.
            assert_eq!(node, sharp_node);
            assert!(
                (0..=1).contains(&(sharp_tx - tx * 2)),
                "node {node} is at {sharp_tx} on a twice-sharp panel and not at {}",
                tx * 2
            );
            assert!(
                (0..=1).contains(&(sharp_ty - ty * 2)),
                "node {node} is at {sharp_ty} on a twice-sharp panel and not at {}",
                ty * 2
            );
        }

        // And the two densities are not one another times anything, which is
        // the assertion a scale factor could not survive: the desk puts the
        // second item at 180 points and the handset at 234, and 234 over 180 is
        // not 4 000 over 1 333.
        let (_, desk_at) = present_on(&remote, Panel::DESK, &mut layout);
        let (_, handset_at) = present_on(&remote, Panel::HANDSET, &mut layout);
        let (_, desk_ty) = desk_at.of(14);
        let (_, handset_ty) = handset_at.of(14);
        assert_ne!(
            handset_ty * i64::from(Panel::DESK.px_per_pt_x1000),
            desk_ty * i64::from(Panel::HANDSET.px_per_pt_x1000),
            "the two panels differ by a scale factor, so no density reached the solve"
        );
    }

    /// **The exit second clause, refresh half.** A refresh rate changes what is
    /// shown and not what it costs.
    ///
    /// Six frames arrive at the stamps below. At sixty hertz three are presented
    /// and at thirty hertz two are, from the same arrivals — and the bytes that
    /// crossed are the same number both times, because a declaration is sent
    /// when it changes and a pixel stream is sent when the panel refreshes. That
    /// difference is the whole of *not an encoded pixel stream*, which is why
    /// the last three assertions are the point of this test rather than a
    /// decoration on it.
    ///
    /// # Why there are two links and not one
    ///
    /// Because a single `Crossed` added to both totals in the same iteration
    /// makes `fast_bytes == slow_bytes` true for every possible implementation
    /// of [`Pacer`], [`Cadence`], [`send`] and [`Remote`] — an assertion about
    /// this loop rather than about this module, and it was one until an audit
    /// on 2026-09-24 said so. Each side now has its own far side and its own
    /// call to [`send`], so the two totals come from two control flows and can
    /// differ. The pixel side is read off `shown()` for the same reason: it was
    /// arithmetic over `Cadence::HZ_60.refresh_hz` and `HZ_30.refresh_hz`, two
    /// constants, and it held whatever the pacers did.
    ///
    /// *The edits that make this go red:* a [`Pacer`] that ignores its cadence —
    /// the two counts then agree, and both the `shown` assertions and the pixel
    /// comparison fail; or a sender that put the declaration on the link once
    /// per *presentation* rather than once per change, which is the encoded
    /// pixel stream this module exists to not be, at which point the two byte
    /// totals are three frames against two.
    #[test]
    fn a_refresh_rate_changes_what_is_shown_and_not_what_it_costs() {
        /// When each frame arrived. Handed in, never read from a clock.
        /// Unit: microseconds.
        const ARRIVED_US: [u64; 6] = [0, 5_000, 20_000, 25_000, 40_000, 55_000];

        assert_eq!(Cadence::HZ_60.period_us(), 16_666);
        assert_eq!(Cadence::HZ_30.period_us(), 33_333);

        let mut layout = Layout::new();
        let mut fast = Pacer::at(Cadence::HZ_60);
        let mut slow = Pacer::at(Cadence::HZ_30);
        let mut fast_bytes = 0;
        let mut slow_bytes = 0;
        for arrival_us in ARRIVED_US {
            // Two far sides and two links, one per panel, and a fresh `Remote`
            // per arrival because this is a first frame each time — `Tree`
            // refuses a redeclaration, which is the property `E3-B06m`'s
            // reconciler is for. The link is driven by the arrival and the
            // pacer by the refresh, and nothing here lets the second reach the
            // first: that is what the byte totals below are asserting.
            let mut fast_remote = Remote::opening(EPOCH);
            let mut slow_remote = Remote::opening(EPOCH);
            fast_bytes += link(&settings_panel(), &mut fast_remote).bytes;
            slow_bytes += link(&settings_panel(), &mut slow_remote).bytes;
            if fast.offer(arrival_us) == Paced::Show {
                present_on(&fast_remote, Panel::DESK, &mut layout);
            }
            if slow.offer(arrival_us) == Paced::Show {
                present_on(&slow_remote, Panel::DESK, &mut layout);
            }
        }
        assert_eq!((fast.shown(), fast.folded()), (3, 3));
        assert_eq!((slow.shown(), slow.folded()), (2, 4));
        assert_ne!(fast.shown(), slow.shown(), "the refresh rate reached nothing");

        // Six declarations each, at whatever rate the panel refreshes.
        assert_eq!(fast_bytes, slow_bytes);
        assert_eq!(fast_bytes, ARRIVED_US.len() as u64 * PANEL_LINK_BYTES);

        // And a pixel stream would not have been equal, because it costs one
        // frame per *presentation*. Read off the pacers rather than off the two
        // refresh rates, so that this compares what the pacers did and not what
        // two constants say they should have done.
        assert_ne!(
            Panel::DESK.pixel_bytes(u64::from(fast.shown())),
            Panel::DESK.pixel_bytes(u64::from(slow.shown())),
            "the refresh rate reached neither the link nor the pixels"
        );
    }

    /// **The exit third clause.** What crossed, counted, against what the pixels
    /// would have cost.
    ///
    /// Every number here is arithmetic a reader can check, and the two sides are
    /// produced differently on purpose: the link side is counted one statement
    /// per entry as the entries are *encoded*, and the pixel side is the panel
    /// own multiplication. A tally computed from a length would have agreed with
    /// whatever the encoder did.
    ///
    /// # Who the second party is, and who it is not
    ///
    /// The **far side**, which decoded these bytes and counted what it staged:
    /// `Applied` is `Tree::apply`'s own tally and the receiver reached it
    /// without asking the sender anything. That is a second count of the same
    /// frame, produced by the other end of the link.
    ///
    /// This test used to add `size_of_val(entry) + payload.len()` up in the sink
    /// closure and call it a third witness. It was not one: that expression *is*
    /// the definition of [`LINK_ENTRY_BYTES`], and `send`'s `put` calls the sink
    /// and increments [`Crossed`] in adjacent statements, so the two sides
    /// stepped in lockstep by construction and no defect could separate them.
    /// An audit on 2026-09-24 said so and the claim is withdrawn: on the *byte*
    /// number the witness is the hand-written literal below, and nothing else in
    /// this tree can be.
    ///
    /// The ratio is published unflattered. It is against an **uncompressed**
    /// frame, which is the upper bound of the pixel side — see the module
    /// honesty notes — so the honest sentence is *a declaration is three orders
    /// of magnitude smaller than the raw framebuffer it replaces, and an
    /// unmeasured amount smaller than a compressed one.*
    #[test]
    fn the_bytes_that_cross_are_counted_and_compared_with_pixels() {
        let closed = Cell::new(None);
        let mut remote = Remote::opening(EPOCH);
        let crossed = send(&settings_panel(), FRAME, &mut |entry, payload| {
            // Nothing is counted in this closure. What it would count is
            // `LINK_ENTRY_BYTES` spelled a second way, beside the statement that
            // increments the first one — see the note above.
            if let Some(applied) = remote.receive(entry, payload).expect("the far side accepts it")
            {
                closed.set(Some(applied));
            }
        })
        .expect("the panel is sendable");

        // The far side's count, from the entries it decoded: fifteen
        // declarations, seven states, the one handshake it carried and did not
        // apply, and nothing removed. The commit is added here rather than
        // hidden in a field, because it is an entry on the link and not an edit
        // in a tree — which is the one place these two counts are allowed to
        // disagree, and it is written out so that a reader can see it does.
        let far = closed.get().expect("the frame closed");
        assert_eq!((far.declared, far.stated, far.removed, far.agreements), (15, 7, 0, 1));
        let far_entries = far.declared + far.stated + far.removed + far.agreements + 1;
        assert_eq!(
            far_entries,
            u64::from(crossed.entries),
            "the sender and the receiver counted different frames"
        );

        assert_eq!(LINK_ENTRY_BYTES, 120, "a submission entry is 64 bytes and a payload slot 56");
        assert_eq!(crossed.entries, 24);
        assert_eq!(crossed.entries, PANEL_ENTRIES);
        assert_eq!(crossed.bytes, 2_880);
        // The byte number against the literal, which is the whole of what checks
        // it: 24 entries at 120 bytes. Both numbers are written by hand.
        assert_eq!(crossed.bytes, u64::from(crossed.entries) * 120);

        // The pixel side. Width times height times four, written as the
        // multiplication as well as the product.
        assert_eq!(Panel::DESK.frame_bytes(), 1_280 * 800 * 4);
        assert_eq!(Panel::DESK.frame_bytes(), 4_096_000);
        assert_eq!(Panel::HANDSET.frame_bytes(), 1_080 * 2_340 * 4);
        assert_eq!(Panel::HANDSET.frame_bytes(), 10_108_800);

        // One frame: the declaration is 1 422 times smaller than the desk
        // pixels and 3 510 times smaller than the handset. The handset ratio is
        // larger for a reason worth stating — it has more pixels and the
        // declaration did not grow, which is the shape of the whole argument.
        assert_eq!(times_smaller(Panel::DESK.frame_bytes(), crossed.bytes), 1_422);
        assert_eq!(times_smaller(Panel::HANDSET.frame_bytes(), crossed.bytes), 3_510);

        // One second of a still interface at sixty hertz, which is where the
        // comparison stops being close: the pixels are sent sixty times over and
        // the declaration is not sent again at all.
        assert_eq!(times_smaller(Panel::DESK.pixel_bytes(60), crossed.bytes), 85_333);
        assert_eq!(times_smaller(Panel::DESK.pixel_bytes(60), 0), 0, "nothing crossed, no ratio");
    }

    /// How many roles answer each way.
    ///
    /// **The half the compiler cannot hold.** `error[E0004]` catches a
    /// twenty-third role and it does not catch a wildcard arm written by
    /// somebody who found the match tedious: a `_ => Ok(Flow::Own)` compiles,
    /// presents every role it swallowed as a mark, and is exactly the *crossing
    /// as nothing* the exit names. This census is what such an arm cannot
    /// survive — it pins how many roles give each answer, so collapsing any of
    /// them into one moves two numbers.
    /// Unit: (roles, roles, roles, roles, roles).
    const CENSUS: (u16, u16, u16, u16, u16) = (6, 4, 10, 1, 1);

    /// **The exit fourth clause.** Every role is decided here, and two are
    /// refused by name.
    #[test]
    fn the_far_side_decides_every_role_and_carries_no_wildcard() {
        let mut block = 0;
        let mut inline = 0;
        let mut own = 0;
        let mut pixels = 0;
        let mut media = 0;
        let mut total = 0;
        for role in Role::all() {
            match presentable(role) {
                Ok(Flow::Block) => block += 1,
                Ok(Flow::Inline) => inline += 1,
                Ok(Flow::Wrap) => panic!("{} wraps, and no role should yet", role.name()),
                Ok(Flow::Own) => own += 1,
                Err(Unpresentable::Pixels) => pixels += 1,
                Err(Unpresentable::Media) => media += 1,
            }
            total += 1;
        }
        assert_eq!(total, Role::COUNT, "the vocabulary is closed and this walked all of it");
        assert_eq!((block, inline, own, pixels, media), CENSUS, "the census of the far side moved");
        assert_eq!(presentable(Role::Canvas), Err(Unpresentable::Pixels));
        assert_eq!(presentable(Role::Image), Err(Unpresentable::Media));
        assert!(!Unpresentable::Pixels.message().is_empty());
        assert!(!Unpresentable::Media.message().is_empty());
    }

    /// A role the remote cannot present is refused by name, and nothing is
    /// drawn.
    ///
    /// Two trees legal in every other way — the vocabulary admits them, the wire
    /// carries them, the far side tree holds them — and the projection stops at
    /// the node it cannot present and says which node it was and why. A silent
    /// skip would have produced a frame whose sender believes it contains a
    /// canvas.
    #[test]
    fn a_role_the_remote_cannot_present_is_refused_by_name() {
        // A canvas with a track under it, because `node::check` calls a
        // childless canvas an escape: this fixture is refused for what it *is*
        // rather than for being malformed.
        let drawn = [
            Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface),
            Node::new(NodeId::new(2), NodeId::new(1), Role::Canvas),
            Node::new(NodeId::new(3), NodeId::new(2), Role::Track),
        ];
        let mut remote = Remote::opening(EPOCH);
        link(&drawn, &mut remote);
        assert_eq!(remote.tree().live(), 3, "the tree holds what it cannot present");
        assert_eq!(
            restate(remote.tree()).err(),
            Some(Refused::Unpresentable {
                node: 2,
                role: Role::Canvas,
                why: Unpresentable::Pixels
            })
        );

        let pictured = [
            Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface),
            Node::new(NodeId::new(2), NodeId::new(1), Role::Image),
        ];
        let mut second = Remote::opening(EPOCH);
        link(&pictured, &mut second);
        assert_eq!(
            restate(second.tree()).err(),
            Some(Refused::Unpresentable { node: 2, role: Role::Image, why: Unpresentable::Media })
        );

        // And the surface alone — the half of each fixture that *is*
        // presentable — presents, so what was refused above was the role and not
        // the shape of the fixture.
        let alone = [Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface)];
        let mut third = Remote::opening(EPOCH);
        link(&alone, &mut third);
        assert_eq!(restate(third.tree()).expect("a surface alone presents").len(), 1);
    }
    /// One entry, put on the link by hand.
    ///
    /// [`send`] writes a whole declaration and cannot express a *second* frame,
    /// which is what the fixture below needs: a removal, and a declaration into
    /// the slot it freed. What is hand-written is the entry; the route is the
    /// same `Delta::encode` and [`Remote::receive`] every other fixture uses.
    fn hand(remote: &mut Remote, body: Entry) -> Option<crate::tree::Applied> {
        let (entry, payload) = around(body).encode();
        remote.receive(&entry, &payload).expect("the far side accepts the fixture entry")
    }

    /// A role as the ordinal it crosses as, through the admission.
    fn ordinal(role: Role) -> f_abi::semantic::RoleOrdinal {
        Handshake::HERE
            .negotiate()
            .expect("this build agrees with itself")
            .admit_role(role.index() as u16, &Interface)
            .expect("the vocabulary names its own roles")
    }

    /// A child in an earlier slot than its parent is refused by name.
    ///
    /// **This test exists because a mutation passed.** Deleting the order check
    /// in [`restate`] left every test in this module green: slot order is
    /// declaration order for a tree that has only ever grown, and every fixture
    /// here had only ever grown. It stops being declaration order the first time
    /// a removal frees a slot in the middle — `Tree::apply` fills the first free
    /// slot, so the next declaration lands *before* nodes declared long ago.
    ///
    /// The tree below is three nodes; the second is removed, and a fourth is
    /// declared under the third. It lands in slot one, and its parent is in slot
    /// two. Without the check the restatement is built in that order and the
    /// refusal comes from `Layout::declare` as a `NoParent` two stages later —
    /// true, and pointing at the wrong layer, which is what a refusal is for.
    #[test]
    fn a_child_in_an_earlier_slot_than_its_parent_is_refused_by_name() {
        let grown = [
            Node::new(NodeId::new(1), NodeId::UNNAMED, Role::Surface),
            Node::new(NodeId::new(2), NodeId::new(1), Role::Group),
            Node::new(NodeId::new(3), NodeId::new(1), Role::Group),
        ];
        let mut remote = Remote::opening(EPOCH);
        link(&grown, &mut remote);
        assert_eq!(restate(remote.tree()).expect("a grown tree is in order").len(), 3);

        // The second frame: the middle node leaves, and a new one takes the slot
        // it left behind.
        hand(&mut remote, Entry::Remove(Remove { node: 2 }));
        hand(
            &mut remote,
            Entry::DeclareNode(DeclareNode {
                node: 4,
                parent: 3,
                before: NO_NODE,
                role: ordinal(Role::Label),
                intent: NO_INTENT,
            }),
        );
        let applied = hand(&mut remote, Entry::Commit(Commit { frame_token: FRAME + 1 }))
            .expect("the commit closed the frame");
        assert_eq!((applied.removed, applied.declared), (1, 1));
        assert_eq!(remote.tree().live(), 3);

        assert_eq!(
            restate(remote.tree()).err(),
            Some(Refused::OutOfOrder(4)),
            "a child ahead of its parent was restated as though the slots were an order"
        );
    }
    /// What cannot be sent, and what cannot be presented once it is.
    ///
    /// The refusals with no fixture of their own until now, which is the reason
    /// this test exists: a check nothing reaches is a check a later edit deletes
    /// for free. Three of them, in the order a frame meets them.
    #[test]
    fn a_frame_that_cannot_be_sent_or_solved_is_refused() {
        let sent = Cell::new(0u32);
        let mut count = |_entry: &Sqe, _payload: &[u8; PAYLOAD_BYTES]| {
            sent.set(sent.get().saturating_add(1));
        };

        // A frame nobody named. Refused before the handshake, because a name is
        // what a refusal would be reported against and there is nothing to
        // report against yet.
        assert_eq!(send(&settings_panel(), 0, &mut count), Err(Unsendable::UnnamedFrame));
        assert_eq!(sent.get(), 0, "a frame with no name put entries on the link");

        // A node that names nothing. The handshake has crossed by then and that
        // is deliberate — `send` refuses at the node rather than walking the
        // tree first, so what a peer sees is a frame that never commits.
        let unnamed = [Node::new(NodeId::UNNAMED, NodeId::UNNAMED, Role::Surface)];
        assert_eq!(send(&unnamed, FRAME, &mut count), Err(Unsendable::Unnamed));
        assert_eq!(sent.get(), 1, "the handshake is the one entry that had already crossed");

        // And a pass larger than the scope it was given. `emit` holds what a
        // refused pass leaves behind; what is asserted here is only that the
        // refusal arrives as this module names it rather than as a panic.
        let remote = received();
        let (resolved, _) = resolve(&Panel::DESK.theme(Theme::DEFAULT));
        let restated = restate(remote.tree()).expect("nothing is refused");
        let mut layout = Layout::new();
        let mut nowhere = |_delta: SceneDelta| {};
        let narrow = Scope { nodes_max: 3 };
        let refused = present(&restated, Panel::DESK, &resolved, &mut layout, narrow, &mut nowhere)
            .expect_err("fifteen nodes is past three");
        assert!(
            matches!(refused, Refused::Cascade(cascade) if cascade.scope_nodes == 3),
            "a cascade arrived as {refused:?}"
        );
    }
}
